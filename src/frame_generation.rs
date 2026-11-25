//! Runtime Frame Generation plugin for both NVIDIA Streamline (DLSS-G 2x / DLSS
//! 4.5 MFG 3x/4x + Reflex) and AMD FidelityFX FSR Frame Generation across DX12
//! and Vulkan.
//!
//! Zero vendor build gating: both providers are dynamically loaded at runtime;
//! whichever hardware/driver/provider is available populates
//! [`FrameGenerationRuntimeCapabilities`] and drives live frames when requested
//! via [`FrameGenerationRuntimeSettings`].
//!
//! FSR FG follows AMD's swapchain model: `solarik_fsr_wgpu::frame_generation`
//! makes wgpu-hal create the window swapchain as a FidelityFX frame-generation
//! swapchain; each frame this plugin configures it and records the prepare pass
//! (depth, motion vectors, camera), and the swapchain generates and paces the
//! extra frame at present. [`present_counters`] reports rendered vs presented.

use bevy_app::{App, Plugin};
use bevy_camera::{MainPassResolutionOverride, Projection};
use bevy_core_pipeline::{
    prepass::ViewPrepassTextures,
    schedule::{Core3d, Core3dSystems},
};
use bevy_diagnostic::FrameCount;
use bevy_ecs::prelude::*;
use bevy_math::{Mat4, Vec4Swizzles};
use bevy_render::camera::ExtractedCamera;
#[cfg(any(feature = "dlss_frame_generation", feature = "fsr_frame_generation"))]
use bevy_render::render_resource::CommandEncoderDescriptor;
use bevy_render::{
    RenderApp,
    camera::TemporalJitter,
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    renderer::{RenderContext, RenderDevice, ViewQuery},
    view::{ExtractedView, ViewTarget},
};
#[cfg(feature = "fsr_frame_generation")]
use {
    bevy_camera::NormalizedRenderTarget,
    std::sync::atomic::{AtomicBool, Ordering},
};

/// Portable surface identity when the Streamline integration is not compiled.
#[cfg(not(feature = "dlss_frame_generation"))]
pub use bevy_render::view::window::WindowSurfaceStamp as SurfaceStamp;
/// Copied native surface identity used by the presentation pause protocol.
#[cfg(feature = "dlss_frame_generation")]
pub use solarik_streamline_wgpu::SurfaceStamp;

/// Temporary application-driven window transaction; never changes user FG settings.
#[derive(Clone, Debug)]
pub struct PresentationPause {
    #[cfg(feature = "dlss_frame_generation")]
    inner: solarik_streamline_wgpu::PresentationPause,
}

impl PresentationPause {
    /// Returns pending when a real Present cannot run (for example minimized).
    pub fn is_ready(&self) -> Result<bool, &'static str> {
        #[cfg(feature = "dlss_frame_generation")]
        {
            self.inner.is_ready()
        }
        #[cfg(not(feature = "dlss_frame_generation"))]
        {
            Ok(true)
        }
    }
    /// Mark issuance immediately before changing the native window request.
    /// Runtime ownership then survives cancellation of the caller's lease.
    pub fn mark_native_mutation_issued(&self, request_epoch: u64) -> Result<(), &'static str> {
        #[cfg(feature = "dlss_frame_generation")]
        {
            self.inner.mark_native_mutation_issued(request_epoch)
        }
        #[cfg(not(feature = "dlss_frame_generation"))]
        {
            let _ = request_epoch;
            Ok(())
        }
    }
    /// Arm only after native application acknowledgment, before requesting a new
    /// renderer configuration for this epoch and expected physical extent.
    pub fn arm_after_native_applied(
        &self,
        request_epoch: u64,
        expected_width: u32,
        expected_height: u32,
    ) -> Result<(), &'static str> {
        #[cfg(feature = "dlss_frame_generation")]
        {
            self.inner
                .arm_after_native_applied(request_epoch, expected_width, expected_height)
        }
        #[cfg(not(feature = "dlss_frame_generation"))]
        {
            let _ = (request_epoch, expected_width, expected_height);
            Ok(())
        }
    }
    /// Compatibility-only failed finish. Active SDK transactions require the
    /// native-applied epoch API, not an arbitrary following Present.
    pub fn finish(&self) {
        #[cfg(feature = "dlss_frame_generation")]
        self.inner.finish();
    }
    /// Whether a configured, matching new-surface Present completed.
    pub fn is_complete(&self) -> bool {
        #[cfg(feature = "dlss_frame_generation")]
        {
            self.inner.is_complete()
        }
        #[cfg(not(feature = "dlss_frame_generation"))]
        {
            true
        }
    }
}

