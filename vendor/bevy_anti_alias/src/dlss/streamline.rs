//! Streamline-backed DLSS Super Resolution (`sl.dlss.dll`) and Ray Reconstruction
//! (`sl.dlss_d.dll`) prepare & render nodes for interposed Windows Vulkan.

use super::{
    Dlss, DlssFeature, DlssRayReconstructionFeature, DlssRayReconstructionGuideStatus,
    DlssSuperResolutionFeature, ViewDlssRayReconstructionTextures,
    newprivateprobe::{ProbeView, RrProbe},
};
use bevy_camera::{Camera3d, CameraMainTextureUsages, MainPassResolutionOverride, Projection};
use bevy_core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass, ViewPrepassTextures};
use bevy_diagnostic::FrameCount;
use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::With,
    system::{Commands, Local, Query, Res, ResMut},
};
use bevy_math::{Mat4, UVec2, Vec2, Vec4Swizzles};
use bevy_render::{
    camera::{ExtractedCamera, MipBias, TemporalJitter},
    diagnostic::RecordDiagnostics,
    render_resource::{
        CommandEncoder, CommandEncoderDescriptor, LoadOp, Operations, RenderPassColorAttachment,
        RenderPassDescriptor, StoreOp, TextureUsages, TextureView,
    },
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
    // Streamline's sl_matrix_helpers.h uses row vectors (translation in row 3).
    // Glam uses column vectors. Its columns therefore become Streamline rows:
    // transpose the mathematical transform as well as changing storage order.
    m.to_cols_array_2d()
}

// History lives on each extracted camera entity, so despawning a view releases it.
#[derive(Component)]
pub struct StreamlineViewHistory {
    clip_from_world: Mat4,
    previous_clip_from_world: Option<Mat4>,
    frame: u32,
    render_size: UVec2,
    output_size: UVec2,
}

/// Caches the most recent `(output_size, sl_mode, is_rr) -> render_size` query
/// so `slDLSS[D]GetOptimalSettings` is not called every frame with identical inputs.
#[derive(Default)]
pub struct OptimalRenderSizeCache {
    last: Option<((UVec2, u32, bool), UVec2)>,
}

impl OptimalRenderSizeCache {
    fn get_or_query(
        &mut self,
        output_res: UVec2,
        sl_mode: u32,
        is_rr: bool,
        query: impl FnOnce([u32; 2], u32, bool) -> Option<[u32; 2]>,
        fallback: impl FnOnce() -> UVec2,
    ) -> UVec2 {
        let key = (output_res, sl_mode, is_rr);
        if let Some((cached_key, cached_size)) = self.last
            && cached_key == key
        {
            return cached_size;
        }
        let size = query([output_res.x, output_res.y], sl_mode, is_rr)
            .map(UVec2::from_array)
            .unwrap_or_else(fallback);
        self.last = Some((key, size));
        size
    }
}

fn clip_reprojection(current: Mat4, previous: Option<Mat4>) -> (Mat4, Mat4) {
    let Some(previous) = previous else {
        return (Mat4::IDENTITY, Mat4::IDENTITY);
    };
    let to_previous = previous * current.inverse();
    (to_previous, to_previous.inverse())
}

