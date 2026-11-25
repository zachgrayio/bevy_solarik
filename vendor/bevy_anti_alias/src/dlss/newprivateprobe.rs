//! Opt-in, one-batch ordinary-wgpu HDR and diffuse-guide measurements around native RR.
use bevy_ecs::{
    component::Component,
    resource::Resource,
    system::{Res, ResMut},
};
use bevy_math::UVec2;
use bevy_render::{
    render_resource::{
        Buffer, BufferDescriptor, BufferUsages, CommandEncoder, Extent3d, MapMode, Origin3d,
        PollType, TexelCopyBufferInfo, TexelCopyBufferLayout, Texture, TextureDimension,
        TextureFormat, TextureUsages,
    },
    renderer::RenderDevice,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
};

const TILE: u32 = 16;
const ROW_BYTES: u32 = 256;
const SETTLED_EVALUATIONS: u32 = 64;

/// Entity-scoped counters: no retained map of obsolete view entities or buffers.
#[derive(Component, Default)]
pub struct ProbeView {
    successful: AtomicU32,
    sampled: AtomicBool,
}
impl ProbeView {
    pub(super) fn reset_settling(&self) {
        self.successful.store(0, Ordering::Relaxed);
    }

    pub(super) fn evaluated(&self) {
        // Saturation avoids wrapping after a very long diagnostic session.
        let _ = self
            .successful
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_add(1))
            });
    }
}

