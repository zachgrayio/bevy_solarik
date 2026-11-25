//! NVIDIA Deep Learning Super Sampling (DLSS).
//!
//! DLSS uses machine learning models to upscale and anti-alias images.
//!
//! Requires a NVIDIA RTX GPU, and the Windows/Linux Vulkan rendering backend. Does not work on other platforms.
//!
//! See https://github.com/bevyengine/dlss_wgpu for licensing requirements and setup instructions.
//!
//! # Usage
//! 1. Enable Bevy's `dlss` feature
//! 2. During app setup, insert the `DlssProjectId` resource before `DefaultPlugins`
//! 3. Check for the presence of `Option<Res<DlssSuperResolutionSupported>>` at runtime to see if DLSS is supported on the current machine
//! 4. Add the `Dlss` component to your camera entity, optionally setting a specific `DlssPerfQualityMode` (defaults to `Auto`)
//! 5. Optionally add sharpening via `ContrastAdaptiveSharpening`
//! 6. Custom rendering code, including third party crates, should account for the optional `MainPassResolutionOverride` to work with DLSS (see the `custom_render_phase` example)

mod extract;
mod newprivateprobe;
mod node;
mod prepare;
mod streamline;

pub use dlss_wgpu::DlssPerfQualityMode;

use bevy_app::{App, Plugin};
use bevy_camera::Hdr;
use bevy_core_pipeline::{
    prepass::{DepthPrepass, MotionVectorPrepass},
    schedule::{Core3d, Core3dSystems},
};
use bevy_ecs::prelude::*;
use bevy_math::{UVec2, Vec2};
use bevy_reflect::{Reflect, reflect_remote};
use bevy_render::{
    ExtractSchedule, Render, RenderApp, RenderSystems,
    camera::{MipBias, TemporalJitter},
    renderer::{
        RenderDevice, RenderQueue,
        raw_vulkan_init::{AdditionalVulkanFeatures, RawVulkanInitSettings},
    },
    texture::CachedTexture,
    view::prepare_view_targets,
};
use dlss_wgpu::{
    FeatureSupport,
    ray_reconstruction::{
        DlssRayReconstruction, DlssRayReconstructionDepthMode, DlssRayReconstructionRoughnessMode,
    },
    super_resolution::DlssSuperResolution,
};
use std::{
    any::Any,
    marker::PhantomData,
    ops::Deref,
    sync::{Arc, Mutex},
};
use tracing::info;
use uuid::Uuid;

/// Initializes DLSS support in the renderer. This must be registered before [`RenderPlugin`](bevy_render::RenderPlugin) because
/// it configures render init code.
#[derive(Default)]
pub struct DlssInitPlugin;

impl Plugin for DlssInitPlugin {
    #[allow(unsafe_code)]
    fn build(&self, app: &mut App) {
        let dlss_project_id = app.world().get_resource::<DlssProjectId>()
                        .expect("The `dlss` feature is enabled, but DlssProjectId was not added to the App before DlssInitPlugin.").0;
        let mut raw_vulkan_settings = app
            .world_mut()
            .get_resource_or_init::<RawVulkanInitSettings>();

        // SAFETY: this does not remove any instance features and only enables features that are supported
        unsafe {
            raw_vulkan_settings.add_create_instance_callback(
                move |mut args, additional_vulkan_features| {
                    let mut feature_support = FeatureSupport::default();
                    match dlss_wgpu::register_instance_extensions(
                        dlss_project_id,
                        &mut args,
                        &mut feature_support,
                    ) {
                        Ok(_) => {
                            if feature_support.super_resolution_supported {
                                additional_vulkan_features.insert::<DlssSuperResolutionSupported>();
                            }
                            if feature_support.ray_reconstruction_supported {
                                additional_vulkan_features
                                    .insert::<DlssRayReconstructionSupported>();
                            }
                        }
                        Err(err) => {
                            tracing::warn!(%err, "DLSS register_instance_extensions failed");
                        }
                    }
                },
            );
        }

        // SAFETY: this does not remove any device features and only enables features that are supported
        unsafe {
            raw_vulkan_settings.add_create_device_callback(
                move |mut args, adapter, additional_vulkan_features| {
                    let mut feature_support = FeatureSupport::default();
                    match dlss_wgpu::register_device_extensions(
                        dlss_project_id,
                        &mut args,
                        adapter,
                        &mut feature_support,
                    ) {
                        Ok(_) => {
                            if feature_support.super_resolution_supported {
                                additional_vulkan_features.insert::<DlssSuperResolutionSupported>();
                            } else {
                                additional_vulkan_features.remove::<DlssSuperResolutionSupported>();
                            }
                            if feature_support.ray_reconstruction_supported {
                                additional_vulkan_features
                                    .insert::<DlssRayReconstructionSupported>();
                            } else {
                                additional_vulkan_features
                                    .remove::<DlssRayReconstructionSupported>();
                            }
                        }
                        Err(err) => {
                            tracing::warn!(%err, "DLSS register_device_extensions failed");
                        }
                    }
                },
            )
        };
    }
}

