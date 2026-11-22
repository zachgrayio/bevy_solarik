mod effects_chain;
mod extract;
#[cfg(feature = "fsr")]
mod fsr;
#[cfg(feature = "fsr")]
mod fsr_effects;
#[cfg(feature = "fsr")]
pub mod fsr_upscaler;
#[cfg(feature = "fsr")]
pub use fsr_upscaler::{FsrUpscaler, FsrUpscalerPlugin, FsrUpscalerSupported};
#[cfg(feature = "graphics_debug")]
pub mod graphics_debug;
mod node;
#[cfg(feature = "nrd")]
mod nrd;
mod prepare;
#[cfg(feature = "graphics_debug")]
pub mod query_capture;
mod view_pipelines;

pub use effects_chain::{
    ChainError, DescriptorChain, DescriptorNode, EffectChainPlan, EffectStep, FfxBackend,
    FsrEffectChainConfig, SkippedEffect, denoiser_signal_chain, desc_types,
    fg_swapchain_descriptors_exist, resolve_and_validate, resolve_effect_chain, validate_chain,
};

use crate::SolarikPlugins;
use bevy_app::{App, Plugin};
use bevy_asset::embedded_asset;
use bevy_camera::Hdr;
use bevy_core_pipeline::{
    core_3d::{main_opaque_pass_3d, main_transparent_pass_3d},
    prepass::{
        DeferredPrepass, DeferredPrepassDoubleBuffer, DepthPrepass, DepthPrepassDoubleBuffer,
        MotionVectorPrepass,
    },
    schedule::{Core3d, Core3dSystems},
};
use bevy_ecs::{component::Component, reflect::ReflectComponent, schedule::IntoScheduleConfigs};
use bevy_pbr::DefaultOpaqueRendererMethod;
use bevy_reflect::{Reflect, std_traits::ReflectDefault};
use bevy_render::render_resource::ShaderType;
use bevy_render::{
    ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems, renderer::RenderDevice,
};
use bevy_shader::load_shader_library;
use extract::extract_solari_lighting;
use node::{
    PrimaryGlassViews, init_solari_lighting_pipelines, prepare_primary_glass,
    restore_primary_glass, solarik_lighting,
};
use prepare::prepare_solari_lighting_resources;
use tracing::warn;

/// Raytraced direct and indirect lighting.
///
/// When using this plugin, it's highly recommended to set `shadow_maps_enabled: false` on all lights, as Solarik replaces
/// traditional shadow mapping.
pub struct SolarikLightingPlugin;

/// Runtime ray-lighting quality shared by the GI, world-cache and compositor shaders.
///
/// The values are delivered through a small uniform buffer so a live preference change
/// does not require another pipeline specialization or shader compile.
#[derive(
    Component,
    Reflect,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    ShaderType,
    bytemuck::Pod,
    bytemuck::Zeroable,
)]
#[repr(C)]
pub struct SolariQuality {
    /// Maximum spatial reservoir taps traced per pixel.
    pub spatial_taps: u32,
    /// Direct-light samples used when refreshing a world-cache cell.
    pub cache_samples: u32,
    /// Soft upper bound on world-cache cells refreshed per frame.
    pub cache_cell_updates: u32,
    /// One skips all indirect diffuse and specular dispatches.
    pub direct_only: u32,
}

impl Default for SolariQuality {
    fn default() -> Self {
        Self {
            spatial_taps: 5,
            cache_samples: 32,
            cache_cell_updates: 40_000,
            direct_only: 0,
        }
    }
}

impl SolariQuality {
    /// Whether only direct lighting should be resolved.
    pub fn is_direct_only(self) -> bool {
        self.direct_only != 0
    }
}

/// AMD FidelityFX FSR quality preset, the engine-facing equivalent of
/// `bevy_anti_alias::dlss::DlssPerfQualityMode`. It drives the camera's render
/// resolution, `TemporalJitter` and `MipBias`; see `docs/fsr.md`.
#[cfg(feature = "fsr")]
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FsrQualityMode {
    /// Temporal anti-aliasing at native resolution.
    NativeAA,
    /// 1.5x per dimension.
    Quality,
    /// 1.7x per dimension.
    Balanced,
    /// 2.0x per dimension.
    #[default]
    Performance,
    /// 3.0x per dimension.
    UltraPerformance,
}