/// Whether SDK presentation requires a target identity and pause transaction.
pub fn presentation_pause_required() -> bool {
    #[cfg(feature = "dlss_frame_generation")]
    {
        solarik_streamline_wgpu::presentation_pause_required()
    }
    #[cfg(not(feature = "dlss_frame_generation"))]
    {
        false
    }
}

/// Request a target-bound pause through the normal ordered rendering path.
pub fn request_presentation_pause_for_surface(baseline: SurfaceStamp) -> PresentationPause {
    #[cfg(not(feature = "dlss_frame_generation"))]
    let _ = baseline;
    PresentationPause {
        #[cfg(feature = "dlss_frame_generation")]
        inner: solarik_streamline_wgpu::request_presentation_pause_for_surface(baseline),
    }
}

/// Compatibility only: active SDK presentation rejects a missing surface identity.
pub fn request_presentation_pause() -> PresentationPause {
    PresentationPause {
        #[cfg(feature = "dlss_frame_generation")]
        inner: solarik_streamline_wgpu::request_presentation_pause(),
    }
}

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
    /// AMD FidelityFX FSR Frame Generation can present on the active backend: the
    /// provider loaded and its frame-generation swapchain hooks are installed.
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

/// Cumulative present counters, see [`present_counters`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresentCounters {
    /// Frames the application rendered and presented.
    pub rendered: u64,
    /// Presents the display swapchain issued (rendered + generated).
    pub presented: u64,
    /// Frames generated by frame generation.
    pub generated: u64,
}

/// Present counters for a frame-rate readout that separates rendered frames
/// from frames actually presented (generated frames included). `None` while no
/// frame-generation swapchain owns presentation; every presented frame is then a
/// rendered frame.
pub fn present_counters() -> Option<PresentCounters> {
    #[cfg(feature = "fsr_frame_generation")]
    if let Some(c) = solarik_fsr_wgpu::frame_generation::present_counters() {
        return Some(PresentCounters {
            rendered: c.rendered,
            presented: c.presented,
            generated: c.generated,
        });
    }
    None
}

/// Environment variable selecting the startup frame-generation presenter.
///
/// Accepted values (case-insensitive): `auto`, `streamline`, `dlss`, `fsr`,
/// `fidelityfx`, `off`, `disabled`. Unset or unknown values default to [`FrameGenerationProviderPreference::Auto`].
pub const FRAME_GENERATION_PROVIDER_ENV: &str = "SOLARIK_FRAME_GENERATION_PROVIDER";

/// Immutable startup preference for which frame-generation presenter owns
/// swapchain presentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrameGenerationProviderPreference {
    /// Prefer NVIDIA Streamline DLSS-G when supported, otherwise AMD FidelityFX FG.
    #[default]
    Auto,
    /// Prefer NVIDIA Streamline DLSS-G only; do not install FidelityFX FG presentation.
    Streamline,
    /// Use AMD FidelityFX FG exclusively for presentation; exclude `kFeatureDLSS_G`
    /// from Streamline while keeping Streamline SR/RR/Reflex available.
    FidelityFx,
    /// Disable frame-generation presentation hooks for both providers.
    Disabled,
}