/// Enables DLSS support. This requires [`DlssInitPlugin`] to function, which must be manually registered in the correct order
/// prior to registering this plugin.
#[derive(Default)]
pub struct DlssPlugin;

impl Plugin for DlssPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Dlss<DlssSuperResolutionFeature>>()
            .register_type::<Dlss<DlssRayReconstructionFeature>>();
    }

    fn finish(&self, app: &mut App) {
        let wgpu_device = {
            let render_world = app.sub_app(RenderApp).world();
            let render_device = render_world.resource::<RenderDevice>().wgpu_device();
            render_device.clone()
        };
        let is_vulkan = app
            .sub_app(RenderApp)
            .world()
            .get_resource::<bevy_render::renderer::RenderAdapterInfo>()
            .map(|info| format!("{:?}", info.backend).contains("Vulkan"))
            .unwrap_or(true);

        let (vk_sr_supported, vk_rr_supported) = {
            let features = app
                .sub_app_mut(RenderApp)
                .world()
                .resource::<AdditionalVulkanFeatures>();
            (
                features.has::<DlssSuperResolutionSupported>(),
                features.has::<DlssRayReconstructionSupported>(),
            )
        };
        if is_vulkan && !vk_sr_supported {
            return;
        }

        let project_id = app.world().get_resource::<DlssProjectId>()
            .expect("The `dlss` feature is enabled, but DlssProjectId was not added to the App before DlssPlugin.");

        // Drive DLSS SR and RR through Streamline (`sl.dlss.dll` / `sl.dlss_d.dll`)
        // instead of direct NGX (`dlss_wgpu`) whenever Streamline owns the device
        // for DLSS-G: on interposed Windows Vulkan it created the `VkDevice`, and
        // on Windows DX12 it upgraded the swapchain via `slUpgradeInterface`.
        // Use a single SR/RR evaluation provider per device. Runtime initialization
        // alone is not evidence of performance or successful frame generation.
        let streamline_owns_device = if is_vulkan {
            solarik_streamline_wgpu::is_vulkan_interposed()
        } else {
            solarik_streamline_wgpu::is_ready() || {
                let stream_project_id = project_id.0.to_string();
                solarik_streamline_wgpu::attach_device(&stream_project_id, &wgpu_device).is_ok()
            }
        };
        if streamline_owns_device {
            let project_id_str = project_id.0.to_string();
            match solarik_streamline_wgpu::attach_device(&project_id_str, &wgpu_device) {
                Ok(caps) if caps.dlss_sr_supported || caps.dlss_rr_supported => {
                    if caps.dlss_sr_supported {
                        app.insert_resource(DlssSuperResolutionSupported);
                    }
                    if caps.dlss_rr_supported {
                        app.insert_resource(DlssRayReconstructionSupported);
                    }
                    app.sub_app_mut(RenderApp)
                        .init_resource::<newprivateprobe::RrProbe>()
                        .add_systems(
                            Render,
                            newprivateprobe::readback.in_set(RenderSystems::Cleanup),
                        )
                        .add_systems(
                            ExtractSchedule,
                            (
                                extract::extract_dlss::<DlssSuperResolutionFeature>,
                                extract::extract_dlss::<DlssRayReconstructionFeature>,
                            ),
                        )
                        .add_systems(
                            Render,
                            (
                                streamline::prepare_dlss_streamline::<DlssSuperResolutionFeature>,
                                streamline::prepare_dlss_streamline::<DlssRayReconstructionFeature>,
                            )
                                .in_set(RenderSystems::PrepareViews)
                                .before(prepare_view_targets),
                        )
                        .add_systems(
                            Core3d,
                            (
                                streamline::dlss_super_resolution_streamline,
                                streamline::dlss_ray_reconstruction_streamline,
                            )
                                .chain()
                                .in_set(Core3dSystems::EarlyPostProcess),
                        );
                    info!(
                        dlss_sr = caps.dlss_sr_supported,
                        dlss_rr = caps.dlss_rr_supported,
                        "DLSS SR/RR enabled via Streamline"
                    );
                    return;
                }
                Ok(_) => {
                    info!("Streamline reported DLSS SR unsupported on interposed Vulkan");
                    return;
                }
                Err(err) => {
                    info!(?err, "Streamline attach_device failed on interposed Vulkan");
                    return;
                }
            }
        }

        let dlss_sdk = dlss_wgpu::DlssSdk::new(project_id.0, wgpu_device);
        let dlss_sdk = match dlss_sdk {
            Ok(sdk) => sdk,
            Err(err) => {
                info!(?err, "DLSS is not supported on this system");
                return;
            }
        };

        if let Some(owners) = app.world().get_resource::<DlssRuntimeKeepalives>() {
            let mut sdk = dlss_sdk.lock().unwrap();
            for owner in &owners.0 {
                sdk.retain_runtime(Arc::clone(owner));
            }
        }

        let ray_reconstruction_supported = if is_vulkan {
            vk_rr_supported
        } else {
            dlss_sdk.lock().unwrap().ray_reconstruction_supported()
        };

        app.insert_resource(DlssSuperResolutionSupported);
        if ray_reconstruction_supported {
            app.insert_resource(DlssRayReconstructionSupported);
        }

        app.sub_app_mut(RenderApp)
            .insert_resource(DlssSdk(dlss_sdk))
            .add_systems(
                ExtractSchedule,
                (
                    extract::extract_dlss::<DlssSuperResolutionFeature>,
                    extract::extract_dlss::<DlssRayReconstructionFeature>,
                ),
            )
            .add_systems(
                Render,
                (
                    prepare::prepare_dlss::<DlssSuperResolutionFeature>,
                    prepare::prepare_dlss::<DlssRayReconstructionFeature>,
                )
                    .in_set(RenderSystems::PrepareViews)
                    .before(prepare_view_targets),
            );

        app.sub_app_mut(RenderApp).add_systems(
            Core3d,
            (node::dlss_super_resolution, node::dlss_ray_reconstruction)
                .chain()
                .in_set(Core3dSystems::EarlyPostProcess),
        );
    }
}

