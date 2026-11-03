mod extract;
#[cfg(feature = "graphics_debug")]
pub mod graphics_debug;
mod node;
#[cfg(feature = "nrd")]
mod nrd;
mod prepare;
#[cfg(feature = "graphics_debug")]
pub mod query_capture;
mod view_pipelines;

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
        {
            embedded_asset!(app, "nrd.wesl");
            app.sub_app_mut(RenderApp)
                .add_systems(RenderStartup, nrd::init_pipelines);
        }

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
                    // Otherwise unlit sprites are overwritten by the G-buffer behind them.
                    .before(main_opaque_pass_3d)
                    .after(crate::atmosphere::AtmosphereBackground)
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
}

impl Default for SolarikLighting {
    fn default() -> Self {
        Self {
            reset: true, // No temporal history on the first frame
            #[cfg(feature = "nrd")]
            nrd: true,
        }
    }
}
