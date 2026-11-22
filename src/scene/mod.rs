mod assembly;
mod input_roots;
mod instance_rows;
pub use assembly::{RaytracingAssembly3d, RaytracingAssemblyPart};
mod binder;
mod blas;
pub mod collimated;
mod dependencies;
mod extract;
pub(crate) mod history;
mod instance_changes;
pub mod placement;

/// Enable full active-index receipts only for an explicit diagnostic capture.
#[derive(bevy_ecs::resource::Resource)]
pub struct CaptureSceneIndices;

#[cfg(feature = "graphics_debug")]
pub mod graphics_debug;
mod light_sampling;
#[cfg(feature = "graphics_debug")]
mod memory_profile;
mod ray_settings;
mod tlas;
mod types;

use bevy_shader::load_shader_library;
pub use binder::{RaytracingSceneBindings, SolarikAlphaTesting, SolarikSkyLight};
pub use ray_settings::SolarikRaySettings;
pub use types::RaytracingMesh3d;

use crate::SolarikPlugins;
use bevy_app::{App, Plugin};
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_render::{
    ExtractSchedule, GpuResourceAppExt, Render, RenderApp, RenderSystems,
    extract_resource::ExtractResourcePlugin,
    mesh::{
        RenderMesh,
        allocator::{MeshAllocatorSettings, allocate_and_free_meshes},
    },
    render_asset::prepare_assets,
    render_resource::BufferUsages,
    renderer::RenderDevice,
};
use binder::prepare_raytracing_scene_bindings;
use blas::{BlasManager, compact_raytracing_blas, prepare_raytracing_blas};
use extract::{StandardMaterialAssets, extract_raytracing_scene, extract_standard_materials};
use tracing::warn;

/// Creates acceleration structures and binding arrays of resources for raytracing.
pub struct RaytracingScenePlugin;

impl Plugin for RaytracingScenePlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::surface_detail::SurfaceDetailPlugin>() {
            app.add_plugins(crate::surface_detail::SurfaceDetailPlugin);
        }
        if !app.is_plugin_added::<collimated::CollimatedEmissionPlugin>() {
            app.add_plugins(collimated::CollimatedEmissionPlugin);
        }
        app.add_plugins(crate::gaussian::GaussianDielectricPlugin);
        app.add_plugins(crate::lommel::LommelSeeligerPlugin);
        app.add_plugins(crate::material::MaterialRegistrationPlugin);
        app.add_plugins(crate::publication::PublicationPlugin);
        app.add_plugins(crate::rings::RingPlugin);
        app.add_plugins(crate::filtered_canopy::CanopyPlugin);
        crate::coarse_scene::load_shader(app);
        crate::load_cloud_field(app);
        load_shader_library!(app, "brdf.wesl");
        load_shader_library!(app, "light_medium.wesl");
        load_shader_library!(app, "thin_glass.wesl");
        load_shader_library!(app, "scene_hit.wesl");
        load_shader_library!(app, "raytracing_scene_bindings.wesl");
        load_shader_library!(app, "sampling.wesl");

        // The sky is optional; the resource exists so the binder can read it
        // (no image = no sky, upstream behaviour).
        app.add_systems(bevy_app::First, assembly::previous_transforms);
        app.init_resource::<bevy_render::scene_readiness::SceneGeometryReadiness>();
        app.init_resource::<SolarikSkyLight>();
        app.init_resource::<SolarikAlphaTesting>();
        app.init_resource::<SolarikRaySettings>();
    }

    fn finish(&self, app: &mut App) {
        // `CanopyPlugin` registers Render-schedule consumers of
        // `SceneGeometryReadiness` (`prepare_assets<GpuRegion>` and
        // `filtered_canopy::scene::prepare`) unconditionally, so the shared
        // receipt must reach the render world on every backend. Publishing it
        // only after the raytracing capability gate below made those systems
        // panic with "Resource does not exist" at the first frame on raster-only
        // backends (wgpu D3D12, older AMD/Intel GPUs without ray query, Metal).
        // `Clone` shares one `Arc<Shared>`, so app-world requests and
        // render-world publishes stay coherent regardless of raytracing support.
        share_scene_readiness(app);

        let render_app = app.sub_app_mut(RenderApp);
        let render_device = render_app.world().resource::<RenderDevice>();
        let features = render_device.features();
        if !features.contains(SolarikPlugins::required_wgpu_features()) {
            warn!(
                "RaytracingScenePlugin not loaded. GPU lacks support for required features: {:?}.",
                SolarikPlugins::required_wgpu_features().difference(features)
            );
            return;
        }

        app.add_plugins((
            ExtractResourcePlugin::<SolarikSkyLight>::default(),
            ExtractResourcePlugin::<SolarikAlphaTesting>::default(),
            ExtractResourcePlugin::<SolarikRaySettings>::default(),
        ));

        let render_app = app.sub_app_mut(RenderApp);

        render_app
            .world_mut()
            .resource_mut::<MeshAllocatorSettings>()
            .extra_buffer_usages |= BufferUsages::BLAS_INPUT | BufferUsages::STORAGE;
        render_app.init_resource::<SolarikSkyLight>();
        render_app.init_resource::<SolarikAlphaTesting>();
        render_app.init_resource::<SolarikRaySettings>();

        let scene_bindings =
            RaytracingSceneBindings::for_device(render_app.world().resource::<RenderDevice>());
        render_app
            .init_gpu_resource::<BlasManager>()
            .init_gpu_resource::<StandardMaterialAssets>()
            .insert_resource(scene_bindings)
            .add_systems(
                ExtractSchedule,
                (
                    extract_raytracing_scene,
                    extract_standard_materials,
                    assembly::extract,
                ),
            )
            .add_systems(
                Render,
                (
                    prepare_raytracing_blas
                        .in_set(RenderSystems::PrepareAssets)
                        .before(prepare_assets::<RenderMesh>)
                        .after(allocate_and_free_meshes),
                    compact_raytracing_blas
                        .in_set(RenderSystems::PrepareAssets)
                        .after(prepare_raytracing_blas),
                    prepare_raytracing_scene_bindings
                        .in_set(RenderSystems::PrepareResources)
                        .after(crate::filtered_canopy::PrepareCanopy),
                    blas::publish_readiness
                        .in_set(RenderSystems::PrepareResources)
                        .after(prepare_raytracing_scene_bindings),
                ),
            );
    }
}

