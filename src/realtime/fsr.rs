//! AMD `FidelityFX` FSR upscaling attached to the Solarik realtime compositor.
//!
//! Mirrors [`super::nrd`]: one native context per view, Bevy owns scheduling, and
//! all unsafe ABI/code lives in `solarik-fsr-wgpu`. The provider DLL is loaded
//! lazily and is optional — when it is absent the view keeps native resolution,
//! just like an unavailable DLSS preference in a consuming application.
//!
//! The default path is FSR `NativeAA` (1:1 temporal anti-aliasing). Scaled presets
//! require the application to set the camera's render resolution and
//! `TemporalJitter`/`MipBias`, the same way it drives `bevy_anti_alias::dlss`;
//! see `docs/fsr.md`.

use super::FsrQualityMode;
use bevy_asset::{AssetServer, load_embedded_asset};
use bevy_ecs::prelude::*;
use bevy_math::UVec2;
use bevy_render::{
    render_resource::{binding_types::*, *},
    renderer::RenderDevice,
};
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Per-view FSR state: the upscaled output texture and a persistent native context.
pub(super) struct ViewFsr {
    output: TextureView,
    size: UVec2,
    context: Mutex<Option<solarik_fsr_wgpu::Context>>,
    warned: AtomicBool,
}

impl ViewFsr {
    pub fn new(device: &RenderDevice, size: UVec2) -> Self {
        let output = device
            .create_texture(&TextureDescriptor {
                label: Some("FSR upscaled color"),
                size: Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba16Float,
                usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&TextureViewDescriptor::default());
        Self {
            output,
            size,
            context: Mutex::new(None),
            warned: AtomicBool::new(false),
        }
    }
}

/// Process-wide provider handle. `None` means FSR is unavailable and every view
/// keeps native resolution; no DLL is required to build or to run.
#[derive(Resource)]
pub(super) struct FsrState {
    pub library: Option<solarik_fsr_wgpu::Library>,
}

/// Compute pipeline that composes FSR's output into the camera HDR target.
#[derive(Resource)]
pub(super) struct FsrPipelines {
    layout: BindGroupLayoutDescriptor,
    compose: CachedComputePipelineId,
}

/// Load the optional provider and queue the compose pipeline once, at render startup.
pub(super) fn init_fsr(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    assets: Res<AssetServer>,
    device: Res<RenderDevice>,
) {
    let library = match solarik_fsr_wgpu::Library::load_for_device(device.wgpu_device()) {
        Ok(library) => {
            tracing::info!("AMD FidelityFX FSR provider available");
            Some(library)
        }
        Err(error) => {
            tracing::info!(%error, "AMD FidelityFX FSR provider unavailable; native resolution retained");
            None
        }
    };
    let layout = BindGroupLayoutDescriptor::new(
        "FSR compose",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_storage_2d(TextureFormat::Rgba16Float, StorageTextureAccess::WriteOnly),
            ),
        ),
    );
    let compose = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("FSR compose".into()),
        layout: vec![layout.clone()],
        shader: load_embedded_asset!(assets.as_ref(), "fsr.wesl"),
        entry_point: Some("compose_fsr".into()),
        ..Default::default()
    });
    commands.insert_resource(FsrState { library });
    commands.insert_resource(FsrPipelines { layout, compose });
}

fn warn_once(view: &ViewFsr, message: &str) {
    if !view.warned.swap(true, Ordering::Relaxed) {
        tracing::warn!("{message}");
    }
}

/// Dispatch FSR after lighting/NRD resolve and compose its result into the view.
#[expect(
    clippy::too_many_arguments,
    reason = "All bindings belong to one camera's opaque FSR dispatch"
)]
pub(super) fn upscale(
    view: &ViewFsr,
    enabled: bool,
    quality: FsrQualityMode,
    sharpness: f32,
    reset: bool,
    frame_index: u32,
    state: &FsrState,
    device: &RenderDevice,
    encoder: &mut CommandEncoder,
    cache: &PipelineCache,
    pipelines: &FsrPipelines,
    color: &WgpuTextureView,
    depth: &TextureView,
    motion: &TextureView,
    target: &WgpuTextureView,
) {
    if !enabled {
        return;
    }
    let Some(library) = &state.library else {
        return;
    };
    let Some(compose) = cache.get_compute_pipeline(pipelines.compose) else {
        return;
    };
    let render_size = [view.size.x, view.size.y];
    // NativeAA by default. Scaled presets need the application to lower the
    // camera render resolution and size ViewFsr::output to display resolution.
    let upscale_size = render_size;
    let native_quality = quality.native();

    let mut guard = view.context.lock().expect("FSR view mutex poisoned");
    if guard.is_none() {
        match solarik_fsr_wgpu::Context::new(
            library,
            device.wgpu_device(),
            render_size,
            upscale_size,
            solarik_fsr_wgpu::flags::HIGH_DYNAMIC_RANGE,
        ) {
            Ok(created) => *guard = Some(created),
            Err(error) => {
                warn_once(view, &format!("FSR context creation failed: {error}"));
                return;
            }
        }
    }
    let context = guard.as_mut().expect("FSR context initialized above");

    let phase_count = native_quality.jitter_phase_count(upscale_size[0]);
    let jitter = native_quality.suggested_jitter(frame_index, phase_count);
    let dispatch = solarik_fsr_wgpu::Dispatch {
        color,
        depth,
        motion_vectors: motion,
        output: &view.output,
        exposure: None,
        reactive: None,
        transparency_and_composition: None,
        jitter,
        motion_vector_scale: [render_size[0] as f32, render_size[1] as f32],
        render_size,
        upscale_size,
        sharpness,
        frame_time_delta_ms: 16.6,
        pre_exposure: 1.0,
        reset,
        camera_near: 0.1,
        camera_far: 1000.0,
        camera_fov_angle_vertical: 1.0,
        view_space_to_meters_factor: 1.0,
        flags: 0,
        encoder,
    };
    if let Err(error) = context.dispatch(dispatch) {
        warn_once(view, &format!("FSR dispatch failed: {error}"));
        return;
    }

    let group = device.create_bind_group(
        "FSR compose",
        &cache.get_bind_group_layout(&pipelines.layout),
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&view.output),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(target),
            },
        ],
    );
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("FSR compose"),
        ..Default::default()
    });
    pass.set_pipeline(compose);
    pass.set_bind_group(0, &group, &[]);
    pass.dispatch_workgroups(view.size.x.div_ceil(8), view.size.y.div_ceil(8), 1);
}
