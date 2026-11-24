//! Runtime Frame Generation plugin for both NVIDIA Streamline (DLSS-G 2x / DLSS
//! 4.5 MFG 3x/4x + Reflex) and AMD FidelityFX FSR Frame Generation across DX12
//! and Vulkan.
//!
//! Zero vendor build gating: both providers are dynamically loaded at runtime;
//! whichever hardware/driver/provider is available populates
//! [`FrameGenerationRuntimeCapabilities`] and drives live frames when requested
//! via [`FrameGenerationRuntimeSettings`].

use bevy_app::{App, Plugin};
use bevy_camera::MainPassResolutionOverride;
use bevy_core_pipeline::{
    prepass::ViewPrepassTextures,
    schedule::{Core3d, Core3dSystems},
};
use bevy_diagnostic::FrameCount;
use bevy_ecs::prelude::*;
use bevy_math::{Mat4, UVec2, Vec4Swizzles};
use bevy_render::{
    RenderApp,
    camera::TemporalJitter,
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    render_resource::CommandEncoderDescriptor,
    renderer::{RenderContext, RenderDevice, ViewQuery},
    view::{ExtractedView, ViewTarget},
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Real runtime capabilities queried at startup from NVIDIA Streamline (`slInit`
/// + `slIsFeatureSupported(kFeatureDLSS_G)` + `slDLSSGGetState`) and AMD FFX.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameGenerationRuntimeCapabilities {
    /// NVIDIA Streamline initialized and `kFeatureDLSS_G` is supported (RTX 40-series+).
    pub dlss_g_supported: bool,
    /// NVIDIA DLSS 4.5 Multi-Frame Generation (3x/4x+) supported (RTX 50-series+).
    pub dlss_mfg_supported: bool,
    /// Hardware max interpolated frames per real frame (`1` = 2x, `>= 2` = MFG).
    pub max_generated_frames: u32,
    /// NVIDIA Reflex low-latency pacing available.
    pub reflex_supported: bool,
    /// AMD FidelityFX FSR Frame Generation provider loaded for the active backend.
    pub fsr_fg_supported: bool,
}

/// Live per-frame Frame Generation settings written by the application (e.g.
/// from BARD's `ActiveCombo.frame_generation`) and extracted into `RenderApp`.
#[derive(Resource, ExtractResource, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[extract_app(RenderApp)]
pub struct FrameGenerationRuntimeSettings {
    /// Number of interpolated frames per real frame: `0` = Off, `1` = 2x, `2` = 3x, `3` = 4x.
    pub num_frames_to_generate: u32,
    /// Use NVIDIA Streamline DLSS-G / MFG when enabled.
    pub use_dlss_g: bool,
    /// Use AMD FidelityFX FSR Frame Generation when enabled.
    pub use_fsr_fg: bool,
    /// Reset temporal history on next frame.
    pub reset: bool,
}

#[cfg(feature = "fsr_frame_generation")]
#[derive(Resource)]
struct FsrFgRuntime {
    library: Option<solarik_fsr_wgpu::Library>,
    context: Mutex<Option<(UVec2, solarik_fsr_wgpu::effect::EffectContext)>>,
    warned: AtomicBool,
}

#[cfg(feature = "dlss_frame_generation")]
#[derive(Resource)]
struct StreamlineShutdownGuard;

#[cfg(feature = "dlss_frame_generation")]
impl Drop for StreamlineShutdownGuard {
    fn drop(&mut self) {
        solarik_streamline_wgpu::shutdown();
    }
}

/// Plugin installing Streamline DLSS-G + Reflex and AMD FSR Frame Generation.
#[derive(Clone, Debug)]
pub struct FrameGenerationPlugin {
    /// Application NGX / Streamline project UUID string.
    pub project_id: String,
}

impl Plugin for FrameGenerationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameGenerationRuntimeCapabilities>()
            .init_resource::<FrameGenerationRuntimeSettings>()
            .add_plugins(ExtractResourcePlugin::<FrameGenerationRuntimeSettings>::default());
    }

    fn finish(&self, app: &mut App) {
        let wgpu_device = {
            let render_world = app.sub_app(RenderApp).world();
            render_world
                .resource::<RenderDevice>()
                .wgpu_device()
                .clone()
        };

        let mut caps = FrameGenerationRuntimeCapabilities::default();

        #[cfg(feature = "dlss_frame_generation")]
        {
            match solarik_streamline_wgpu::attach_device(&self.project_id, &wgpu_device) {
                Ok(sl_caps) => {
                    caps.dlss_g_supported = sl_caps.dlss_g_supported;
                    caps.dlss_mfg_supported = sl_caps.mfg_supported;
                    caps.max_generated_frames = sl_caps.max_generated_frames;
                    caps.reflex_supported = sl_caps.reflex_supported;
                }
                Err(error) => {
                    tracing::info!(%error, "Streamline DLSS-G unavailable on this system");
                }
            }
        }

        #[cfg(feature = "fsr_frame_generation")]
        {
            // AMD FidelityFX Frame Generation requires FFX swapchain
            // presentation (`FrameGenerationSwapChain` DX12/VK) to present
            // interpolated frames; keep `fsr_fg_supported = false` until that
            // swapchain hook is wired so we never advertise fake FG or dispatch
            // into dummy output handles.
            let library = solarik_fsr_wgpu::Library::load_for_device(&wgpu_device).ok();
            caps.fsr_fg_supported = false;
            app.sub_app_mut(RenderApp).insert_resource(FsrFgRuntime {
                library,
                context: Mutex::new(None),
                warned: AtomicBool::new(false),
            });
        }

        app.insert_resource(caps);
        #[cfg(feature = "dlss_frame_generation")]
        {
            #[cfg(all(windows, feature = "dx12_ngx_init"))]
            let retained = crate::graphics_session::retain(app);
            #[cfg(not(all(windows, feature = "dx12_ngx_init")))]
            let retained = false;
            // D3D12 shared NGX ownership is carried by the outer runner + SDK leases,
            // never by a main-world guard which can disappear before RenderApp.
            if !retained {
                app.insert_resource(StreamlineShutdownGuard);
            }
        }
        app.sub_app_mut(RenderApp)
            .insert_resource(caps)
            .add_systems(
                Core3d,
                run_frame_generation.after(Core3dSystems::EarlyPostProcess),
            );
    }
}