#[cfg(feature = "fsr")]
impl FsrQualityMode {
    /// Map to the bindings crate's `FfxApi` enum value.
    pub(crate) fn native(self) -> solarik_fsr_wgpu::QualityMode {
        use solarik_fsr_wgpu::QualityMode as Q;
        match self {
            Self::NativeAA => Q::NativeAA,
            Self::Quality => Q::Quality,
            Self::Balanced => Q::Balanced,
            Self::Performance => Q::Performance,
            Self::UltraPerformance => Q::UltraPerformance,
        }
    }

    /// Render resolution for a display size, for `MainPassResolutionOverride`.
    pub fn render_resolution(self, display: [u32; 2]) -> [u32; 2] {
        self.native().render_resolution(display)
    }

    /// Subpixel jitter for this frame, in `[-0.5, 0.5]` pixel units.
    pub fn suggested_jitter(self, frame_index: u32, display_width: u32) -> [f32; 2] {
        let native = self.native();
        native.suggested_jitter(frame_index, native.jitter_phase_count(display_width))
    }

    /// Texture mip bias that keeps detail stable at the lower render resolution.
    pub fn suggested_mip_bias(self, display_width: u32) -> f32 {
        self.native().suggested_mip_bias(display_width)
    }
}

impl Plugin for SolarikLightingPlugin {
    fn build(&self, app: &mut App) {
        load_shader_library!(app, "gbuffer_utils.wesl");
        load_shader_library!(app, "primary_light_trace.wesl");
        load_shader_library!(app, "realtime_bindings.wesl");
        load_shader_library!(app, "presample_light_tiles.wesl");
        embedded_asset!(app, "restir_di.wesl");
        embedded_asset!(app, "restir_gi.wesl");
        load_shader_library!(app, "sky_sampling.wesl");
        load_shader_library!(app, "specular_gi.wesl");
        embedded_asset!(app, "primary_glass.wesl");
        embedded_asset!(app, "primary_foliage.wesl");
        load_shader_library!(app, "foliage_math.wesl");
        load_shader_library!(app, "surface_path.wesl");
        load_shader_library!(app, "world_cache_query.wesl");
        embedded_asset!(app, "world_cache_compact.wesl");
        load_shader_library!(app, "world_cache_priority.wesl");
        embedded_asset!(app, "world_cache_update.wesl");
        #[cfg(feature = "nrd")]
        embedded_asset!(app, "nrd.wesl");
        #[cfg(feature = "fsr")]
        embedded_asset!(app, "fsr.wesl");
        #[cfg(all(feature = "fsr", feature = "nrd"))]
        embedded_asset!(app, "fsr_rr.wesl");

        load_shader_library!(app, "resolve_dlss_rr_textures.wesl");

        app.insert_resource(DefaultOpaqueRendererMethod::deferred());
    }

    fn finish(&self, app: &mut App) {
        let render_app = app.sub_app_mut(RenderApp);

        let render_device = render_app.world().resource::<RenderDevice>();
        let features = render_device.features();
        if !features.contains(SolarikPlugins::required_wgpu_features()) {
            warn!(
                "SolarikLightingPlugin not loaded. GPU lacks support for required features: {:?}.",
                SolarikPlugins::required_wgpu_features().difference(features)
            );
            return;
        }

        let is_dx12 = render_app
            .world()
            .get_resource::<bevy_render::renderer::RenderAdapterInfo>()
            .is_some_and(|info| format!("{:?}", info.backend).contains("Dx12"));

        // NRD embeds SPIR-V only; never initialize or queue its pipelines on DX12.
        #[cfg(feature = "nrd")]
        if !is_dx12 {
            render_app.add_systems(RenderStartup, nrd::init_pipelines);
        }
        #[cfg(feature = "fsr")]
        render_app
            .init_resource::<fsr_effects::FsrSolariEffects>()
            .add_systems(RenderStartup, fsr::init_fsr);
        // FSR Ray Regeneration denoises Solari's split signals where NRD cannot run.
        #[cfg(all(feature = "fsr", feature = "nrd"))]
        if is_dx12 {
            render_app.add_systems(RenderStartup, fsr_effects::init_pipelines);
        }

        render_app
            .init_resource::<PrimaryGlassViews>()
            .add_systems(
                Render,
                restore_primary_glass
                    .after(RenderSystems::PrepareViews)
                    .before(RenderSystems::Queue),
            )
            .add_systems(
                Render,
                prepare_primary_glass
                    .after(RenderSystems::PrepareResources)
                    .before(RenderSystems::PrepareResourcesBatchPhases),
            )
            .add_systems(RenderStartup, init_solari_lighting_pipelines)
            .add_systems(ExtractSchedule, extract_solari_lighting)
            .add_systems(
                Render,
                prepare_solari_lighting_resources.in_set(RenderSystems::PrepareResources),
            )
            .add_systems(
                Core3d,
                solarik_lighting::<false>
                    // Resolve deferred lighting before forward opaque/masked content.
                    // Otherwise unlit sprites and emissive VFX are overwritten by the
                    // G-buffer behind them. The atmosphere sky is a later pass and does
                    // not depend on this resolve.
                    .before(main_opaque_pass_3d)
                    .before(main_transparent_pass_3d)
                    .in_set(Core3dSystems::MainPass),
            )
            .add_systems(
                Core3d,
                // Keep the existing primary glass/foliage compositor after raster opaque.
                solarik_lighting::<true>
                    .after(main_opaque_pass_3d)
                    .before(main_transparent_pass_3d)
                    .in_set(Core3dSystems::MainPass),
            );
    }
}

