//! Streamline-backed DLSS Super Resolution (`sl.dlss.dll`) and Ray Reconstruction
//! (`sl.dlss_d.dll`) prepare & render nodes for interposed Windows Vulkan.

use super::{
    Dlss, DlssFeature, DlssRayReconstructionFeature, DlssSuperResolutionFeature,
    ViewDlssRayReconstructionTextures,
};
use bevy_camera::{Camera3d, CameraMainTextureUsages, MainPassResolutionOverride};
use bevy_core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass, ViewPrepassTextures};
use bevy_diagnostic::FrameCount;
use bevy_ecs::{
    entity::Entity,
    query::With,
    system::{Commands, Query, Res},
};
use bevy_math::{Mat4, UVec2, Vec2, Vec4Swizzles};
use bevy_render::{
    camera::{MipBias, TemporalJitter},
    diagnostic::RecordDiagnostics,
    render_resource::{CommandEncoderDescriptor, TextureUsages},
    renderer::{RenderContext, RenderDevice, ViewQuery},
    view::{ExtractedView, ViewTarget},
};
use dlss_wgpu::DlssPerfQualityMode;

fn to_sl_mode(mode: DlssPerfQualityMode) -> u32 {
    use solarik_streamline_wgpu::abi::*;
    match mode {
        DlssPerfQualityMode::Auto | DlssPerfQualityMode::Performance => DLSS_MODE_MAX_PERFORMANCE,
        DlssPerfQualityMode::UltraPerformance => DLSS_MODE_ULTRA_PERFORMANCE,
        DlssPerfQualityMode::Balanced => DLSS_MODE_BALANCED,
        DlssPerfQualityMode::Quality => DLSS_MODE_MAX_QUALITY,
        DlssPerfQualityMode::Dlaa => DLSS_MODE_DLAA,
    }
}

fn fallback_render_size(output: UVec2, mode: DlssPerfQualityMode) -> UVec2 {
    let scale = match mode {
        DlssPerfQualityMode::UltraPerformance => 3.0_f32,
        DlssPerfQualityMode::Auto | DlssPerfQualityMode::Performance => 2.0,
        DlssPerfQualityMode::Balanced => 1.7,
        DlssPerfQualityMode::Quality => 1.5,
        DlssPerfQualityMode::Dlaa => 1.0,
    };
    UVec2::new(
        ((output.x as f32 / scale).round() as u32).max(1),
        ((output.y as f32 / scale).round() as u32).max(1),
    )
}

fn halton(mut index: u32, base: u32) -> f32 {
    let mut f = 1.0_f32;
    let mut r = 0.0_f32;
    let base_f = base as f32;
    while index > 0 {
        f /= base_f;
        r += f * (index % base) as f32;
        index /= base;
    }
    r
}

fn suggested_jitter(frame_index: u32, render_res: UVec2, output_res: UVec2) -> Vec2 {
    let ratio = (output_res.x as f32 / render_res.x.max(1) as f32).max(1.0);
    let phase_count = (8.0 * ratio * ratio).ceil() as u32;
    let phase = (frame_index % phase_count.max(1)) + 1;
    Vec2::new(halton(phase, 2) - 0.5, halton(phase, 3) - 0.5)
}

fn mat4_to_row_major(m: Mat4) -> [[f32; 4]; 4] {
    let cols = m.to_cols_array_2d();
    [
        [cols[0][0], cols[1][0], cols[2][0], cols[3][0]],
        [cols[0][1], cols[1][1], cols[2][1], cols[3][1]],
        [cols[0][2], cols[1][2], cols[2][2], cols[3][2]],
        [cols[0][3], cols[1][3], cols[2][3], cols[3][3]],
    ]
}

pub fn prepare_dlss_streamline<F: DlssFeature>(
    mut query: Query<
        (
            Entity,
            &ExtractedView,
            &Dlss<F>,
            &mut Camera3d,
            &mut CameraMainTextureUsages,
            &mut TemporalJitter,
            &mut MipBias,
        ),
        (
            With<Camera3d>,
            With<TemporalJitter>,
            With<DepthPrepass>,
            With<MotionVectorPrepass>,
        ),
    >,
    frame_count: Res<FrameCount>,
    mut commands: Commands,
) {
    let is_rr = core::any::TypeId::of::<F>()
        == core::any::TypeId::of::<DlssRayReconstructionFeature>();
    for (
        entity,
        view,
        dlss,
        mut camera_3d,
        mut camera_main_texture_usages,
        mut temporal_jitter,
        mut mip_bias,
    ) in &mut query
    {
        camera_main_texture_usages.0 |= TextureUsages::STORAGE_BINDING;
        let mut depth_usages = TextureUsages::from(camera_3d.depth_texture_usages);
        depth_usages |= TextureUsages::TEXTURE_BINDING;
        camera_3d.depth_texture_usages = depth_usages.into();

        let output_res = view.viewport.zw();
        if output_res.x == 0 || output_res.y == 0 {
            continue;
        }
        let sl_mode = to_sl_mode(dlss.perf_quality_mode);
        let render_res = solarik_streamline_wgpu::dlss_optimal_render_size(
            [output_res.x, output_res.y],
            sl_mode,
            is_rr,
        )
        .map(UVec2::from_array)
        .unwrap_or_else(|| fallback_render_size(output_res, dlss.perf_quality_mode));

        temporal_jitter.offset = suggested_jitter(frame_count.0, render_res, output_res);
        let ratio = render_res.x as f32 / output_res.x.max(1) as f32;
        mip_bias.0 = ratio.log2() - 1.0;

        commands
            .entity(entity)
            .insert(MainPassResolutionOverride(render_res));
    }
}

