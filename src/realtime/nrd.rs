//! Opaque RELAX integration. Native controllers are retained per view; Bevy owns scheduling.
use bevy_asset::{AssetServer, load_embedded_asset};
use bevy_ecs::prelude::*;
use bevy_math::{Mat4, UVec2};
use bevy_render::{
    render_resource::{binding_types::*, *},
    renderer::{RenderDevice, RenderQueue},
    view::{ExtractedView, ViewUniform},
};
use std::sync::Mutex;

pub(super) struct ViewDenoiser {
    pub diffuse: TextureView,
    pub specular: TextureView,
    size: UVec2,
    state: Mutex<State>,
}

struct State {
    context: Option<solarik_nrd_wgpu::Context>,
    projection: Mat4,
    world_to_view: Mat4,
    active: bool,
}

impl ViewDenoiser {
    pub fn new(device: &RenderDevice, size: UVec2) -> Self {
        let signal = |label| {
            device
                .create_texture(&TextureDescriptor {
                    label: Some(label),
                    size: Extent3d {
                        width: size.x,
                        height: size.y,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba16Float,
                    usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&TextureViewDescriptor::default())
        };
        Self {
            diffuse: signal("NRD diffuse lighting"),
            specular: signal("NRD specular lighting"),
            size,
            state: Mutex::new(State {
                context: None,
                projection: Mat4::IDENTITY,
                world_to_view: Mat4::IDENTITY,
                active: false,
            }),
        }
    }
}

#[derive(Resource)]
pub(super) struct NrdPipelines {
    layout: BindGroupLayoutDescriptor,
    prepare: CachedComputePipelineId,
    resolve: CachedComputePipelineId,
}

pub(super) fn init_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    assets: Res<AssetServer>,
) {
    let float = || texture_2d(TextureSampleType::Float { filterable: false });
    let output = |format| texture_storage_2d(format, StorageTextureAccess::WriteOnly);
    let layout = BindGroupLayoutDescriptor::new(
        "NRD prepare and resolve",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                float(),
                float(),
                texture_2d(TextureSampleType::Uint),
                texture_depth_2d(),
                float(),
                uniform_buffer::<ViewUniform>(true),
                output(TextureFormat::Rgba16Float),
                output(TextureFormat::Rg16Float),
                output(TextureFormat::R32Float),
                output(TextureFormat::Rgba16Float),
                output(TextureFormat::Rgba16Float),
                float(),
                float(),
                output(TextureFormat::Rgba16Float),
            ),
        ),
    );
    let pipeline = |entry: &'static str| {
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(entry.into()),
            layout: vec![layout.clone()],
            shader: load_embedded_asset!(assets.as_ref(), "nrd.wesl"),
            entry_point: Some(entry.into()),
            ..Default::default()
        })
    };
    commands.insert_resource(NrdPipelines {
        prepare: pipeline("prepare_nrd"),
        resolve: pipeline("resolve_nrd"),
        layout,
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "All bindings belong to one camera's opaque lighting dispatch"
)]
pub(super) fn denoise(
    view: &ViewDenoiser,
    enabled: bool,
    reset: bool,
    index: u32,
    camera: &ExtractedView,
    device: &RenderDevice,
    queue: &RenderQueue,
    encoder: &mut CommandEncoder,
    cache: &PipelineCache,
    pipelines: &NrdPipelines,
    gbuffer: &TextureView,
    depth: &TextureView,
    motion: &TextureView,
    uniforms: BindingResource<'_>,
    offset: u32,
    target: &WgpuTextureView,
) {
    let mut state = view.state.lock().expect("NRD view mutex poisoned");
    if !enabled {
        state.active = false;
        return;
    }
    let (Some(prepare), Some(resolve)) = (
        cache.get_compute_pipeline(pipelines.prepare),
        cache.get_compute_pipeline(pipelines.resolve),
    ) else {
        state.active = false;
        return;
    };
    let images = &state
        .context
        .get_or_insert_with(|| {
            solarik_nrd_wgpu::Context::new(device.wgpu_device(), view.size.x, view.size.y)
                .expect("Source-built NRD 4.18 RELAX must initialize")
        })
        .images;
    let mut entries = BindGroupEntries::sequential((
        &view.diffuse,
        &view.specular,
        gbuffer,
        depth,
        motion,
        uniforms,
    ))
    .to_vec();
    for (binding, image) in [(6, 1), (7, 0), (8, 2), (9, 3), (10, 4), (11, 5), (12, 6)] {
        entries.push(BindGroupEntry {
            binding,
            resource: BindingResource::TextureView(&images[image].view),
        });
    }
    entries.push(BindGroupEntry {
        binding: 13,
        resource: BindingResource::TextureView(target),
    });
    let group = device.create_bind_group(
        "NRD prepare and resolve",
        &cache.get_bind_group_layout(&pipelines.layout),
        &entries,
    );
    let size = images[0].texture.size();
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("NRD prepare"),
            ..Default::default()
        });
        pass.set_pipeline(prepare);
        pass.set_bind_group(0, &group, &[offset]);
        pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
    }
    let projection = camera.clip_from_view;
    let world_to_view = camera.world_from_view.to_matrix().inverse();
    let restart = reset || !state.active;
    let frame = solarik_nrd_wgpu::Frame {
        projection: projection.to_cols_array(),
        world_to_view: world_to_view.to_cols_array(),
        previous_projection: if restart {
            projection
        } else {
            state.projection
        }
        .to_cols_array(),
        previous_world_to_view: if restart {
            world_to_view
        } else {
            state.world_to_view
        }
        .to_cols_array(),
        index,
        reset: restart,
    };
    state
        .context
        .as_mut()
        .expect("NRD context initialized above")
        .denoise(device.wgpu_device(), queue, encoder, &frame)
        .expect("NRD RELAX dispatch failed");
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("NRD resolve"),
            ..Default::default()
        });
        pass.set_pipeline(resolve);
        pass.set_bind_group(0, &group, &[offset]);
        pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
    }
    if !state.active {
        tracing::info!(
            width = size.width,
            height = size.height,
            "NRD RELAX opaque diffuse/specular denoising active"
        );
    }
    state.projection = projection;
    state.world_to_view = world_to_view;
    state.active = true;
}