/// A component for a 3d camera entity to enable the Solarik raytraced lighting system.
///
/// Must be used with `CameraMainTextureUsages::default().with(TextureUsages::STORAGE_BINDING)`, and
/// `Msaa::Off`.
#[derive(Component, Reflect, Clone)]
#[reflect(Component, Default, Clone)]
#[require(
    Hdr,
    DeferredPrepass,
    DepthPrepass,
    MotionVectorPrepass,
    DeferredPrepassDoubleBuffer,
    DepthPrepassDoubleBuffer
)]
pub struct SolarikLighting {
    /// Set to true to delete the saved temporal history (past frames).
    ///
    /// Useful for preventing ghosting when the history is no longer
    /// representative of the current frame, such as in sudden camera cuts.
    /// Discards both screen-space reservoirs and world-cache history, so it is
    /// also suitable after coordinate-origin or material discontinuities.
    /// GPU allocations and the camera entity are retained.
    ///
    /// After setting this to true, it will automatically be toggled
    /// back to false at the end of the frame.
    pub reset: bool,
    /// Denoise opaque lighting with NRD RELAX when DLSS Ray Reconstruction is absent.
    /// Changing this flag resets NRD history on the next enabled frame.
    #[cfg(feature = "nrd")]
    pub nrd: bool,
    /// Upscale the resolved opaque lighting with AMD FSR. Opt-in,
    /// like `bevy_anti_alias::dlss::Dlss`; an unavailable provider keeps native
    /// resolution instead of failing the frame.
    #[cfg(feature = "fsr")]
    pub fsr: bool,
    /// FSR quality preset when `fsr` is enabled.
    #[cfg(feature = "fsr")]
    pub fsr_quality: FsrQualityMode,
    /// FSR extra sharpening, `0..1`.
    #[cfg(feature = "fsr")]
    pub fsr_sharpness: f32,
    /// Enable AMD FSR Ray Regeneration denoiser (DX12 / RX 9000+ only).
    #[cfg(feature = "fsr")]
    pub fsr_rr: bool,
    /// Enable AMD FSR Neural Radiance Cache (DX12 / RX 9000+ only).
    #[cfg(feature = "fsr")]
    pub fsr_radiance_cache: bool,
    /// Runtime GI and world-cache budgets for this camera.
    pub quality: SolariQuality,
}

impl Default for SolarikLighting {
    fn default() -> Self {
        Self {
            reset: true, // No temporal history on the first frame
            #[cfg(feature = "nrd")]
            nrd: true,
            #[cfg(feature = "fsr")]
            fsr: false,
            #[cfg(feature = "fsr")]
            fsr_quality: FsrQualityMode::default(),
            #[cfg(feature = "fsr")]
            fsr_sharpness: 0.0,
            #[cfg(feature = "fsr")]
            fsr_rr: false,
            #[cfg(feature = "fsr")]
            fsr_radiance_cache: false,
            quality: SolariQuality::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SolariQuality;

    #[test]
    fn default_ray_quality_preserves_original_gi_budget() {
        let quality = SolariQuality::default();
        assert_eq!(quality.spatial_taps, 5);
        assert_eq!(quality.cache_samples, 32);
        assert_eq!(quality.cache_cell_updates, 40_000);
        assert!(!quality.is_direct_only());
    }

    #[test]
    fn direct_only_is_an_explicit_quality_flag() {
        let mut quality = SolariQuality {
            spatial_taps: 2,
            ..Default::default()
        };
        assert!(!quality.is_direct_only());
        quality.direct_only = 1;
        assert!(quality.is_direct_only());
    }
}
