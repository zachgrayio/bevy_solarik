//! AMD `FidelityFX` FSR Ray Regeneration (the SDK 2.4.0 denoiser) for Solari on
//! DX12 / AMD RX 9000+.
//!
//! Mirrors the NRD path: Solari's split lighting (`NRD_SIGNALS`: diffuse and
//! specular radiance with hit distance in alpha) is demodulated and packed into
//! the denoiser's inputs by `prepare_fsr_rr`, denoised as the indirect-diffuse and
//! indirect-specular signals, then remodulated into the view target by
//! `resolve_fsr_rr` (`fsr_rr.wesl`). The `FfxApi` dispatch records raw D3D12
//! commands, so it gets its own command encoder between the two wgpu passes:
//! wgpu rejects mixing raw and wgpu encoding on one encoder.
//!
//! The neural radiance cache is not integrated: it needs prediction/training
//! buffers Solari does not produce.

use bevy_asset::{AssetServer, load_embedded_asset};
use bevy_core_pipeline::prepass::PreviousViewData;
use bevy_ecs::prelude::*;
use bevy_math::{Mat4, UVec2, Vec2, Vec3};
use bevy_render::{
    render_resource::{binding_types::*, *},
    renderer::{RenderContext, RenderDevice},
    view::{ExtractedView, ViewUniform},
};
use core::sync::atomic::{AtomicBool, Ordering};
use solarik_fsr_wgpu::{
    effect::{BackendDevice, EffectConfig, EffectContext, EffectKind, FfxApiHeader},
    effects::{self, desc_types as d},
    states,
};
use std::sync::Mutex;

/// `FFX_DENOISER_SIGNAL_INDIRECT_DIFFUSE | FFX_DENOISER_SIGNAL_INDIRECT_SPECULAR`.
const SIGNALS: u32 = (1 << 4) | (1 << 5);
/// `FFX_DENOISER_DISPATCH_RESET`.
const DISPATCH_RESET: u32 = 1 << 0;
/// `FFX_DENOISER_DISPATCH_NON_ENCODED_ALBEDO`: albedo is stored linear in RGBA16F.
const DISPATCH_NON_ENCODED_ALBEDO: u32 = 1 << 1;
/// Absolute view-space depth range that is denoised; the prepare pass writes
/// `-1e6` for empty pixels so they pass through.
const LINEAR_DEPTH_BOUNDS: effects::FloatBounds = effects::FloatBounds {
    min: 0.0,
    max: 1.0e5,
};

/// Diagnostic opt-out: `SOLARIK_FSR_RR=0` leaves Solari's undenoised lighting in
/// the view target, for A/B comparison with the denoised result.
const DISABLE_ENV: &str = "SOLARIK_FSR_RR";

fn disabled_by_env() -> bool {
    static DISABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DISABLED.get_or_init(|| {
        let disabled = std::env::var(DISABLE_ENV).is_ok_and(|v| v == "0");
        if disabled {
            tracing::info!("FSR Ray Regeneration disabled by {DISABLE_ENV}=0");
        }
        disabled
    })
}

/// Process-wide FSR Ray Regeneration state for the ray-traced camera.
#[derive(Resource, Default)]
pub(super) struct FsrSolariEffects {
    library: Mutex<Option<solarik_fsr_wgpu::Library>>,
    library_probed: AtomicBool,
    pipelines: Option<RrPipelines>,
    state: Mutex<Option<RrState>>,
    warned: AtomicBool,
    active: AtomicBool,
}

struct RrPipelines {
    layout: BindGroupLayoutDescriptor,
    prepare: CachedComputePipelineId,
    resolve: CachedComputePipelineId,
}

/// Denoiser inputs/outputs at render resolution.
struct RrTargets {
    size: UVec2,
    linear_depth: TextureView,
    motion: TextureView,
    normals: TextureView,
    diffuse_albedo: TextureView,
    specular_albedo: TextureView,
    diffuse_in: TextureView,
    specular_in: TextureView,
    diffuse_out: TextureView,
    specular_out: TextureView,
}