/// Camera component to enable DLSS.
#[derive(Component, Reflect, Clone)]
#[reflect(Component)]
#[require(TemporalJitter, MipBias, DepthPrepass, MotionVectorPrepass, Hdr)]
pub struct Dlss<F: DlssFeature = DlssSuperResolutionFeature> {
    /// How much upscaling should be applied.
    #[reflect(remote = DlssPerfQualityModeRemoteReflect)]
    pub perf_quality_mode: DlssPerfQualityMode,
    /// Set to true to delete the saved temporal history (past frames).
    ///
    /// Useful for preventing ghosting when the history is no longer
    /// representative of the current frame, such as in sudden camera cuts.
    ///
    /// After setting this to true, it will automatically be toggled
    /// back to false at the end of the frame.
    pub reset: bool,
    #[reflect(ignore)]
    pub _phantom_data: PhantomData<F>,
}

impl<F: DlssFeature> Default for Dlss<F> {
    fn default() -> Self {
        Self {
            perf_quality_mode: Default::default(),
            reset: Default::default(),
            _phantom_data: Default::default(),
        }
    }
}

pub trait DlssFeature: Reflect + Clone + Default {
    type Context: Send;

    fn upscaled_resolution(context: &Self::Context) -> UVec2;

    fn render_resolution(context: &Self::Context) -> UVec2;

    fn suggested_jitter(
        context: &Self::Context,
        frame_number: u32,
        render_resolution: UVec2,
    ) -> Vec2;

    fn suggested_mip_bias(context: &Self::Context, render_resolution: UVec2) -> f32;

    fn new_context(
        upscaled_resolution: UVec2,
        perf_quality_mode: DlssPerfQualityMode,
        feature_flags: dlss_wgpu::DlssFeatureFlags,
        sdk: Arc<Mutex<dlss_wgpu::DlssSdk>>,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> Result<Self::Context, dlss_wgpu::DlssError>;
}

/// DLSS Super Resolution.
///
/// Only available when the [`DlssSuperResolutionSupported`] resource exists.
#[derive(Reflect, Clone, Default)]
pub struct DlssSuperResolutionFeature;

impl DlssFeature for DlssSuperResolutionFeature {
    type Context = DlssSuperResolution;