impl FrameGenerationProviderPreference {
    /// Parse a startup provider string (trimmed, ASCII case-insensitive).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "streamline" | "dlss" => Some(Self::Streamline),
            "fsr" | "fidelityfx" => Some(Self::FidelityFx),
            "off" | "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }

    /// Startup preference parsed once from [`FRAME_GENERATION_PROVIDER_ENV`] (default `Auto`).
    pub fn from_env() -> Self {
        static PREFERENCE: std::sync::OnceLock<FrameGenerationProviderPreference> =
            std::sync::OnceLock::new();
        *PREFERENCE.get_or_init(|| match std::env::var(FRAME_GENERATION_PROVIDER_ENV) {
            Ok(raw) => Self::parse(&raw).unwrap_or_else(|| {
                tracing::warn!(
                    env = FRAME_GENERATION_PROVIDER_ENV,
                    value = %raw,
                    "Unknown frame generation provider preference; defaulting to Auto"
                );
                Self::Auto
            }),
            Err(_) => Self::Auto,
        })
    }

    /// Whether Streamline should load `kFeatureDLSS_G` and own swapchain proxies.
    pub const fn allows_streamline_dlss_g(self) -> bool {
        matches!(self, Self::Auto | Self::Streamline)
    }

    /// Whether AMD FidelityFX FG may install swapchain presentation hooks.
    pub const fn allows_fidelityfx(self) -> bool {
        matches!(self, Self::Auto | Self::FidelityFx)
    }
}

/// Diagnostic opt-out: `SOLARIK_FSR_FG_SWAPCHAIN=0` keeps the plain wgpu
/// swapchain and reports FSR Frame Generation as unsupported.
#[cfg(feature = "fsr_frame_generation")]
pub const FSR_FG_SWAPCHAIN_ENV: &str = "SOLARIK_FSR_FG_SWAPCHAIN";

/// Diagnostic: `SOLARIK_FSR_FG_DEBUG_CHECKS=1` creates the frame-generation context
/// with `FFX_FRAMEGENERATION_ENABLE_DEBUG_CHECKING`; violations reach tracing target
/// `fidelityfx`.
#[cfg(feature = "fsr_frame_generation")]
pub const FSR_FG_DEBUG_CHECKS_ENV: &str = "SOLARIK_FSR_FG_DEBUG_CHECKS";