/// Clone the app-world `SceneGeometryReadiness` into the render world.
///
/// The canopy prepass, `prepare_assets<GpuRegion>` and the geometry binder all
/// read this receipt in the `Render` schedule, and `CanopyPlugin` registers them
/// for every backend. It is an `Arc`-backed shared receipt, so publishing a
/// clone preserves the app↔render request/publication link. Doing this outside
/// the raytracing capability gate keeps raster-only backends (wgpu D3D12,
/// AMD/Intel GPUs without ray query, Metal) from panicking with
/// "`Res<SceneGeometryReadiness>` ... Resource does not exist" on the first frame.
fn share_scene_readiness(app: &mut App) {
    let readiness = app
        .world()
        .resource::<bevy_render::scene_readiness::SceneGeometryReadiness>()
        .clone();
    app.sub_app_mut(RenderApp).insert_resource(readiness);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::SubApp;
    use bevy_asset::{AssetId, UntypedAssetId};
    use bevy_mesh::Mesh;

    /// The failing D3D12/older-GPU case: no `RenderDevice`, no raytracing
    /// features, yet every registered `Render` consumer must still find the
    /// shared readiness receipt in the render world.
    #[test]
    fn scene_readiness_reaches_render_world_without_raytracing_features() {
        let mut app = App::new();
        app.init_resource::<bevy_render::scene_readiness::SceneGeometryReadiness>();
        app.insert_sub_app(RenderApp, SubApp::new());

        // Request a geometry asset in the app world before sharing.
        let mesh = AssetId::<Mesh>::default();
        let id: UntypedAssetId = mesh.untyped();
        app.world()
            .resource::<bevy_render::scene_readiness::SceneGeometryReadiness>()
            .request(id);

        share_scene_readiness(&mut app);

        let render = app.sub_app(RenderApp).world();
        assert!(render.contains_resource::<bevy_render::scene_readiness::SceneGeometryReadiness>());
        // The clone shares the same Arc, so a render-world publish is visible to
        // the app world and the original request is visible to the render world.
        render
            .resource::<bevy_render::scene_readiness::SceneGeometryReadiness>()
            .publish(id, true);
        assert!(
            app.world()
                .resource::<bevy_render::scene_readiness::SceneGeometryReadiness>()
                .ready(id)
        );
    }
}