    fn upscaled_resolution(context: &Self::Context) -> UVec2 {
        context.upscaled_resolution().into()
    }

    fn render_resolution(context: &Self::Context) -> UVec2 {
        context.render_resolution().into()
    }

    fn suggested_jitter(
        context: &Self::Context,
        frame_number: u32,
        render_resolution: UVec2,
    ) -> Vec2 {
        context
            .suggested_jitter(frame_number, render_resolution.to_array())
            .into()
    }

    fn suggested_mip_bias(context: &Self::Context, render_resolution: UVec2) -> f32 {
        context.suggested_mip_bias(render_resolution.to_array())
    }

    fn new_context(
        upscaled_resolution: UVec2,
        perf_quality_mode: DlssPerfQualityMode,
        feature_flags: dlss_wgpu::DlssFeatureFlags,
        sdk: Arc<Mutex<dlss_wgpu::DlssSdk>>,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> Result<Self::Context, dlss_wgpu::DlssError> {
        DlssSuperResolution::new(
            upscaled_resolution.to_array(),
            perf_quality_mode,
            feature_flags,
            sdk,
            device.wgpu_device(),
            queue.deref(),
        )
    }
}

/// DLSS Ray Reconstruction.
///
/// Only available when the [`DlssRayReconstructionSupported`] resource exists.
#[derive(Reflect, Clone, Default)]
pub struct DlssRayReconstructionFeature;

impl DlssFeature for DlssRayReconstructionFeature {
    type Context = DlssRayReconstruction;

    fn upscaled_resolution(context: &Self::Context) -> UVec2 {
        context.upscaled_resolution().into()
    }

    fn render_resolution(context: &Self::Context) -> UVec2 {
        context.render_resolution().into()
    }

    fn suggested_jitter(
        context: &Self::Context,
        frame_number: u32,
        render_resolution: UVec2,
    ) -> Vec2 {
        context
            .suggested_jitter(frame_number, render_resolution.to_array())
            .into()
    }

    fn suggested_mip_bias(context: &Self::Context, render_resolution: UVec2) -> f32 {
        context.suggested_mip_bias(render_resolution.to_array())
    }

    fn new_context(
        upscaled_resolution: UVec2,
        perf_quality_mode: DlssPerfQualityMode,
        feature_flags: dlss_wgpu::DlssFeatureFlags,
        sdk: Arc<Mutex<dlss_wgpu::DlssSdk>>,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> Result<Self::Context, dlss_wgpu::DlssError> {
        DlssRayReconstruction::new(
            upscaled_resolution.to_array(),
            perf_quality_mode,
            // Not supported by ray reconstruction
            feature_flags - dlss_wgpu::DlssFeatureFlags::AutoExposure,
            DlssRayReconstructionRoughnessMode::Packed,
            DlssRayReconstructionDepthMode::Hardware,
            sdk,
            device.wgpu_device(),
            queue.deref(),
        )
    }
}

/// Optional producer/Streamline handshake for ray reconstruction guides.
///
/// Producers mark a frame ready only after recording every guide write, before the
/// RR node runs. Without this component, external producers retain responsibility
/// for supplying initialized, current-frame guides. Readiness records CPU command
/// recording, not GPU completion; render graph submission order remains required.
#[derive(Component)]
pub struct DlssRayReconstructionGuideStatus {
    produced_frame: std::sync::atomic::AtomicU64,
    evaluated_frame: std::sync::atomic::AtomicU64,
}

impl Default for DlssRayReconstructionGuideStatus {
    fn default() -> Self {
        Self {
            produced_frame: std::sync::atomic::AtomicU64::new(u64::MAX),
            evaluated_frame: std::sync::atomic::AtomicU64::new(u64::MAX),
        }
    }
}

impl DlssRayReconstructionGuideStatus {
    /// Publish completion of all guide command recording for this frame.
    pub fn mark_ready(&self, frame: u32) {
        self.produced_frame
            .store(u64::from(frame), std::sync::atomic::Ordering::Release);
    }

    /// Whether the producer recorded all guides for this exact frame.
    pub fn is_ready(&self, frame: u32) -> bool {
        self.produced_frame
            .load(std::sync::atomic::Ordering::Acquire)
            == u64::from(frame)
    }