impl RrTargets {
    fn new(device: &RenderDevice, size: UVec2) -> Self {
        let target = |label, format| {
            device
                .create_texture(&TextureDescriptor {
                    label: Some(label),
                    size: Extent3d {
                        width: size.x,
                        height: size.y,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format,
                    usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&TextureViewDescriptor::default())
        };
        let half = TextureFormat::Rgba16Float;
        Self {
            size,
            linear_depth: target("FSR RR linear depth", TextureFormat::R32Float),
            motion: target("FSR RR motion", half),
            normals: target("FSR RR normals", half),
            diffuse_albedo: target("FSR RR diffuse albedo", half),
            specular_albedo: target("FSR RR specular albedo", half),
            diffuse_in: target("FSR RR diffuse input", half),
            specular_in: target("FSR RR specular input", half),
            diffuse_out: target("FSR RR diffuse output", half),
            specular_out: target("FSR RR specular output", half),
        }
    }
}

struct RrState {
    targets: RrTargets,
    context: EffectContext,
    previous_camera: Option<Vec3>,
}

impl FsrSolariEffects {
    fn ensure_library(&self, device: &RenderDevice) -> Option<solarik_fsr_wgpu::Library> {
        if !self.library_probed.swap(true, Ordering::AcqRel) {
            let loaded = solarik_fsr_wgpu::Library::load_for_device(device.wgpu_device()).ok();
            *self
                .library
                .lock()
                .expect("FSR effects library mutex poisoned") = loaded;
        }
        self.library
            .lock()
            .expect("FSR effects library mutex poisoned")
            .clone()
    }

    fn warn_once(&self, error: &dyn core::fmt::Display, what: &str) {
        if !self.warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(%error, "FSR Ray Regeneration {what}; showing undenoised lighting");
        }
    }
}

/// Queue the prepare/resolve pipelines (DX12 only; NRD covers Vulkan).
pub(super) fn init_pipelines(
    mut effects: ResMut<FsrSolariEffects>,
    cache: Res<PipelineCache>,
    assets: Res<AssetServer>,
) {
    let float = || texture_2d(TextureSampleType::Float { filterable: false });
    let output = |format| texture_storage_2d(format, StorageTextureAccess::WriteOnly);
    let half = TextureFormat::Rgba16Float;
    let layout = BindGroupLayoutDescriptor::new(
        "FSR Ray Regeneration prepare and resolve",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                float(),
                float(),
                texture_2d(TextureSampleType::Uint),
                texture_depth_2d(),
                float(),
                uniform_buffer::<ViewUniform>(true),
                uniform_buffer::<PreviousViewData>(true),
                output(TextureFormat::R32Float),
                output(half),
                output(half),
                output(half),
                output(half),
                output(half),
                output(half),
                float(),
                float(),
                output(half),
            ),
        ),
    );
    let pipeline = |entry: &'static str| {
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(entry.into()),
            layout: vec![layout.clone()],
            shader: load_embedded_asset!(assets.as_ref(), "fsr_rr.wesl"),
            entry_point: Some(entry.into()),
            ..Default::default()
        })
    };
    effects.pipelines = Some(RrPipelines {
        prepare: pipeline("prepare_fsr_rr"),
        resolve: pipeline("resolve_fsr_rr"),
        layout,
    });
}

/// Solari inputs for one ray-traced view.
pub(super) struct RrInputs<'a> {
    pub size: UVec2,
    pub frame_index: u32,
    pub reset: bool,
    pub camera: &'a ExtractedView,
    /// Sub-pixel projection jitter in pixels (the upscaler's sign convention).
    pub jitter: Vec2,
    pub raw_diffuse: &'a TextureView,
    pub raw_specular: &'a TextureView,
    pub gbuffer: &'a TextureView,
    pub depth: &'a TextureView,
    pub motion: &'a TextureView,
    pub view_uniforms: BindingResource<'a>,
    pub previous_view_uniforms: BindingResource<'a>,
    pub view_offset: u32,
    pub previous_view_offset: u32,
    pub target: &'a WgpuTextureView,
}

/// glam is column-major with column vectors; `FfxApiMatrix4x4` is row-major with
/// row vectors, so the storage is copied as-is (no transpose).
fn ffx_matrix(m: Mat4) -> effects::Matrix4x4 {
    let rows = m.to_cols_array_2d().map(|c| effects::Float4 {
        x: c[0],
        y: c[1],
        z: c[2],
        w: c[3],
    });
    effects::Matrix4x4 { rows }
}