fn common_constants(
    view: &ExtractedView,
    history: &StreamlineViewHistory,
    projection_info: &Projection,
    output_size: UVec2,
    jitter: Vec2,
    reset: bool,
) -> solarik_streamline_wgpu::abi::Constants {
    use solarik_streamline_wgpu::abi::{BOOLEAN_FALSE, BOOLEAN_TRUE, Constants};
    let projection = view.clip_from_view;
    let world_from_view = view.world_from_view.to_matrix();
    let clip_from_world = projection * world_from_view.inverse();
    let reset = reset || history.previous_clip_from_world.is_none();
    let previous = if reset {
        None
    } else {
        history.previous_clip_from_world
    };
    let (to_previous, from_previous) = clip_reprojection(clip_from_world, previous);
    let orthographic = projection.w_axis.w != 0.0;
    // Bevy uses reversed-Z, with a right-handed view looking down -Z.
    let (near, far, fov) = if orthographic {
        (
            (projection.w_axis.z - 1.0) / projection.z_axis.z,
            projection.w_axis.z / projection.z_axis.z,
            0.0,
        )
    } else {
        (
            projection.w_axis.z / (1.0 + projection.z_axis.z),
            // The matrix is infinite-reverse-Z, but the camera still owns a
            // finite culling far plane. f32::MAX is Streamline's INVALID_FLOAT.
            projection_info.far(),
            2.0 * projection.y_axis.y.recip().atan(),
        )
    };
    Constants {
        camera_view_to_clip: mat4_to_row_major(projection),
        clip_to_camera_view: mat4_to_row_major(projection.inverse()),
        clip_to_prev_clip: mat4_to_row_major(to_previous),
        prev_clip_to_clip: mat4_to_row_major(from_previous),
        camera_pos: view.world_from_view.translation().into(),
        camera_up: view.world_from_view.up().as_vec3().into(),
        camera_right: view.world_from_view.right().as_vec3().into(),
        camera_fwd: view.world_from_view.forward().as_vec3().into(),
        camera_near: near,
        camera_far: far,
        camera_fov: fov,
        camera_aspect_ratio: output_size.x as f32 / output_size.y.max(1) as f32,
        jitter_offset: (-jitter).to_array(),
        // Unlike direct NGX, Streamline multiplies normalized motion by the
        // render dimensions internally (sl.dlss/dlssEntry.cpp).
        mvec_scale: [-1.0, -1.0],
        depth_inverted: BOOLEAN_TRUE,
        camera_motion_included: BOOLEAN_TRUE,
        orthographic_projection: if orthographic {
            BOOLEAN_TRUE
        } else {
            BOOLEAN_FALSE
        },
        reset: if reset { BOOLEAN_TRUE } else { BOOLEAN_FALSE },
        ..Constants::default()
    }
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
            Option<&StreamlineViewHistory>,
            Option<&ProbeView>,
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
    probe: Res<RrProbe>,
    mut optimal_size_cache: Local<OptimalRenderSizeCache>,
) {
    let is_rr =
        core::any::TypeId::of::<F>() == core::any::TypeId::of::<DlssRayReconstructionFeature>();
    for (
        entity,
        view,
        dlss,
        mut camera_3d,
        mut camera_main_texture_usages,
        mut temporal_jitter,
        mut mip_bias,
        history,
        probe_view,
    ) in &mut query
    {
        if is_rr && probe.enabled && probe_view.is_none() {
            commands.entity(entity).insert(ProbeView::default());
        }
        camera_main_texture_usages.0 |= TextureUsages::STORAGE_BINDING;
        let mut depth_usages = TextureUsages::from(camera_3d.depth_texture_usages);
        depth_usages |= TextureUsages::TEXTURE_BINDING;
        camera_3d.depth_texture_usages = depth_usages.into();

        let output_res = view.viewport.zw();
        if output_res.x == 0 || output_res.y == 0 {
            continue;
        }
        let sl_mode = to_sl_mode(dlss.perf_quality_mode);
        let render_res = optimal_size_cache.get_or_query(
            output_res,
            sl_mode,
            is_rr,
            solarik_streamline_wgpu::dlss_optimal_render_size,
            || fallback_render_size(output_res, dlss.perf_quality_mode),
        );

        temporal_jitter.offset = suggested_jitter(frame_count.0, render_res, output_res);
        let ratio = render_res.x as f32 / output_res.x.max(1) as f32;
        mip_bias.0 = ratio.log2() - 1.0;

        let previous_clip_from_world = history
            .filter(|history| {
                history.frame.wrapping_add(1) == frame_count.0
                    && history.render_size == render_res
                    && history.output_size == output_res
                    && !dlss.reset
            })
            .map(|history| history.clip_from_world);
        commands.entity(entity).insert((
            MainPassResolutionOverride(render_res),
            StreamlineViewHistory {
                clip_from_world: view.clip_from_view * view.world_from_view.to_matrix().inverse(),
                previous_clip_from_world,
                frame: frame_count.0,
                render_size: render_res,
                output_size: output_res,
            },
        ));
    }
}

