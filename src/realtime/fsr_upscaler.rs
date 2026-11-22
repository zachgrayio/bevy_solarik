//! Camera-level scaled AMD FidelityFX FSR temporal upscaler for both Vulkan and DX12.
//!
//! Mirrors `bevy_anti_alias::dlss`: attaches as an optional [`FsrUpscaler`] component
//! on any HDR `Camera3d`, sets `MainPassResolutionOverride`, `TemporalJitter`, and
//! `MipBias`, and dispatches real FidelityFX FSR (`amd_fidelityfx_loader_vk.dll` /
//! `amd_fidelityfx_loader_dx12.dll`) in `Core3dSystems::EarlyPostProcess` — working
//! identically on **both** raster and ray-traced cameras, on **both** DX12 and Vulkan.

use super::FsrQualityMode;
use bevy_app::{App, Plugin};
use bevy_camera::{
    Camera, Camera3d, CameraMainTextureUsages, Hdr, MainPassResolutionOverride, Projection,
};
use bevy_core_pipeline::{
    prepass::{DepthPrepass, MotionVectorPrepass, ViewPrepassTextures},
    schedule::{Core3d, Core3dSystems},
};
use bevy_diagnostic::FrameCount;
use bevy_ecs::prelude::*;
use bevy_math::{UVec2, Vec2, Vec4Swizzles};
use bevy_reflect::{Reflect, std_traits::ReflectDefault};
use bevy_render::{
    ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems,
    camera::{MipBias, TemporalJitter},
    render_resource::{CommandEncoderDescriptor, TextureUsages},
    renderer::{RenderContext, RenderDevice, ViewQuery},
    sync_world::RenderEntity,
    view::{ExtractedView, ViewTarget, prepare_view_targets},
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Inserted on the main [`App`] when the runtime FidelityFX FSR provider loaded
/// cleanly for the active graphics backend (Vulkan or DX12).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FsrUpscalerSupported;

/// Camera component enabling scaled AMD FidelityFX FSR temporal upscaling.
#[derive(Component, Reflect, Clone, Copy, Debug)]
#[reflect(Component, Default, Clone)]
#[require(TemporalJitter, MipBias, DepthPrepass, MotionVectorPrepass, Hdr)]
pub struct FsrUpscaler {
    /// Quality preset (drives internal render resolution, jitter, and mip bias).
    pub quality: FsrQualityMode,
    /// Optional RCAS sharpening in `0.0..=1.0`.
    pub sharpness: f32,
    /// Reset temporal history on the next frame (e.g. camera cut or mode switch).
    pub reset: bool,
}

impl Default for FsrUpscaler {
    fn default() -> Self {
        Self {
            quality: FsrQualityMode::Performance,
            sharpness: 0.0,
            reset: true,
        }
    }
}

#[derive(Resource)]
struct FsrLibraryResource(solarik_fsr_wgpu::Library);

#[derive(Component)]
struct FsrRenderContext {
    context: Mutex<solarik_fsr_wgpu::Context>,
    quality: FsrQualityMode,
    upscaled_resolution: UVec2,
    render_resolution: UVec2,
    warned: AtomicBool,
}

/// Plugin installing camera-level scaled FSR upscaling (Vulkan + DX12, raster + RT).
#[derive(Default)]
pub struct FsrUpscalerPlugin;

impl Plugin for FsrUpscalerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<FsrUpscaler>();
    }

    fn finish(&self, app: &mut App) {
        let wgpu_device = {
            let render_world = app.sub_app(RenderApp).world();
            render_world
                .resource::<RenderDevice>()
                .wgpu_device()
                .clone()
        };
        let library = match solarik_fsr_wgpu::Library::load_for_device(&wgpu_device) {
            Ok(library) => library,
            Err(error) => {
                tracing::info!(%error, "FSR camera upscaler unavailable on this backend");
                return;
            }
        };

        app.insert_resource(FsrUpscalerSupported);

        app.sub_app_mut(RenderApp)
            .insert_resource(FsrLibraryResource(library))
            .add_systems(ExtractSchedule, extract_fsr_upscaler)
            .add_systems(
                Render,
                prepare_fsr_upscaler
                    .in_set(RenderSystems::PrepareViews)
                    .before(prepare_view_targets),
            )
            .add_systems(
                Core3d,
                run_fsr_upscaler.in_set(Core3dSystems::EarlyPostProcess),
            );
    }
}

fn extract_fsr_upscaler(
    mut commands: Commands,
    mut main_world: ResMut<MainWorld>,
    cleanup_query: Query<Has<FsrUpscaler>>,
) {
    let mut cameras = main_world.query_filtered::<(
        RenderEntity,
        &Camera,
        &Projection,
        Option<&mut FsrUpscaler>,
    ), With<Hdr>>();

    for (entity, camera, projection, mut fsr) in cameras.iter_mut(&mut main_world) {
        let Ok(mut entity_commands) = commands.get_entity(entity) else {
            continue;
        };
        if let Some(fsr) = fsr.as_deref_mut()
            && camera.is_active
            && projection.is_perspective()
        {
            entity_commands.insert(*fsr);
            fsr.reset = false;
        } else if cleanup_query.get(entity) == Ok(true) {
            entity_commands.remove::<(FsrUpscaler, FsrRenderContext, MainPassResolutionOverride)>();
        }
    }
}