#[cfg(feature = "fsr_frame_generation")]
#[derive(Resource, Default)]
struct FsrFgLog {
    failed: AtomicBool,
    generating: AtomicBool,
    debug_checks: bool,
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

/// Route Vulkan entry-point loading through `sl.interposer.dll` **before**
/// `RenderPlugin` builds the device.
///
/// DLSS-G on Vulkan has no "bring your own swapchain" entry point: Streamline's
/// `vkCreateSwapchainKHR`/`vkQueuePresentKHR` proxies must own present, and they
/// only accept a surface whose `VkInstance` the interposer created. Calling this
/// before device creation is what makes that true; without it Vulkan reports
/// frame generation as unavailable rather than crashing on `Invalid surface`.
///
/// DX12 also requires SDK initialization before graphics API creation; use
/// [`install_dx12_preinit`] for its factory proxy, without a Vulkan entry hook.
#[cfg(feature = "dlss_frame_generation")]
pub fn install_vulkan_preinit(project_id: &str) {
    let preference = FrameGenerationProviderPreference::from_env();
    let enable_dlss_g = preference.allows_streamline_dlss_g();
    match solarik_streamline_wgpu::preinit_with_options(project_id, false, enable_dlss_g) {
        Ok(()) => tracing::info!(
            ?preference,
            dlss_g = enable_dlss_g,
            "Streamline Vulkan pre-init armed"
        ),
        Err(error) => {
            tracing::info!(%error, "Streamline Vulkan pre-init unavailable; frame generation off")
        }
    }
}

#[cfg(not(feature = "dlss_frame_generation"))]
pub fn install_vulkan_preinit(_project_id: &str) {}

/// Initialize Streamline before DX12 graphics API creation so its factory-create
/// proxy is ready. Call before FFX registration and `RenderPlugin`; unlike Vulkan,
/// this does not install a Vulkan entry-library hook.
#[cfg(feature = "dlss_frame_generation")]
pub fn install_dx12_preinit(project_id: &str) {
    let preference = FrameGenerationProviderPreference::from_env();
    let enable_dlss_g = preference.allows_streamline_dlss_g();
    match solarik_streamline_wgpu::preinit_with_options(project_id, true, enable_dlss_g) {
        Ok(()) => tracing::info!(
            ?preference,
            dlss_g = enable_dlss_g,
            "Streamline DX12 pre-init armed"
        ),
        Err(error) => {
            tracing::info!(%error, "Streamline DX12 pre-init unavailable; frame generation off")
        }
    }
}

#[cfg(not(feature = "dlss_frame_generation"))]
pub fn install_dx12_preinit(_project_id: &str) {}

/// Total frames actually presented to the swapchain (real + generated) from the
/// active frame-generation presenter (Streamline `slDLSSGGetState` or FidelityFX
/// swapchain present counter).
pub fn total_presented_frames() -> u64 {
    #[cfg(feature = "dlss_frame_generation")]
    let sl = solarik_streamline_wgpu::total_presented_frames();
    #[cfg(not(feature = "dlss_frame_generation"))]
    let sl = 0u64;
    #[cfg(feature = "fsr_frame_generation")]
    let fsr = solarik_fsr_wgpu::frame_generation::present_counters().map_or(0, |c| c.presented);
    #[cfg(not(feature = "fsr_frame_generation"))]
    let fsr = 0u64;
    sl.saturating_add(fsr)
}

/// Plugin installing Streamline DLSS-G + Reflex and AMD FSR Frame Generation.
#[derive(Clone, Debug)]
pub struct FrameGenerationPlugin {
    /// Application NGX / Streamline project UUID string.
    pub project_id: String,
}

impl Plugin for FrameGenerationPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(all(windows, feature = "dlss_frame_generation"))]
        if !app.is_plugin_added::<crate::graphics_session::SessionPlugin>() {
            app.add_plugins(crate::graphics_session::SessionPlugin);
        }
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

        let provider_preference = FrameGenerationProviderPreference::from_env();
        #[cfg(not(any(feature = "dlss_frame_generation", feature = "fsr_frame_generation")))]
        let _ = provider_preference;
        let mut caps = FrameGenerationRuntimeCapabilities::default();

        #[cfg(feature = "dlss_frame_generation")]
        {
            solarik_streamline_wgpu::set_dlss_g_enabled(
                provider_preference.allows_streamline_dlss_g(),
            );
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

        // FSR FG presents through the FidelityFX frame-generation swapchain, which
        // must own the window swapchain from creation, so it is installed here
        // (before the first surface is configured) and never alongside the
        // Streamline DLSS-G swapchain hooks.
        #[cfg(feature = "fsr_frame_generation")]
        {
            let render_device = app
                .sub_app(RenderApp)
                .world()
                .resource::<RenderDevice>()
                .clone();
            caps.fsr_fg_supported = if provider_preference.allows_fidelityfx() {
                install_fsr_presentation(&render_device, caps.dlss_g_supported)
            } else {
                false
            };
            app.sub_app_mut(RenderApp).insert_resource(FsrFgLog {
                debug_checks: std::env::var(FSR_FG_DEBUG_CHECKS_ENV).is_ok_and(|v| v == "1"),
                ..FsrFgLog::default()
            });
        }

        app.insert_resource(caps);
        #[cfg(feature = "dlss_frame_generation")]
        {
            #[cfg(windows)]
            let retained = crate::graphics_session::retain(app);
            #[cfg(not(windows))]
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

#[cfg(feature = "fsr_frame_generation")]
fn install_fsr_presentation(render_device: &RenderDevice, dlss_g_supported: bool) -> bool {
    let device = render_device.wgpu_device();
    let Ok(library) = solarik_fsr_wgpu::Library::load_for_device(device) else {
        return false;
    };
    if dlss_g_supported {
        return false;
    }
    if std::env::var(FSR_FG_SWAPCHAIN_ENV).is_ok_and(|value| value == "0") {
        tracing::info!("FSR Frame Generation swapchain disabled by {FSR_FG_SWAPCHAIN_ENV}=0");
        return false;
    }
    match solarik_fsr_wgpu::frame_generation::install_presentation(&library, device) {
        Ok(backend) => {
            tracing::info!(?backend, "FSR Frame Generation swapchain hooks installed");
            true
        }
        Err(error) => {
            tracing::info!(%error, "FSR Frame Generation presentation unavailable on this backend");
            false
        }
    }
}

#[derive(Default)]
struct PrevCameraState {
    prev_clip_from_world: Option<Mat4>,
    logged_active: bool,
    #[cfg(feature = "fsr_frame_generation")]
    last_frame: Option<std::time::Instant>,
}

#[cfg(feature = "dlss_frame_generation")]
fn mat4_to_row_major(m: Mat4) -> [[f32; 4]; 4] {
    // Streamline uses row-vector transforms; Glam uses column vectors. Store
    // Glam columns as Streamline rows, including translation in the last row.
    m.to_cols_array_2d()
}

/// FidelityFX depth convention and camera planes for a Bevy projection.
#[cfg_attr(not(feature = "fsr_frame_generation"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct FgCamera {
    create_flags: u32,
    near: f32,
    far: f32,
    fov_y: f32,
}

/// Bevy perspective cameras use reversed, infinite-far depth
/// (`w_axis = (0, 0, near, 0)`); orthographic cameras use reversed finite depth
/// (`ndc_z = z / (far - near) + far / (far - near)`).
#[cfg_attr(not(feature = "fsr_frame_generation"), allow(dead_code))]
fn fg_camera(clip_from_view: Mat4) -> FgCamera {
    const DEPTH_INVERTED: u32 = 1 << 3;
    const DEPTH_INFINITE: u32 = 1 << 4;
    if clip_from_view.w_axis.w == 0.0 {
        let near = clip_from_view.w_axis.z;
        FgCamera {
            create_flags: DEPTH_INVERTED | DEPTH_INFINITE,
            near,
            far: near * 1.0e6,
            fov_y: 2.0 * (1.0 / clip_from_view.y_axis.y).atan(),
        }
    } else {
        let scale = clip_from_view.z_axis.z;
        let far = clip_from_view.w_axis.z / scale;
        FgCamera {
            create_flags: DEPTH_INVERTED,
            near: far - 1.0 / scale,
            far,
            fov_y: core::f32::consts::FRAC_PI_4,
        }
    }
}

fn run_frame_generation(
    view: ViewQuery<(
        &ExtractedView,
        &ViewTarget,
        &ViewPrepassTextures,
        Option<&MainPassResolutionOverride>,
        Option<&TemporalJitter>,
        Option<&ExtractedCamera>,
        Option<&Projection>,
    )>,
    settings: Res<FrameGenerationRuntimeSettings>,
    caps: Res<FrameGenerationRuntimeCapabilities>,
    frame_count: Res<FrameCount>,
    render_device: Res<RenderDevice>,
    #[cfg(feature = "fsr_frame_generation")] fsr_log: Res<FsrFgLog>,
    mut prev: Local<PrevCameraState>,
    mut ctx: RenderContext,
) {
    let (extracted_view, view_target, prepass, res_override, jitter, camera, projection_info) =
        view.into_inner();
    #[cfg(not(feature = "dlss_frame_generation"))]
    let _ = projection_info;
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
            // Streamline normalizes motion; it applies render dimensions when
            // forwarding normalized vectors to NGX's pixel-space parameters.
            mvec_scale: [-1.0, -1.0],
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
        let camera_planes = fg_camera(clip_from_view);
        constants.camera_near = camera_planes.near;
        constants.camera_far =
            projection_info.map_or(camera_planes.far, |projection| projection.far());
        constants.camera_fov = camera_planes.fov_y;
        constants.orthographic_projection = if clip_from_view.w_axis.w == 0.0 {
            solarik_streamline_wgpu::abi::BOOLEAN_FALSE
        } else {
            solarik_streamline_wgpu::abi::BOOLEAN_TRUE
        };
        constants.depth_inverted = solarik_streamline_wgpu::abi::BOOLEAN_TRUE;
        constants.camera_motion_included = solarik_streamline_wgpu::abi::BOOLEAN_TRUE;

        let color_view = view_target.main_texture_view();
        // Keep native tagging on a fresh encoder, after tracked input barriers.
        // Turning DLSS-G off needs no native command buffer.
        let mut sl_encoder = (num_frames > 0).then(|| {
            render_device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("streamline_dlss_fg"),
            })
        });
        match solarik_streamline_wgpu::record_frame(
            frame_count.0,
            num_frames,
            &constants,
            &depth_tex.texture.default_view,
            &mv_tex.texture.default_view,
            color_view,
            [render_size.x, render_size.y],
            [display_size.x, display_size.y],
            ctx.command_encoder(),
            sl_encoder.as_mut(),
        ) {
            Ok(state) => {
                if num_frames > 0 && !prev.logged_active {
                    prev.logged_active = true;
                    tracing::info!(
                        num_frames,
                        max_gen = state.max_generated_frames,
                        "Streamline DLSS-G frame generation request recorded"
                    );
                } else if num_frames == 0 {
                    prev.logged_active = false;
                }
            }
            Err(error) => {
                tracing::debug!(%error, "Streamline record_frame skipped");
            }
        }
        if let Some(sl_encoder) = sl_encoder {
            ctx.add_command_buffer(sl_encoder.finish());
        }
    }

    // Only the view presented to a window drives the frame-generation swapchain.
    #[cfg(feature = "fsr_frame_generation")]
    if caps.fsr_fg_supported
        && camera.is_some_and(|c| matches!(c.target, Some(NormalizedRenderTarget::Window(_))))
        && let Some(backbuffer_format) = view_target.out_texture_view_format()
    {
        use solarik_fsr_wgpu::frame_generation::{FrameInputs, FrameStatus, Prepare};

        let enabled = settings.use_fsr_fg && settings.num_frames_to_generate > 0;
        let now = std::time::Instant::now();
        let frame_time_delta_ms = prev
            .last_frame
            .replace(now)
            .map_or(16.67, |last| (now - last).as_secs_f32() * 1000.0);
        let depth = &depth_tex.texture.default_view;
        let motion_vectors = &mv_tex.texture.default_view;
        if enabled {
            // FidelityFX reads both as shader resources in a separate command buffer.
            solarik_fsr_wgpu::frame_generation::transition_prepare_inputs(
                ctx.command_encoder(),
                depth,
                motion_vectors,
            );
        }
        let fg_camera = fg_camera(clip_from_view);
        let inputs = FrameInputs {
            enabled,
            display_size: [display_size.x, display_size.y],
            max_render_size: [display_size.x, display_size.y],
            backbuffer_format,
            create_flags: fg_camera.create_flags
                | if fsr_log.debug_checks {
                    solarik_fsr_wgpu::effects::framegeneration_create_flags::ENABLE_DEBUG_CHECKING
                } else {
                    0
                },
            dispatch_flags: 0,
            prepare: Prepare {
                depth,
                motion_vectors,
                render_size: [render_size.x, render_size.y],
                jitter: jitter.map_or([0.0, 0.0], |j| (-j.offset).to_array()),
                motion_vector_scale: (-render_size.as_vec2()).to_array(),
                frame_time_delta_ms,
                reset: settings.reset,
                camera_near: fg_camera.near,
                camera_far: fg_camera.far,
                camera_fov_angle_vertical: fg_camera.fov_y,
                view_space_to_meters_factor: 1.0,
                camera_position: extracted_view.world_from_view.translation().into(),
                camera_up: extracted_view.world_from_view.up().as_vec3().into(),
                camera_right: extracted_view.world_from_view.right().as_vec3().into(),
                camera_forward: extracted_view.world_from_view.forward().as_vec3().into(),
                frame_id: 0,
            },
        };
        let mut fg_encoder = render_device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("fsr_frame_generation_prepare"),
        });
        match solarik_fsr_wgpu::frame_generation::record_frame(
            render_device.wgpu_device(),
            &mut fg_encoder,
            &inputs,
        ) {
            Ok(FrameStatus::Configured { generating }) => {
                if generating {
                    ctx.add_command_buffer(fg_encoder.finish());
                }
                if fsr_log.generating.swap(generating, Ordering::Relaxed) != generating {
                    tracing::info!(generating, "FSR Frame Generation state changed");
                }
            }
            Ok(FrameStatus::NoSwapchain) => {}
            Err(error) => {
                if !fsr_log.failed.swap(true, Ordering::Relaxed) {
                    tracing::warn!(%error, "FSR Frame Generation configure/prepare failed");
                }
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
        &camera,
    );
}