pub fn dlss_super_resolution_streamline(
    view: ViewQuery<(
        &ExtractedView,
        &Dlss<DlssSuperResolutionFeature>,
        &MainPassResolutionOverride,
        &TemporalJitter,
        &ViewTarget,
        &ViewPrepassTextures,
    )>,
    frame_count: Res<FrameCount>,
    render_device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (extracted_view, dlss, resolution_override, temporal_jitter, view_target, prepass) =
        view.into_inner();
    let (Some(depth), Some(motion_vectors)) = (&prepass.depth, &prepass.motion_vectors) else {
        return;
    };
    let output_res = extracted_view.viewport.zw();
    let render_res = resolution_override.0;
    let view_target = view_target.post_process_write();

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "dlss_super_resolution_sl");

    let mut sl_encoder = render_device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("streamline_dlss_sr"),
    });
    // Ensure wgpu-core opens the underlying VkCommandBuffer / ID3D12GraphicsCommandList
    // before extracting the raw handle for `slEvaluateFeature`.
    sl_encoder.push_debug_group("streamline_dlss_sr");
    sl_encoder.pop_debug_group();
    if let Err(err) = solarik_streamline_wgpu::evaluate_dlss_sr(
        frame_count.0,
        to_sl_mode(dlss.perf_quality_mode),
        [output_res.x, output_res.y],
        [render_res.x, render_res.y],
        (-temporal_jitter.offset).to_array(),
        dlss.reset,
        &view_target.source,
        &view_target.destination,
        &depth.texture.default_view,
        &motion_vectors.texture.default_view,
        None,
        &mut sl_encoder,
    ) {
        tracing::warn!(%err, "Streamline DLSS Super Resolution evaluation failed");
    }
    ctx.add_command_buffer(sl_encoder.finish());

    time_span.end(ctx.command_encoder());
}

pub fn dlss_ray_reconstruction_streamline(
    view: ViewQuery<(
        &ExtractedView,
        &Dlss<DlssRayReconstructionFeature>,
        &MainPassResolutionOverride,
        &TemporalJitter,
        &ViewTarget,
        &ViewPrepassTextures,
        &ViewDlssRayReconstructionTextures,
    )>,
    frame_count: Res<FrameCount>,
    render_device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (
        extracted_view,
        dlss,
        resolution_override,
        temporal_jitter,
        view_target,
        prepass,
        rr_textures,
    ) = view.into_inner();
    let Some(motion_vectors) = &prepass.motion_vectors else {
        return;
    };
    let output_res = extracted_view.viewport.zw();
    let render_res = resolution_override.0;
    let view_target = view_target.post_process_write();

    let world_from_view = extracted_view.world_from_view.to_matrix();
    let view_from_world = world_from_view.inverse();

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "dlss_ray_reconstruction_sl");

    let mut sl_encoder = render_device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("streamline_dlss_rr"),
    });
    // Ensure wgpu-core opens the underlying VkCommandBuffer / ID3D12GraphicsCommandList
    // before extracting the raw handle for `slEvaluateFeature`.
    sl_encoder.push_debug_group("streamline_dlss_rr");
    sl_encoder.pop_debug_group();
    if let Err(err) = solarik_streamline_wgpu::evaluate_dlss_rr(
        frame_count.0,
        to_sl_mode(dlss.perf_quality_mode),
        [output_res.x, output_res.y],
        [render_res.x, render_res.y],
        (-temporal_jitter.offset).to_array(),
        dlss.reset,
        mat4_to_row_major(view_from_world),
        mat4_to_row_major(world_from_view),
        &view_target.source,
        &view_target.destination,
        &rr_textures.depth.default_view,
        &motion_vectors.texture.default_view,
        &rr_textures.diffuse_albedo.default_view,
        &rr_textures.specular_albedo.default_view,
        &rr_textures.normal_roughness.default_view,
        Some(&rr_textures.specular_motion_vectors.default_view),
        None,
        &mut sl_encoder,
    ) {
        tracing::warn!(%err, "Streamline DLSS Ray Reconstruction evaluation failed");
    }
    ctx.add_command_buffer(sl_encoder.finish());

    time_span.end(ctx.command_encoder());
}