fn prepare_fsr_upscaler(
    mut query: Query<
        (
            Entity,
            &ExtractedView,
            &FsrUpscaler,
            &mut Camera3d,
            &mut CameraMainTextureUsages,
            &mut TemporalJitter,
            &mut MipBias,
            Option<&FsrRenderContext>,
        ),
        (
            With<Camera3d>,
            With<TemporalJitter>,
            With<DepthPrepass>,
            With<MotionVectorPrepass>,
        ),
    >,
    library: Res<FsrLibraryResource>,
    render_device: Res<RenderDevice>,
    frame_count: Res<FrameCount>,
    mut commands: Commands,
) {
    for (
        entity,
        view,
        fsr,
        mut camera_3d,
        mut camera_main_texture_usages,
        mut temporal_jitter,
        mut mip_bias,
        existing,
    ) in &mut query
    {
        camera_main_texture_usages.0 |= TextureUsages::STORAGE_BINDING;
        let mut depth_usages = TextureUsages::from(camera_3d.depth_texture_usages);
        depth_usages |= TextureUsages::TEXTURE_BINDING;
        camera_3d.depth_texture_usages = depth_usages.into();

        let upscaled_resolution = view.viewport.zw();
        if upscaled_resolution.x == 0 || upscaled_resolution.y == 0 {
            continue;
        }
        let render_arr = fsr
            .quality
            .render_resolution([upscaled_resolution.x, upscaled_resolution.y]);
        let render_resolution = UVec2::new(render_arr[0].max(1), render_arr[1].max(1));

        let reuse = existing.is_some_and(|ctx| {
            ctx.upscaled_resolution == upscaled_resolution && ctx.quality == fsr.quality
        });

        let jitter = fsr
            .quality
            .suggested_jitter(frame_count.0, upscaled_resolution.x);
        temporal_jitter.offset = Vec2::from(jitter);
        mip_bias.0 = fsr.quality.suggested_mip_bias(upscaled_resolution.x);

        if reuse {
            continue;
        }

        let create_flags = solarik_fsr_wgpu::flags::HIGH_DYNAMIC_RANGE
            | solarik_fsr_wgpu::flags::DEPTH_INVERTED
            | solarik_fsr_wgpu::flags::AUTO_EXPOSURE;
        match solarik_fsr_wgpu::Context::new(
            &library.0,
            render_device.wgpu_device(),
            [render_resolution.x, render_resolution.y],
            [upscaled_resolution.x, upscaled_resolution.y],
            create_flags,
        ) {
            Ok(context) => {
                commands.entity(entity).insert((
                    FsrRenderContext {
                        context: Mutex::new(context),
                        quality: fsr.quality,
                        upscaled_resolution,
                        render_resolution,
                        warned: AtomicBool::new(false),
                    },
                    MainPassResolutionOverride(render_resolution),
                ));
            }
            Err(error) => {
                tracing::warn!(%error, "Failed to create FSR upscaler context; keeping native resolution");
                commands
                    .entity(entity)
                    .remove::<(FsrRenderContext, MainPassResolutionOverride)>();
            }
        }
    }
}

fn run_fsr_upscaler(
    view: ViewQuery<(
        &FsrUpscaler,
        &FsrRenderContext,
        &MainPassResolutionOverride,
        &TemporalJitter,
        &ViewTarget,
        &ViewPrepassTextures,
    )>,
    render_device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (fsr, fsr_ctx, resolution_override, temporal_jitter, view_target, prepass_textures) =
        view.into_inner();
    let (Some(prepass_depth), Some(prepass_motion)) =
        (&prepass_textures.depth, &prepass_textures.motion_vectors)
    else {
        return;
    };

    let post_process = view_target.post_process_write();
    let render_resolution = resolution_override.0.min(fsr_ctx.render_resolution);
    let upscale_resolution = fsr_ctx.upscaled_resolution;

    solarik_fsr_wgpu::Context::transition_resources(
        ctx.command_encoder(),
        &post_process.source,
        &prepass_depth.texture.default_view,
        &prepass_motion.texture.default_view,
        &post_process.destination,
    );

    let mut fsr_encoder = render_device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("fsr_upscaler"),
    });

    let dispatch_result = {
        let mut context = fsr_ctx.context.lock().expect("FSR context mutex poisoned");
        context.dispatch(solarik_fsr_wgpu::Dispatch {
            color: &post_process.source,
            depth: &prepass_depth.texture.default_view,
            motion_vectors: &prepass_motion.texture.default_view,
            output: &post_process.destination,
            exposure: None,
            reactive: None,
            transparency_and_composition: None,
            jitter: (-temporal_jitter.offset).to_array(),
            motion_vector_scale: (-render_resolution.as_vec2()).to_array(),
            render_size: [render_resolution.x, render_resolution.y],
            upscale_size: [upscale_resolution.x, upscale_resolution.y],
            sharpness: fsr.sharpness,
            frame_time_delta_ms: 16.67,
            pre_exposure: 1.0,
            reset: fsr.reset,
            camera_near: 0.1,
            camera_far: 1000.0,
            camera_fov_angle_vertical: core::f32::consts::FRAC_PI_4,
            view_space_to_meters_factor: 1.0,
            flags: 0,
            encoder: &mut fsr_encoder,
        })
    };

    match dispatch_result {
        Ok(()) => {
            ctx.add_command_buffer(fsr_encoder.finish());
        }
        Err(error) => {
            if !fsr_ctx.warned.swap(true, Ordering::Relaxed) {
                tracing::warn!(%error, "FSR upscaler dispatch failed");
            }
        }
    }
}