#[cfg(test)]
mod tests {
    use super::{FrameGenerationProviderPreference, fg_camera};
    use bevy_math::Mat4;

    #[test]
    fn provider_preference_parses_all_aliases_and_policies() {
        use FrameGenerationProviderPreference as Pref;

        assert_eq!(Pref::default(), Pref::Auto);
        for (raw, expected) in [
            ("auto", Pref::Auto),
            (" AUTO ", Pref::Auto),
            ("streamline", Pref::Streamline),
            ("dlss", Pref::Streamline),
            ("DLSS", Pref::Streamline),
            ("fsr", Pref::FidelityFx),
            ("fidelityfx", Pref::FidelityFx),
            ("FidelityFX", Pref::FidelityFx),
            ("off", Pref::Disabled),
            ("disabled", Pref::Disabled),
            ("DISABLED", Pref::Disabled),
        ] {
            assert_eq!(Pref::parse(raw), Some(expected), "raw={raw:?}");
        }
        for unknown in ["", "unknown", "xess", "metal"] {
            assert_eq!(Pref::parse(unknown), None, "unknown={unknown:?}");
        }

        assert!(Pref::Auto.allows_streamline_dlss_g());
        assert!(Pref::Auto.allows_fidelityfx());
        assert!(Pref::Streamline.allows_streamline_dlss_g());
        assert!(!Pref::Streamline.allows_fidelityfx());
        assert!(!Pref::FidelityFx.allows_streamline_dlss_g());
        assert!(Pref::FidelityFx.allows_fidelityfx());
        assert!(!Pref::Disabled.allows_streamline_dlss_g());
        assert!(!Pref::Disabled.allows_fidelityfx());
    }

    #[test]
    fn perspective_cameras_use_inverted_infinite_depth() {
        let fov = 0.9_f32;
        let camera = fg_camera(Mat4::perspective_infinite_reverse_rh(fov, 16.0 / 9.0, 0.1));
        assert_eq!(camera.create_flags, (1 << 3) | (1 << 4));
        assert!((camera.near - 0.1).abs() < 1e-6);
        assert!((camera.fov_y - fov).abs() < 1e-5);
    }

    #[test]
    fn orthographic_cameras_recover_reversed_finite_planes() {
        let camera = fg_camera(Mat4::orthographic_rh(-8.0, 8.0, -4.5, 4.5, 500.0, 0.5));
        assert_eq!(camera.create_flags, 1 << 3);
        assert!((camera.near - 0.5).abs() < 1e-3);
        assert!((camera.far - 500.0).abs() < 1e-2);
    }
}