#[derive(Default)]
struct PrevCameraState {
    prev_clip_from_world: Option<Mat4>,
    logged_active: bool,
}

#[cfg(feature = "dlss_frame_generation")]
fn mat4_to_row_major(m: Mat4) -> [[f32; 4]; 4] {
    let cols = m.to_cols_array_2d();
    [
        [cols[0][0], cols[1][0], cols[2][0], cols[3][0]],
        [cols[0][1], cols[1][1], cols[2][1], cols[3][1]],
        [cols[0][2], cols[1][2], cols[2][2], cols[3][2]],
        [cols[0][3], cols[1][3], cols[2][3], cols[3][3]],
    ]
}

fn run_frame_generation(
    view: ViewQuery<(
        &ExtractedView,
        &ViewTarget,
        &ViewPrepassTextures,
        Option<&MainPassResolutionOverride>,
        Option<&TemporalJitter>,
    )>,
    settings: Res<FrameGenerationRuntimeSettings>,
    caps: Res<FrameGenerationRuntimeCapabilities>,
    frame_count: Res<FrameCount>,
    render_device: Res<RenderDevice>,
    #[cfg(feature = "fsr_frame_generation")] fsr_fg: Option<Res<FsrFgRuntime>>,
    mut prev: Local<PrevCameraState>,
    mut ctx: RenderContext,
) {
    let (extracted_view, view_target, prepass, res_override, jitter) = view.into_inner();
    let display_size = extracted_view.viewport.zw();
    if display_size.x == 0 || display_size.y == 0 {
        return;
    }
    let render_size = res_override.map_or(display_size, |r| r.0);
    let (Some(depth_tex), Some(mv_tex)) = (&prepass.depth, &prepass.motion_vectors) else {
        return;
    };

    let clip_from_view = extracted_view.clip_from_view;
    let view_from_clip = clip_from_view.inverse();
    let world_from_view = extracted_view.world_from_view.to_matrix();
    let view_from_world = world_from_view.inverse();
    let clip_from_world = clip_from_view * view_from_world;
    let world_from_clip = world_from_view * view_from_clip;
    let prev_clip_from_world = prev.prev_clip_from_world.unwrap_or(clip_from_world);
    let clip_to_prev_clip = prev_clip_from_world * world_from_clip;
    let prev_clip_to_clip = clip_to_prev_clip.inverse();
    prev.prev_clip_from_world = Some(clip_from_world);

    #[cfg(feature = "dlss_frame_generation")]
    if caps.dlss_g_supported && solarik_streamline_wgpu::is_ready() {
        let num_frames = if settings.use_dlss_g {
            settings.num_frames_to_generate
        } else {
            0
        };
        let mut constants = solarik_streamline_wgpu::abi::Constants {
            camera_view_to_clip: mat4_to_row_major(clip_from_view),
            clip_to_camera_view: mat4_to_row_major(view_from_clip),
            clip_to_prev_clip: mat4_to_row_major(clip_to_prev_clip),
            prev_clip_to_clip: mat4_to_row_major(prev_clip_to_clip),
            jitter_offset: jitter.map_or([0.0, 0.0], |j| (-j.offset).to_array()),
            mvec_scale: [1.0, 1.0],
            camera_pos: extracted_view.world_from_view.translation().into(),
            camera_up: extracted_view.world_from_view.up().as_vec3().into(),
            camera_right: extracted_view.world_from_view.right().as_vec3().into(),
            camera_fwd: extracted_view.world_from_view.forward().as_vec3().into(),
            camera_aspect_ratio: display_size.x as f32 / display_size.y.max(1) as f32,
            reset: if settings.reset {
                solarik_streamline_wgpu::abi::BOOLEAN_TRUE
            } else {
                solarik_streamline_wgpu::abi::BOOLEAN_FALSE
            },
            ..Default::default()
        };
        constants.depth_inverted = solarik_streamline_wgpu::abi::BOOLEAN_TRUE;
        constants.camera_motion_included = solarik_streamline_wgpu::abi::BOOLEAN_TRUE;

        let color_view = view_target.main_texture_view();
        match solarik_streamline_wgpu::record_frame(
            frame_count.0,
            num_frames,
            &constants,
            &depth_tex.texture.default_view,
            &mv_tex.texture.default_view,
            color_view,
            [render_size.x, render_size.y],
            [display_size.x, display_size.y],
            Some(ctx.command_encoder()),
        ) {
            Ok(state) => {
                if num_frames > 0 && !prev.logged_active {
                    prev.logged_active = true;
                    tracing::info!(
                        num_frames,
                        status = state.status,
                        presented = state.num_frames_actually_presented,
                        max_gen = state.max_generated_frames,
                        "Streamline DLSS-G frame generation active"
                    );
                } else if num_frames == 0 {
                    prev.logged_active = false;
                }
            }
            Err(error) => {
                tracing::debug!(%error, "Streamline record_frame skipped");
            }
        }
    }

    #[cfg(feature = "fsr_frame_generation")]
    if settings.use_fsr_fg
        && settings.num_frames_to_generate > 0
        && caps.fsr_fg_supported
        && let Some(fsr_fg) = fsr_fg.as_deref()
        && let Some(library) = fsr_fg.library.as_ref()
    {
        let mut guard = fsr_fg.context.lock().expect("FSR FG mutex poisoned");
        let need_create = guard.as_ref().is_none_or(|(size, _)| *size != display_size);
        if need_create {
            *guard = None;
            if let Ok(backend_dev) =
                solarik_fsr_wgpu::effect::BackendDevice::from_wgpu(render_device.wgpu_device())
            {
                let cfg = solarik_fsr_wgpu::effect::EffectConfig {
                    max_render_size: [render_size.x, render_size.y],
                    display_size: [display_size.x, display_size.y],
                    backbuffer_format: solarik_fsr_wgpu::surface_format(
                        view_target.main_texture_format(),
                    ),
                    ..Default::default()
                };
                match solarik_fsr_wgpu::effect::EffectContext::create(
                    library,
                    solarik_fsr_wgpu::effect::EffectKind::FrameGeneration,
                    backend_dev,
                    &cfg,
                ) {
                    Ok(effect) => {
                        *guard = Some((display_size, effect));
                    }
                    Err(error) => {
                        if !fsr_fg.warned.swap(true, Ordering::Relaxed) {
                            tracing::warn!(%error, "FSR Frame Generation context creation unavailable");
                        }
                    }
                }
            }
        }
        if let Some((_, effect)) = guard.as_mut()
            && let Ok(hudless) = solarik_fsr_wgpu::effect::resource_for_wgpu(
                view_target.main_texture_view(),
                solarik_fsr_wgpu::states::COMPUTE_READ,
            )
        {
            let mut fg_encoder = render_device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("fsr_frame_generation"),
            });
            let mut desc = solarik_fsr_wgpu::effects::DispatchDescFrameGeneration {
                header: solarik_fsr_wgpu::effect::FfxApiHeader {
                    kind: solarik_fsr_wgpu::effects::desc_types::FRAMEGENERATION_DISPATCH,
                    next: std::ptr::null_mut(),
                },
                command_list: std::ptr::null_mut(),
                present_color: hudless,
                outputs: [hudless; 4],
                num_generated_frames: 1,
                reset: settings.reset,
                backbuffer_transfer_function: 0,
                min_max_luminance: [0.0, 1000.0],
                generation_rect: solarik_fsr_wgpu::effects::Rect2D {
                    left: 0,
                    top: 0,
                    width: display_size.x as i32,
                    height: display_size.y as i32,
                },
                frame_id: frame_count.0 as u64,
            };
            if effect.dispatch_wgpu(&mut fg_encoder, &mut desc).is_ok() {
                ctx.add_command_buffer(fg_encoder.finish());
            }
        }
    }

    #[cfg(not(feature = "fsr_frame_generation"))]
    let _ = (
        &render_device,
        &ctx,
        &caps,
        &frame_count,
        &settings,
        &depth_tex,
        &mv_tex,
        &jitter,
        &view_target,
        &render_size,
    );
}
