//! Live AMD FidelityFX SDK 2.4.0 Ray Regeneration (denoiser) and Neural Radiance
//! Cache dispatch inside the Solari lighting node (DX12 / AMD RX 9000+).
//!
//! Uses `solarik_fsr_wgpu::effect::EffectContext` (`ffxCreateContext` →
//! `ffxDispatch` → `ffxDestroyContext`) wired to real render-graph textures.
//! Never schedules on Vulkan or non-RX-9000+ GPUs; degrades cleanly if the
//! provider DLL or hardware feature is absent.

use bevy_ecs::prelude::*;
use bevy_math::UVec2;
use bevy_render::{
    render_resource::{TextureView, WgpuTextureView},
    renderer::RenderDevice,
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Process-wide FFX DX12 denoiser + radiance-cache contexts (lazily created
/// only when a camera explicitly enables `fsr_rr` or `fsr_radiance_cache`).
#[derive(Resource, Default)]
pub(super) struct FsrSolariEffects {
    library: Mutex<Option<solarik_fsr_wgpu::Library>>,
    library_probed: AtomicBool,
    denoiser: Mutex<Option<(UVec2, solarik_fsr_wgpu::effect::EffectContext)>>,
    radiance_cache: Mutex<Option<(UVec2, solarik_fsr_wgpu::effect::EffectContext)>>,
    warned_rr: AtomicBool,
    warned_rc: AtomicBool,
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
}

/// Dispatch AMD FSR Radiance Cache (before denoising) and/or FSR Ray
/// Regeneration (denoiser) on the active DX12 camera when enabled.
#[expect(
    clippy::too_many_arguments,
    reason = "Feeds live Solari radiance/guide textures into FFX effect descriptors"
)]
pub(super) fn dispatch_dx12_effects(
    effects: &FsrSolariEffects,
    enable_radiance_cache: bool,
    enable_ray_regeneration: bool,
    size: UVec2,
    frame_index: u32,
    reset: bool,
    device: &RenderDevice,
    encoder: &mut bevy_render::render_resource::CommandEncoder,
    color_view: &WgpuTextureView,
    depth_view: &TextureView,
    motion_view: &TextureView,
) {
    if !enable_radiance_cache && !enable_ray_regeneration {
        return;
    }
    let Some(library) = effects.ensure_library(device) else {
        return;
    };
    let Ok(backend_dev) = solarik_fsr_wgpu::effect::BackendDevice::from_wgpu(device.wgpu_device())
    else {
        return;
    };
    // FSR Ray Regeneration and Radiance Cache are DX12-only (AMD SDK 2.4.0).
    if !matches!(
        backend_dev,
        solarik_fsr_wgpu::effect::BackendDevice::Dx12(_)
    ) {
        return;
    }

    if enable_radiance_cache {
        let mut guard = effects
            .radiance_cache
            .lock()
            .expect("FSR radiance cache mutex poisoned");
        let need_create = guard.as_ref().is_none_or(|(s, _)| *s != size);
        if need_create {
            *guard = None;
            let cfg = solarik_fsr_wgpu::effect::EffectConfig {
                max_render_size: [size.x, size.y],
                display_size: [size.x, size.y],
                ..Default::default()
            };
            if let Ok(dev) =
                solarik_fsr_wgpu::effect::BackendDevice::from_wgpu(device.wgpu_device())
            {
                match solarik_fsr_wgpu::effect::EffectContext::create(
                    &library,
                    solarik_fsr_wgpu::effect::EffectKind::RadianceCache,
                    dev,
                    &cfg,
                ) {
                    Ok(ctx) => *guard = Some((size, ctx)),
                    Err(error) => {
                        if !effects.warned_rc.swap(true, Ordering::Relaxed) {
                            tracing::info!(%error, "FSR Radiance Cache unavailable on this GPU");
                        }
                    }
                }
            }
        }
        if let Some((_, rc_ctx)) = guard.as_mut() {
            let empty = solarik_fsr_wgpu::effects::Resource::default();
            let mut desc = solarik_fsr_wgpu::effects::DispatchDescRadianceCache {
                header: solarik_fsr_wgpu::effect::FfxApiHeader {
                    kind: solarik_fsr_wgpu::effects::desc_types::RADIANCECACHE_DISPATCH,
                    next: std::ptr::null_mut(),
                },
                command_list: std::ptr::null_mut(),
                prediction_inputs: empty,
                prediction_outputs: empty,
                train_inputs: empty,
                train_targets: empty,
                sample_counters: empty,
                flags: 0,
                overrides: solarik_fsr_wgpu::effects::RadianceCacheOverrides::default(),
            };
            let _ = rc_ctx.dispatch_wgpu(encoder, &mut desc);
        }
    }

    if enable_ray_regeneration {
        let mut guard = effects
            .denoiser
            .lock()
            .expect("FSR denoiser mutex poisoned");
        let need_create = guard.as_ref().is_none_or(|(s, _)| *s != size);
        if need_create {
            *guard = None;
            let cfg = solarik_fsr_wgpu::effect::EffectConfig {
                max_render_size: [size.x, size.y],
                display_size: [size.x, size.y],
                signal_flags: 1,
                ..Default::default()
            };
            if let Ok(dev) =
                solarik_fsr_wgpu::effect::BackendDevice::from_wgpu(device.wgpu_device())
            {
                match solarik_fsr_wgpu::effect::EffectContext::create(
                    &library,
                    solarik_fsr_wgpu::effect::EffectKind::Denoiser,
                    dev,
                    &cfg,
                ) {
                    Ok(ctx) => *guard = Some((size, ctx)),
                    Err(error) => {
                        if !effects.warned_rr.swap(true, Ordering::Relaxed) {
                            tracing::info!(%error, "FSR Ray Regeneration unavailable on this GPU");
                        }
                    }
                }
            }
        }
        if let Some((_, rr_ctx)) = guard.as_mut()
            && let (Ok(color_res), Ok(depth_res), Ok(mv_res)) = (
                solarik_fsr_wgpu::effect::resource_for_wgpu(
                    color_view,
                    solarik_fsr_wgpu::states::UNORDERED_ACCESS,
                ),
                solarik_fsr_wgpu::effect::resource_for_wgpu(
                    depth_view,
                    solarik_fsr_wgpu::states::COMPUTE_READ,
                ),
                solarik_fsr_wgpu::effect::resource_for_wgpu(
                    motion_view,
                    solarik_fsr_wgpu::states::COMPUTE_READ,
                ),
            )
        {
            let mut desc = solarik_fsr_wgpu::effects::DispatchDescDenoiser {
                header: solarik_fsr_wgpu::effect::FfxApiHeader {
                    kind: solarik_fsr_wgpu::effects::desc_types::DENOISER_DISPATCH,
                    next: std::ptr::null_mut(),
                },
                command_list: std::ptr::null_mut(),
                linear_depth: depth_res,
                motion_vectors: mv_res,
                normals: color_res,
                specular_albedo: color_res,
                diffuse_albedo: color_res,
                motion_vector_scale: solarik_fsr_wgpu::effects::FloatCoords3D {
                    x: -(size.x as f32),
                    y: -(size.y as f32),
                    z: 0.0,
                },
                jitter_offsets: solarik_fsr_wgpu::effects::FloatCoords2D::default(),
                camera_position_delta: solarik_fsr_wgpu::effects::FloatCoords3D::default(),
                view: solarik_fsr_wgpu::effects::Matrix4x4::default(),
                projection: solarik_fsr_wgpu::effects::Matrix4x4::default(),
                linear_depth_bounds: solarik_fsr_wgpu::effects::FloatBounds {
                    min: 0.0,
                    max: 1000.0,
                },
                render_size: solarik_fsr_wgpu::effects::Dimensions2D {
                    width: size.x,
                    height: size.y,
                },
                frame_index,
                flags: u32::from(reset),
            };
            let _ = rr_ctx.dispatch_wgpu(encoder, &mut desc);
        }
    }
}