// Native writes do not update wgpu's initialization tracker. Load/Store asks
// wgpu to initialize only still-uninitialized destination subresources in the
// tracked preamble, before the SDK writes them on its separate fresh encoder.
fn initialize_native_output(encoder: &mut CommandEncoder, destination: &TextureView) {
    encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("streamline_native_output_init"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: destination,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

pub fn dlss_super_resolution_streamline(
    view: ViewQuery<(
        &ExtractedView,
        &Projection,
        &Dlss<DlssSuperResolutionFeature>,
        &MainPassResolutionOverride,
        &TemporalJitter,
        &StreamlineViewHistory,
        &ViewTarget,
        &ViewPrepassTextures,
    )>,
    frame_count: Res<FrameCount>,
    render_device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (
        extracted_view,
        projection_info,
        dlss,
        resolution_override,
        temporal_jitter,
        history,
        view_target,
        prepass,
    ) = view.into_inner();
    let (Some(depth), Some(motion_vectors)) = (&prepass.depth, &prepass.motion_vectors) else {
        return;
    };
    let output_res = extracted_view.viewport.zw();
    let render_res = resolution_override.0;
    let constants = common_constants(
        extracted_view,
        history,
        projection_info,
        output_res,
        temporal_jitter.offset,
        dlss.reset,
    );
    let view_target = view_target.post_process_write();

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "dlss_super_resolution_sl");

    let mut sl_encoder = render_device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("streamline_dlss_sr"),
    });
    initialize_native_output(ctx.command_encoder(), &view_target.destination);
    let evaluation_start = std::time::Instant::now();
    if let Err(err) = solarik_streamline_wgpu::evaluate_dlss_sr(
        frame_count.0,
        to_sl_mode(dlss.perf_quality_mode),
        [output_res.x, output_res.y],
        [render_res.x, render_res.y],
        &constants,
        &view_target.source,
        &view_target.destination,
        &depth.texture.default_view,
        &motion_vectors.texture.default_view,
        None,
        ctx.command_encoder(),
        &mut sl_encoder,
    ) {
        tracing::warn!(%err, "Streamline DLSS Super Resolution evaluation failed");
    }
    tracing::debug!(
        elapsed_ms = evaluation_start.elapsed().as_secs_f64() * 1000.0,
        "Streamline DLSS Super Resolution CPU evaluation duration"
    );
    ctx.add_command_buffer(sl_encoder.finish());

    time_span.end(ctx.command_encoder());
}