    /// Reset native history on the first evaluation or after an evaluation gap.
    pub fn reset_needed(&self, frame: u32) -> bool {
        self.evaluated_frame
            .load(std::sync::atomic::Ordering::Acquire)
            != u64::from(frame.wrapping_sub(1))
    }

    /// Record a successful native evaluation, never a skipped or failed call.
    pub fn mark_evaluated(&self, frame: u32) {
        self.evaluated_frame
            .store(u64::from(frame), std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
mod guide_status_tests {
    use super::DlssRayReconstructionGuideStatus;

    #[test]
    fn pending_current_stale_and_resume() {
        let status = DlssRayReconstructionGuideStatus::default();
        assert!(!status.is_ready(0));
        assert!(!status.is_ready(u32::MAX));
        assert!(status.reset_needed(0));
        status.mark_ready(0);
        assert!(status.is_ready(0));
        assert!(status.reset_needed(0));
        status.mark_evaluated(0);
        assert!(!status.is_ready(1));
        assert!(!status.reset_needed(1));
        status.mark_ready(2);
        assert!(!status.is_ready(0));
        assert!(status.is_ready(2));
        assert!(status.reset_needed(2));
        status.mark_evaluated(2);
        assert!(!status.reset_needed(3));
    }

    #[test]
    fn frame_counter_wraps_without_colliding_with_sentinel() {
        let status = DlssRayReconstructionGuideStatus::default();
        assert!(status.reset_needed(u32::MAX));
        status.mark_ready(u32::MAX);
        assert!(status.is_ready(u32::MAX));
        status.mark_evaluated(u32::MAX);
        status.mark_ready(0);
        assert!(status.is_ready(0));
        assert!(!status.is_ready(u32::MAX));
        assert!(!status.reset_needed(0));
    }
}

/// Additional textures needed as inputs for [`DlssRayReconstructionFeature`].
#[derive(Component)]
pub struct ViewDlssRayReconstructionTextures {
    pub diffuse_albedo: CachedTexture,
    pub specular_albedo: CachedTexture,
    pub normal_roughness: CachedTexture,
    pub depth: CachedTexture,
    pub specular_motion_vectors: CachedTexture,
}

#[reflect_remote(DlssPerfQualityMode)]
#[derive(Default)]
enum DlssPerfQualityModeRemoteReflect {
    #[default]
    Auto,
    Dlaa,
    Quality,
    Balanced,
    Performance,
    UltraPerformance,
}

#[derive(Resource, Default)]
struct DlssRuntimeKeepalives(Vec<Arc<dyn Any + Send + Sync>>);

impl DlssRuntimeKeepalives {
    fn retain(&mut self, owner: Arc<dyn Any + Send + Sync>) {
        if !self.0.iter().any(|retained| Arc::ptr_eq(retained, &owner)) {
            self.0.push(owner);
        }
    }
}

/// Keep an external native runtime alive through every SR/RR feature release and parameter
/// destructor. Registration works before or after this plugin's `finish`, and neither world
/// owns a strong SDK reference that could cycle back through the retained runtime.
pub fn retain_dlss_runtime(app: &mut App, owner: Arc<dyn Any + Send + Sync>) {
    app.world_mut()
        .get_resource_or_init::<DlssRuntimeKeepalives>()
        .retain(Arc::clone(&owner));
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        if let Some(sdk) = render_app.world().get_resource::<DlssSdk>() {
            sdk.0.lock().unwrap().retain_runtime(Arc::clone(&owner));
        }
        render_app
            .world_mut()
            .get_resource_or_init::<DlssRuntimeKeepalives>()
            .retain(owner);
    }
}

#[derive(Resource)]
struct DlssSdk(Arc<Mutex<dlss_wgpu::DlssSdk>>);

/// Application-specific ID for DLSS.
///
/// See the DLSS programming guide for more info.
#[derive(Resource, Clone)]
pub struct DlssProjectId(pub Uuid);

/// When DLSS Super Resolution is supported by the current system, this resource will exist in the main world.
/// Otherwise this resource will be absent.
#[derive(Resource, Clone, Copy)]
pub struct DlssSuperResolutionSupported;

/// When DLSS Ray Reconstruction is supported by the current system, this resource will exist in the main world.
/// Otherwise this resource will be absent.
#[derive(Resource, Clone, Copy)]
pub struct DlssRayReconstructionSupported;