/// Denoise Solari's opaque lighting with FSR Ray Regeneration and write the
/// result into the view target. Returns whether the view target was resolved.
pub(super) fn denoise(
    effects_state: &FsrSolariEffects,
    enabled: bool,
    inputs: RrInputs<'_>,
    device: &RenderDevice,
    cache: &PipelineCache,
    ctx: &mut RenderContext,
) -> bool {
    if !enabled || disabled_by_env() {
        effects_state.active.store(false, Ordering::Relaxed);
        return false;
    }
    let Some(pipelines) = effects_state.pipelines.as_ref() else {
        return false;
    };
    let (Some(prepare), Some(resolve)) = (
        cache.get_compute_pipeline(pipelines.prepare),
        cache.get_compute_pipeline(pipelines.resolve),
    ) else {
        return false;
    };
    let Some(library) = effects_state.ensure_library(device) else {
        return false;
    };
    let Ok(backend) = BackendDevice::from_wgpu(device.wgpu_device()) else {
        return false;
    };
    if !matches!(backend, BackendDevice::Dx12(_)) {
        return false;
    }

    let mut guard = effects_state
        .state
        .lock()
        .expect("FSR Ray Regeneration mutex poisoned");
    if guard
        .as_ref()
        .is_none_or(|state| state.targets.size != inputs.size)
    {
        *guard = None;
        let cfg = EffectConfig {
            max_render_size: [inputs.size.x, inputs.size.y],
            display_size: [inputs.size.x, inputs.size.y],
            signal_flags: SIGNALS,
            checkerboard_signal_flags: 0,
            flags: 0,
            ..Default::default()
        };
        match EffectContext::create(&library, EffectKind::Denoiser, backend, &cfg) {
            Ok(context) => {
                tracing::info!(size = ?inputs.size, "FSR Ray Regeneration context created");
                *guard = Some(RrState {
                    targets: RrTargets::new(device, inputs.size),
                    context,
                    previous_camera: None,
                });
            }
            Err(error) => {
                effects_state.warn_once(&error, "context creation failed");
                return false;
            }
        }
    }
    let Some(state) = guard.as_mut() else {
        return false;
    };

    let t = &state.targets;
    let group = device.create_bind_group(
        "FSR Ray Regeneration prepare and resolve",
        &cache.get_bind_group_layout(&pipelines.layout),
        &BindGroupEntries::sequential((
            inputs.raw_diffuse,
            inputs.raw_specular,
            inputs.gbuffer,
            inputs.depth,
            inputs.motion,
            inputs.view_uniforms,
            inputs.previous_view_uniforms,
            &t.linear_depth,
            &t.motion,
            &t.normals,
            &t.diffuse_albedo,
            &t.specular_albedo,
            &t.diffuse_in,
            &t.specular_in,
            &t.diffuse_out,
            &t.specular_out,
            inputs.target,
        )),
    );
    let offsets = [inputs.view_offset, inputs.previous_view_offset];
    let groups = (inputs.size.x.div_ceil(8), inputs.size.y.div_ceil(8));
    {
        let mut pass = ctx
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor {
                label: Some("FSR Ray Regeneration prepare"),
                ..Default::default()
            });
        pass.set_pipeline(prepare);
        pass.set_bind_group(0, &group, &offsets);
        pass.dispatch_workgroups(groups.0, groups.1, 1);
    }

    let read = states::PIXEL_COMPUTE_READ;
    let write = states::UNORDERED_ACCESS;
    let resource =
        |view: &TextureView, state| solarik_fsr_wgpu::effect::resource_for_wgpu(view, state);
    let resources = (|| {
        Ok::<_, solarik_fsr_wgpu::Error>((
            resource(&t.linear_depth, read)?,
            resource(&t.motion, read)?,
            resource(&t.normals, read)?,
            resource(&t.specular_albedo, read)?,
            resource(&t.diffuse_albedo, read)?,
            resource(&t.diffuse_in, read)?,
            resource(&t.diffuse_out, write)?,
            resource(&t.specular_in, read)?,
            resource(&t.specular_out, write)?,
        ))
    })();
    let (
        linear_depth,
        motion_vectors,
        normals,
        specular_albedo,
        diffuse_albedo,
        diffuse_in,
        diffuse_out,
        specular_in,
        specular_out,
    ) = match resources {
        Ok(resources) => resources,
        Err(error) => {
            effects_state.warn_once(&error, "inputs unavailable");
            return false;
        }
    };
    solarik_fsr_wgpu::effect::transition_for_dispatch(
        ctx.command_encoder(),
        &[
            &t.linear_depth,
            &t.motion,
            &t.normals,
            &t.diffuse_albedo,
            &t.specular_albedo,
            &t.diffuse_in,
            &t.specular_in,
        ],
        &[&t.diffuse_out, &t.specular_out],
    );

    let camera_position = inputs.camera.world_from_view.translation();
    let restart = inputs.reset || !effects_state.active.load(Ordering::Relaxed);
    let camera_position_delta = match state.previous_camera {
        Some(previous) if !restart => previous - camera_position,
        _ => Vec3::ZERO,
    };
    let signal = |input, output| effects::DenoiserSignal {
        input,
        output,
        checkerboard_origin: 0,
    };
    let mut specular_signal = effects::DispatchDescDenoiserIndirectSpecular {
        header: FfxApiHeader {
            kind: d::DENOISER_DISPATCH_INDIRECT_SPECULAR,
            next: core::ptr::null_mut(),
        },
        signal: signal(specular_in, specular_out),
    };
    let mut diffuse_signal = effects::DispatchDescDenoiserIndirectDiffuse {
        header: FfxApiHeader {
            kind: d::DENOISER_DISPATCH_INDIRECT_DIFFUSE,
            next: &raw mut specular_signal.header,
        },
        signal: signal(diffuse_in, diffuse_out),
    };
    let mut desc = effects::DispatchDescDenoiser {
        header: FfxApiHeader {
            kind: d::DENOISER_DISPATCH,
            next: &raw mut diffuse_signal.header,
        },
        command_list: core::ptr::null_mut(),
        linear_depth,
        motion_vectors,
        normals,
        specular_albedo,
        diffuse_albedo,
        // Bevy stores `CurrentUV - PreviousUV`; the denoiser wants the reverse.
        motion_vector_scale: effects::FloatCoords3D {
            x: -1.0,
            y: -1.0,
            z: 1.0,
        },
        jitter_offsets: effects::FloatCoords2D {
            x: inputs.jitter.x,
            y: inputs.jitter.y,
        },
        camera_position_delta: effects::FloatCoords3D {
            x: camera_position_delta.x,
            y: camera_position_delta.y,
            z: camera_position_delta.z,
        },
        view: ffx_matrix(inputs.camera.world_from_view.to_matrix().inverse()),
        projection: ffx_matrix(inputs.camera.clip_from_view),
        linear_depth_bounds: LINEAR_DEPTH_BOUNDS,
        render_size: effects::Dimensions2D {
            width: inputs.size.x,
            height: inputs.size.y,
        },
        frame_index: inputs.frame_index,
        flags: DISPATCH_NON_ENCODED_ALBEDO | if restart { DISPATCH_RESET } else { 0 },
    };
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("fsr_ray_regeneration"),
    });
    if let Err(error) = state.context.dispatch_wgpu(&mut encoder, &mut desc) {
        effects_state.warn_once(&error, "dispatch failed");
        effects_state.active.store(false, Ordering::Relaxed);
        return false;
    }
    ctx.add_command_buffer(encoder.finish());

    {
        let mut pass = ctx
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor {
                label: Some("FSR Ray Regeneration resolve"),
                ..Default::default()
            });
        pass.set_pipeline(resolve);
        pass.set_bind_group(0, &group, &offsets);
        pass.dispatch_workgroups(groups.0, groups.1, 1);
    }
    state.previous_camera = Some(camera_position);
    if !effects_state.active.swap(true, Ordering::Relaxed) {
        tracing::info!(size = ?inputs.size, "FSR Ray Regeneration denoising active");
    }
    true
}