pub fn dlss_ray_reconstruction_streamline(
    view: ViewQuery<(
        &ExtractedView,
        &Projection,
        &Dlss<DlssRayReconstructionFeature>,
        &MainPassResolutionOverride,
        &TemporalJitter,
        &StreamlineViewHistory,
        &ViewTarget,
        &ViewPrepassTextures,
        &ViewDlssRayReconstructionTextures,
        Option<&DlssRayReconstructionGuideStatus>,
        Option<&ProbeView>,
        Option<&ExtractedCamera>,
    )>,
    frame_count: Res<FrameCount>,
    render_device: Res<RenderDevice>,
    mut probe: ResMut<RrProbe>,
    mut ctx: RenderContext,
) {
    let (
        extracted_view,
        projection_info,
        dlss,
        resolution_override,
        temporal_jitter,
        history,
        view_target,
        prepass,
        rr_textures,
        guide_status,
        probe_view,
        camera,
    ) = view.into_inner();
    if guide_status.is_some_and(|status| !status.is_ready(frame_count.0)) {
        return;
    }
    let Some(motion_vectors) = &prepass.motion_vectors else {
        return;
    };
    let output_res = extracted_view.viewport.zw();
    let render_res = resolution_override.0;
    let constants = common_constants(
        extracted_view,
        history,
        projection_info,
        output_res,
        temporal_jitter.offset,
        dlss.reset || guide_status.is_some_and(|status| status.reset_needed(frame_count.0)),
    );
    let view_target = view_target.post_process_write();

    let world_from_view = extracted_view.world_from_view.to_matrix();
    let view_from_world = world_from_view.inverse();

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "dlss_ray_reconstruction_sl");

    let mut sl_encoder = render_device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("streamline_dlss_rr"),
    });
    initialize_native_output(ctx.command_encoder(), &view_target.destination);
    if let Some(state) = probe_view {
        if dlss.reset
            || history.previous_clip_from_world.is_none()
            || guide_status.is_some_and(|status| status.reset_needed(frame_count.0))
        {
            state.reset_settling();
        }
    }
    // This copy is ordinary wgpu work on the tracked preamble, never sl_encoder.
    let mut probe_pair = probe.begin(
        probe_view,
        &render_device,
        (
            view_target.source_texture,
            view_target.destination_texture,
            &rr_textures.diffuse_albedo.texture,
        ),
        (render_res, output_res),
        (
            frame_count.0,
            extracted_view.retained_view_entity,
            camera.map(|camera| camera.exposure),
        ),
        ctx.command_encoder(),
    );
    let evaluation_start = std::time::Instant::now();
    if let Err(err) = solarik_streamline_wgpu::evaluate_dlss_rr(
        frame_count.0,
        to_sl_mode(dlss.perf_quality_mode),
        [output_res.x, output_res.y],
        [render_res.x, render_res.y],
        &constants,
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
        ctx.command_encoder(),
        &mut sl_encoder,
    ) {
        tracing::warn!(%err, "Streamline DLSS Ray Reconstruction evaluation failed");
        if let Some(state) = probe_view {
            state.reset_settling();
        }
    } else {
        if let Some(status) = guide_status {
            status.mark_evaluated(frame_count.0);
        }
        if let Some(state) = probe_view {
            state.evaluated();
        }
        if let Some(pair) = &mut probe_pair {
            pair.evaluation_succeeded = true;
        }
    }
    tracing::debug!(
        elapsed_ms = evaluation_start.elapsed().as_secs_f64() * 1000.0,
        "Streamline DLSS Ray Reconstruction CPU evaluation duration"
    );
    ctx.add_command_buffer(sl_encoder.finish());
    // add_command_buffer flushes the preamble before native RR. This new tracked
    // encoder therefore reads the destination after native RR in submission order.
    if let Some(pair) = probe_pair {
        probe.finish(pair, view_target.destination_texture, ctx.command_encoder());
    }

    time_span.end(ctx.command_encoder());
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_math::{Vec3, Vec4};

    fn reverse_projection() -> Mat4 {
        Mat4::from_cols(
            Vec4::new(1.2, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 1.8, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, -1.0),
            Vec4::new(0.0, 0.0, 0.1, 0.0),
        )
    }

    #[test]
    fn common_camera_data_uses_finite_far_and_normalized_motion() {
        let projection = Projection::Perspective(bevy_camera::PerspectiveProjection {
            near: 0.1,
            far: 4321.0,
            ..Default::default()
        });
        let view = ExtractedView {
            retained_view_entity: bevy_render::view::RetainedViewEntity::new(
                Entity::PLACEHOLDER.into(),
                None,
                0,
            ),
            clip_from_view: reverse_projection(),
            world_from_view: Default::default(),
            clip_from_world: None,
            target_format: bevy_render::render_resource::TextureFormat::Rgba16Float,
            viewport: bevy_math::UVec4::new(0, 0, 1280, 720),
            color_grading: Default::default(),
            invert_culling: false,
        };
        let history = StreamlineViewHistory {
            clip_from_world: view.clip_from_view,
            previous_clip_from_world: None,
            frame: 0,
            render_size: UVec2::new(762, 446),
            output_size: UVec2::new(1280, 720),
        };
        let constants = common_constants(
            &view,
            &history,
            &projection,
            history.output_size,
            Vec2::ZERO,
            false,
        );
        assert_eq!(constants.camera_far, 4321.0);
        assert_eq!(constants.mvec_scale, [-1.0, -1.0]);
        assert_eq!(
            constants.camera_view_to_clip,
            view.clip_from_view.to_cols_array_2d()
        );
    }

    #[test]
    fn missing_history_resets_reprojection_to_identity() {
        let current = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let (previous, inverse) = clip_reprojection(current, None);
        assert_eq!(previous, Mat4::IDENTITY);
        assert_eq!(inverse, Mat4::IDENTITY);
    }

    #[test]
    fn reprojection_maps_current_clip_to_previous_and_back() {
        let projection = reverse_projection();
        let current = projection * Mat4::from_translation(Vec3::new(-2.0, 1.0, 0.0));
        let previous = projection * Mat4::from_translation(Vec3::new(-1.0, 0.0, 0.0));
        let world = Vec4::new(3.0, 4.0, -10.0, 1.0);
        let (to_previous, from_previous) = clip_reprojection(current, Some(previous));
        let current_clip = current * world;
        assert!(
            (to_previous * current_clip - previous * world)
                .abs()
                .max_element()
                < 0.0001
        );
        assert!(
            (from_previous * (previous * world) - current_clip)
                .abs()
                .max_element()
                < 0.0001
        );
    }

    #[test]
    fn streamline_row_vectors_match_glam_column_vector_transforms() {
        let transform = Mat4::from_translation(Vec3::new(2.0, 3.0, 4.0));
        assert_eq!(mat4_to_row_major(transform)[3], [2.0, 3.0, 4.0, 1.0]);
        let projection = reverse_projection();
        for matrix in [transform, projection, projection * transform] {
            let point = Vec4::new(3.0, 4.0, -10.0, 1.0);
            let v = point.to_array();
            let rows = mat4_to_row_major(matrix);
            let row_product: [f32; 4] =
                core::array::from_fn(|column| (0..4).map(|row| v[row] * rows[row][column]).sum());
            assert!(
                (Vec4::from_array(row_product) - matrix * point)
                    .abs()
                    .max_element()
                    < 0.0001
            );
        }
    }

    #[test]
    fn optimal_render_size_cache_queries_only_when_inputs_change() {
        let mut cache = OptimalRenderSizeCache::default();
        let calls = std::cell::Cell::new(0u32);
        let mut run = |out: UVec2, mode: u32, is_rr: bool| {
            cache.get_or_query(
                out,
                mode,
                is_rr,
                |size, _, _| {
                    calls.set(calls.get() + 1);
                    Some([size[0] / 2, size[1] / 2])
                },
                || UVec2::ONE,
            )
        };

        assert_eq!(run(UVec2::new(1920, 1080), 1, true), UVec2::new(960, 540));
        assert_eq!(calls.get(), 1);
        // Repeated identical calls hit cache.
        assert_eq!(run(UVec2::new(1920, 1080), 1, true), UVec2::new(960, 540));
        assert_eq!(run(UVec2::new(1920, 1080), 1, true), UVec2::new(960, 540));
        assert_eq!(calls.get(), 1);
        // Changing mode invalidates.
        assert_eq!(run(UVec2::new(1920, 1080), 2, true), UVec2::new(960, 540));
        assert_eq!(calls.get(), 2);
        // Changing SR/RR kind invalidates.
        assert_eq!(run(UVec2::new(1920, 1080), 2, false), UVec2::new(960, 540));
        assert_eq!(calls.get(), 3);
        // Changing output resolution invalidates.
        assert_eq!(run(UVec2::new(2560, 1440), 2, false), UVec2::new(1280, 720));
        assert_eq!(calls.get(), 4);
    }
}