#[derive(Resource)]
pub struct RrProbe {
    pub(super) enabled: bool,
    pending: Option<ProbePair>,
    // While callbacks own the buffers, prevent another six-tile batch being allocated.
    mapping: Arc<AtomicUsize>,
}
impl Default for RrProbe {
    fn default() -> Self {
        Self {
            enabled: std::env::var("SOLARIK_DLSS_RR_PROBE").is_ok_and(|v| v == "1"),
            pending: None,
            mapping: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Tile {
    active: UVec2,
    physical: UVec2,
    origin: UVec2,
    size: UVec2,
}
impl Tile {
    fn checked(
        texture: &Texture,
        active: UVec2,
        format: TextureFormat,
        position: Position,
    ) -> Option<Self> {
        if texture.format() != format
            || !texture.usage().contains(TextureUsages::COPY_SRC)
            || texture.sample_count() != 1
            || texture.dimension() != TextureDimension::D2
            || texture.depth_or_array_layers() != 1
            || active.min_element() == 0
            || active.x > texture.width()
            || active.y > texture.height()
        {
            tracing::info!(target: "solarik_rr_probe", ?active,
                format = ?texture.format(), usage = ?texture.usage(),
                samples = texture.sample_count(), dimension = ?texture.dimension(),
                physical = ?texture.size(), "skip unsupported RR probe texture");
            return None;
        }
        let size = active.min(UVec2::splat(TILE));
        Some(Self {
            active,
            physical: UVec2::new(texture.width(), texture.height()),
            // Streamline tags the active render/output extent at texture origin zero.
            // Do not use the physical texture centre for the reduced-resolution input.
            origin: position.origin(active, size),
            size,
        })
    }
}

#[derive(Clone, Copy, Debug)]
enum Position {
    Centre,
    UpperRightQuarter,
}
impl Position {
    fn origin(self, active: UVec2, size: UVec2) -> UVec2 {
        // Normalized screen coordinates: upper-right quarter targets the wall,
        // away from the central Bard sprite. Clamp the entire tile to the active extent.
        let centre = match self {
            Self::Centre => active / 2,
            Self::UpperRightQuarter => UVec2::new(active.x - active.x / 4, active.y / 4),
        };
        centre.saturating_sub(size / 2).min(active - size)
    }
}
struct Sample {
    label: &'static str,
    buffer: Buffer,
    tile: Tile,
    position: Position,
    format: TextureFormat,
}
pub(super) struct ProbePair {
    samples: Vec<Sample>,
    frame: u32,
    view: String,
    exposure: Option<f32>,
    pub(super) evaluation_succeeded: bool,
}
impl RrProbe {
    pub(super) fn begin(
        &mut self,
        state: Option<&ProbeView>,
        device: &RenderDevice,
        textures: (&Texture, &Texture, &Texture),
        dimensions: (UVec2, UVec2),
        metadata: (u32, impl std::fmt::Debug, Option<f32>),
        encoder: &mut CommandEncoder,
    ) -> Option<ProbePair> {
        let state = state?;
        if !self.enabled
            || self.pending.is_some()
            || self.mapping.load(Ordering::Acquire) != 0
            || state.successful.load(Ordering::Relaxed) < SETTLED_EVALUATIONS
            || state.sampled.swap(true, Ordering::Relaxed)
        {
            return None;
        }
        let (source, destination, diffuse) = textures;
        let (render, output) = dimensions;
        let (frame, view, exposure) = metadata;
        let mut samples = Vec::with_capacity(6);
        for position in [Position::Centre, Position::UpperRightQuarter] {
            for (label, texture, active, format) in [
                (
                    "before_native_rr",
                    source,
                    render,
                    TextureFormat::Rgba16Float,
                ),
                (
                    "after_native_rr",
                    destination,
                    output,
                    TextureFormat::Rgba16Float,
                ),
                (
                    "diffuse_albedo_guide",
                    diffuse,
                    render,
                    TextureFormat::Rgba8Unorm,
                ),
            ] {
                let tile = Tile::checked(texture, active, format, position)?;
                let buffer = device.create_buffer(&BufferDescriptor {
                    label: Some(label),
                    size: u64::from(ROW_BYTES * tile.size.y),
                    usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                samples.push(Sample {
                    label,
                    buffer,
                    tile,
                    position,
                    format,
                });
            }
        }
        // Validate and allocate the complete bounded batch before recording any copies.
        for sample in &samples {
            let texture = match sample.label {
                "before_native_rr" => source,
                "diffuse_albedo_guide" => diffuse,
                _ => continue,
            };
            copy(encoder, texture, &sample.buffer, sample.tile);
        }
        Some(ProbePair {
            samples,
            frame,
            view: format!("{view:?}"),
            exposure,
            evaluation_succeeded: false,
        })
    }

    pub(super) fn finish(
        &mut self,
        pair: ProbePair,
        destination: &Texture,
        encoder: &mut CommandEncoder,
    ) {
        for sample in &pair.samples {
            if sample.label == "after_native_rr" {
                copy(encoder, destination, &sample.buffer, sample.tile);
            }
        }
        self.pending = Some(pair);
    }
}
fn copy(encoder: &mut CommandEncoder, texture: &Texture, buffer: &Buffer, tile: Tile) {
    let mut source = texture.as_image_copy();
    source.origin = Origin3d {
        x: tile.origin.x,
        y: tile.origin.y,
        z: 0,
    };
    encoder.copy_texture_to_buffer(
        source,
        TexelCopyBufferInfo {
            buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(tile.size.y),
            },
        },
        Extent3d {
            width: tile.size.x,
            height: tile.size.y,
            depth_or_array_layers: 1,
        },
    );
}

/// Cleanup follows RenderSystems::Render: all tracked tile copies have been submitted.
/// No map request is made while the frame's command buffers are still being recorded.
pub(super) fn readback(mut probe: ResMut<RrProbe>, device: Res<RenderDevice>) {
    if !probe.enabled {
        return;
    }
    let _ = device.poll(PollType::Poll);
    let Some(pair) = probe.pending.take() else {
        return;
    };
    probe.mapping.store(pair.samples.len(), Ordering::Release);
    for Sample {
        label,
        buffer,
        tile,
        position,
        format,
    } in pair.samples
    {
        let mapped = buffer.clone();
        let mapping = probe.mapping.clone();
        let view = pair.view.clone();
        let frame = pair.frame;
        let exposure = pair.exposure;
        let evaluation_succeeded = pair.evaluation_succeeded;
        buffer.slice(..).map_async(MapMode::Read, move |result| {
            if let Err(error) = result {
                tracing::info!(target: "solarik_rr_probe", label, frame, %view, %error,
                    "RR boundary probe map failed");
            } else {
                match mapped.slice(..).get_mapped_range() {
                    Ok(bytes) => {
                        let stats = summarize_format(&bytes, tile.size, format);
                        tracing::info!(target: "solarik_rr_probe", label, ?position, ?format, frame, %view,
                            ?exposure, evaluation_succeeded, active_dimensions = ?tile.active,
                            physical_dimensions = ?tile.physical, tile_origin = ?tile.origin,
                            tile_dimensions = ?tile.size, mean_rgb = ?stats.mean,
                            max_rgb = ?stats.max, centre_rgb = ?stats.centre,
                            finite_rgb_counts = ?stats.finite, nonfinite_rgb = stats.nonfinite,
                            nonzero_pixels = stats.nonzero,
                            "RR boundary probe linear RGB");
                        drop(bytes);
                    }
                    Err(error) => tracing::info!(target: "solarik_rr_probe", label,
                        frame, %view, %error, "RR boundary probe mapped range unavailable"),
                }
                mapped.unmap();
            }
            // Callback owns the final buffer reference; release it here, not in a Local cache.
            drop(mapped);
            mapping.fetch_sub(1, Ordering::AcqRel);
        });
    }
}

#[derive(Debug, PartialEq)]
struct Stats {
    mean: [Option<f64>; 3],
    max: [Option<f32>; 3],
    centre: [Option<f32>; 3],
    finite: [u32; 3],
    nonfinite: u32,
    nonzero: u32,
}
#[cfg(test)]
fn summarize(bytes: &[u8], size: UVec2) -> Stats {
    summarize_format(bytes, size, TextureFormat::Rgba16Float)
}
fn summarize_format(bytes: &[u8], size: UVec2, format: TextureFormat) -> Stats {
    let mut sum = [0.0_f64; 3];
    let mut stats = Stats {
        mean: [None; 3],
        max: [None; 3],
        centre: [None; 3],
        finite: [0; 3],
        nonfinite: 0,
        nonzero: 0,
    };
    for y in 0..size.y {
        for x in 0..size.x {
            let mut nonzero = false;
            for channel in 0..3 {
                let value = if format == TextureFormat::Rgba8Unorm {
                    let offset = (y * ROW_BYTES + x * 4) as usize + channel;
                    let Some(raw) = bytes.get(offset) else {
                        continue;
                    };
                    f32::from(*raw) / 255.0
                } else {
                    let offset = (y * ROW_BYTES + x * 8) as usize + channel * 2;
                    let Some(raw) = bytes.get(offset..offset + 2) else {
                        continue;
                    };
                    decode_f16(u16::from_le_bytes([raw[0], raw[1]]))
                };
                if !value.is_finite() {
                    stats.nonfinite += 1;
                    continue;
                }
                nonzero |= value != 0.0;
                sum[channel] += f64::from(value);
                stats.finite[channel] += 1;
                stats.max[channel] = Some(stats.max[channel].map_or(value, |v| v.max(value)));
                if x == size.x / 2 && y == size.y / 2 {
                    stats.centre[channel] = Some(value);
                }
            }
            stats.nonzero += u32::from(nonzero);
        }
    }
    for channel in 0..3 {
        if stats.finite[channel] != 0 {
            stats.mean[channel] = Some(sum[channel] / f64::from(stats.finite[channel]));
        }
    }
    stats
}

// No half-float dependency in bevy_anti_alias. Exact IEEE-754 binary16 expansion.
fn decode_f16(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = (bits >> 10) & 31;
    let fraction = u32::from(bits & 1023);
    if exponent == 0 {
        // All binary16 subnormals are exactly representable in f32 (including signed zero).
        let magnitude = fraction as f32 * (1.0 / 16_777_216.0);
        return if sign == 0 { magnitude } else { -magnitude };
    }
    let exponent = if exponent == 31 {
        255
    } else {
        u32::from(exponent) + 112
    };
    f32::from_bits(sign | (exponent << 23) | (fraction << 13))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rgba8_summary_decodes_unorm_and_ignores_alpha_and_padding() {
        let mut bytes = vec![0xff; (ROW_BYTES * 2) as usize];
        bytes[..8].copy_from_slice(&[0, 0, 0, 255, 255, 128, 0, 0]);
        let second = ROW_BYTES as usize;
        bytes[second..second + 8].copy_from_slice(&[0, 0, 255, 0, 0, 0, 0, 255]);
        let stats = summarize_format(&bytes, UVec2::splat(2), TextureFormat::Rgba8Unorm);
        assert_eq!(stats.finite, [4; 3]);
        assert_eq!(stats.nonfinite, 0);
        assert_eq!(stats.nonzero, 2);
        assert_eq!(
            stats.mean,
            [
                Some(0.25),
                Some(f64::from(128.0_f32 / 255.0) / 4.0),
                Some(0.25)
            ]
        );
        assert_eq!(stats.max, [Some(1.0), Some(128.0 / 255.0), Some(1.0)]);
        assert_eq!(stats.centre, [Some(0.0); 3]);
    }

    #[test]
    fn positions_align_across_active_input_and_output_extents() {
        let size = UVec2::splat(TILE);
        let input = UVec2::new(800, 600);
        let output = input * 2;
        for position in [Position::Centre, Position::UpperRightQuarter] {
            let input_centre = position.origin(input, size) + size / 2;
            let output_centre = position.origin(output, size) + size / 2;
            assert_eq!(input_centre * 2, output_centre);
        }
        assert_eq!(Position::Centre.origin(input, size), UVec2::new(392, 292));
        assert_eq!(
            Position::UpperRightQuarter.origin(input, size),
            UVec2::new(592, 142)
        );
        let tiny = UVec2::new(3, 5);
        assert_eq!(Position::UpperRightQuarter.origin(tiny, tiny), UVec2::ZERO);
        let odd = UVec2::new(101, 75);
        let origin = Position::UpperRightQuarter.origin(odd, size);
        assert!((origin + size).cmple(odd).all());
    }

    #[test]
    fn binary16_known_values() {
        for (bits, expected) in [
            (0, 0.0),
            (0x3c00, 1.0),
            (0xc000, -2.0),
            (0x7bff, 65504.0),
            (1, 1.0 / 16_777_216.0),
            (0x0400, 1.0 / 16384.0),
        ] {
            assert_eq!(decode_f16(bits), expected);
        }
        assert_eq!(decode_f16(0x8000).to_bits(), (-0.0_f32).to_bits());
        assert_eq!(decode_f16(0x7c00), f32::INFINITY);
        assert_eq!(decode_f16(0xfc00), f32::NEG_INFINITY);
        assert!(decode_f16(0x7e00).is_nan());
    }
    #[test]
    fn summary_ignores_padding_alpha_and_nonfinite_rgb() {
        let mut bytes = vec![0xff; (ROW_BYTES * 2) as usize];
        for (pixel, rgb) in [
            [0x3c00_u16, 0x4000, 0x4200],
            [0x4200, 0x4400, 0x4500],
            [0x4500, 0x4600, 0x4700],
            [0x7e00, 0x7c00, 0xc000],
        ]
        .into_iter()
        .enumerate()
        {
            let base = pixel / 2 * ROW_BYTES as usize + pixel % 2 * 8;
            for (channel, value) in rgb.into_iter().enumerate() {
                bytes[base + channel * 2..base + channel * 2 + 2]
                    .copy_from_slice(&value.to_le_bytes());
            }
        }
        let stats = summarize(&bytes, UVec2::splat(2));
        assert_eq!(stats.finite, [3, 3, 4]);
        assert_eq!(stats.nonfinite, 2);
        assert_eq!(stats.mean, [Some(3.0), Some(4.0), Some(3.25)]);
        assert_eq!(stats.max, [Some(5.0), Some(6.0), Some(7.0)]);
        assert_eq!(stats.centre, [None, None, Some(-2.0)]);
    }
}
