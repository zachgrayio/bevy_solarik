//! Native NRD 4.18 RELAX controller and Vulkan compute dispatches for wgpu 30.
//! SDK source and shader binaries are supplied by the application build, not redistributed here.
use std::{collections::HashMap, num::NonZeroU64};

#[allow(
    unsafe_code,
    reason = "The SDK ABI and trusted SPIR-V boundary are isolated behind owned safe Rust values"
)]
mod native;

/// Failures detected before submitting NRD work.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("NRD SDK version/encoding or native instance creation failed")]
    Initialization,
    #[error("NRD returned status {0}")]
    Native(u32),
    #[error("Unsupported NRD texture format {0}")]
    Format(u32),
    #[error("Unsupported NRD input resource {0}")]
    Resource(u32),
    #[error("NRD requires nonzero dimensions no larger than 65535")]
    Dimensions,
}

/// Non-jittered camera state. Matrices are column-major; motion is supplied in Bevy's current-minus-previous UV convention.
pub struct Frame {
    pub projection: [f32; 16],
    pub previous_projection: [f32; 16],
    pub world_to_view: [f32; 16],
    pub previous_world_to_view: [f32; 16],
    pub index: u32,
    pub reset: bool,
}

/// A persistent NRD image. The view can be bound directly by the application's prepare/resolve shaders.
pub struct Image {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub format: wgpu::TextureFormat,
}

impl Image {
    fn new(
        device: &wgpu::Device,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            format,
        }
    }
}

/// One camera's native controller, shader cache and bounded permanent/transient image pools.
pub struct Context {
    native: native::Instance,
    pub images: [Image; 7],
    permanent: Vec<Image>,
    transient: Vec<Image>,
    pipelines: HashMap<(u32, Vec<wgpu::TextureFormat>), wgpu::ComputePipeline>,
    constants_layout: wgpu::BindGroupLayout,
    constants: wgpu::Buffer,
    samplers: [wgpu::Sampler; 2],
    stride: u64,
    width: u32,
    height: u32,
}

impl Context {
    /// Texture slots: motion, encoded normal/roughness, positive view-Z, diffuse input,
    /// specular input, diffuse output, specular output. Radiance inputs are demodulated, linear HDR;
    /// alpha is unnormalized first-bounce hit distance, as required by RELAX.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > 65535 || height > 65535 {
            return Err(Error::Dimensions);
        }
        let native = native::Instance::new()?;
        let make_pool = |permanent| -> Result<Vec<Image>, Error> {
            native
                .pool(permanent)
                .into_iter()
                .map(|(format, downsample)| {
                    let downsample = downsample.max(1);
                    Ok(Image::new(
                        device,
                        "NRD pool",
                        width.div_ceil(downsample),
                        height.div_ceil(downsample),
                        texture_format(format)?,
                    ))
                })
                .collect()
        };
        let permanent = make_pool(true)?;
        let transient = make_pool(false)?;
        let formats = [
            wgpu::TextureFormat::Rg16Float,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::R32Float,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Rgba16Float,
        ];
        let images = std::array::from_fn(|i| {
            Image::new(device, "NRD guide/signal", width, height, formats[i])
        });
        let constants_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("NRD constants and samplers"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let stride = (u64::from(native.constant_bytes()) + 255) & !255;
        let constants = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("NRD per-dispatch constants"),
            size: stride * 128,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let samplers = [wgpu::FilterMode::Nearest, wgpu::FilterMode::Linear].map(|filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("NRD clamp sampler"),
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        });
        Ok(Self {
            native,
            images,
            permanent,
            transient,
            pipelines: HashMap::new(),
            constants_layout,
            constants,
            samplers,
            stride,
            width,
            height,
        })
    }

    fn resource(&self, kind: u32, index: usize) -> Result<&Image, Error> {
        match kind {
            0..=6 => self.images.get(kind as usize),
            7 => self.transient.get(index),
            8 => self.permanent.get(index),
            _ => None,
        }
        .ok_or(Error::Resource(kind))
    }

    /// Encode RELAX after lighting guides/signals have been written and before the application's resolve.
    /// Controllers and image pools are retained; resizing requires a new context.
    pub fn denoise(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
    ) -> Result<(), Error> {
        let dispatches = self.native.update(self.width, self.height, frame)?;
        if dispatches.len() > 128 {
            return Err(Error::Native(u32::MAX));
        }
        let mut constants = vec![0u8; self.stride as usize * dispatches.len()];
        for (i, dispatch) in dispatches.iter().enumerate() {
            let offset = i * self.stride as usize;
            constants[offset..offset + dispatch.constants.len()]
                .copy_from_slice(&dispatch.constants);
        }
        queue.write_buffer(&self.constants, 0, &constants);
        for (i, dispatch) in dispatches.iter().enumerate() {
            let formats: Vec<_> = dispatch
                .resources
                .iter()
                .map(|r| self.resource(r.kind, r.index as usize).map(|t| t.format))
                .collect::<Result<_, _>>()?;
            let key = (dispatch.pipeline, formats);
            if !self.pipelines.contains_key(&key) {
                let info = self.native.pipeline(dispatch.pipeline);
                let mut textures = 0;
                let mut storage = 0;
                let entries: Vec<_> = dispatch
                    .resources
                    .iter()
                    .map(|r| {
                        let image = self.resource(r.kind, r.index as usize)?;
                        let (binding, ty) = if r.storage {
                            let binding = storage;
                            storage += 1;
                            (
                                binding,
                                wgpu::BindingType::StorageTexture {
                                    access: wgpu::StorageTextureAccess::WriteOnly,
                                    format: image.format,
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                },
                            )
                        } else {
                            let binding = info.storage_textures + textures;
                            textures += 1;
                            let sample_type = image
                                .format
                                .sample_type(None, Some(device.features()))
                                .ok_or(Error::Format(u32::MAX))?;
                            (
                                binding,
                                wgpu::BindingType::Texture {
                                    sample_type,
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                            )
                        };
                        Ok(wgpu::BindGroupLayoutEntry {
                            binding,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty,
                            count: None,
                        })
                    })
                    .collect::<Result<_, Error>>()?;
                if info.common_space != 1
                    || info.resource_space != 0
                    || textures != info.textures
                    || storage != info.storage_textures
                {
                    return Err(Error::Initialization);
                }
                let resources_layout =
                    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("NRD resources"),
                        entries: &entries,
                    });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("NRD RELAX"),
                    bind_group_layouts: &[Some(&resources_layout), Some(&self.constants_layout)],
                    immediate_size: 0,
                });
                let module = native::shader(device, &info.spirv, info.storage_textures);
                let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("NRD RELAX"),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });
                self.pipelines.insert(key.clone(), pipeline);
            }
            let pipeline = &self.pipelines[&key];
            let mut textures = 0;
            let mut storage = 0;
            let entries: Vec<_> = dispatch
                .resources
                .iter()
                .map(|r| {
                    let binding = if r.storage {
                        let b = storage;
                        storage += 1;
                        b
                    } else {
                        let b = dispatch.resources.iter().filter(|r| r.storage).count() as u32
                            + textures;
                        textures += 1;
                        b
                    };
                    Ok(wgpu::BindGroupEntry {
                        binding,
                        resource: wgpu::BindingResource::TextureView(
                            &self.resource(r.kind, r.index as usize)?.view,
                        ),
                    })
                })
                .collect::<Result<_, Error>>()?;
            let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("NRD resources"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            let common = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("NRD constants"),
                layout: &self.constants_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Sampler(&self.samplers[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.samplers[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.constants,
                            offset: i as u64 * self.stride,
                            size: NonZeroU64::new(self.stride),
                        }),
                    },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("NRD RELAX"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.set_bind_group(1, &common, &[]);
            pass.dispatch_workgroups(dispatch.width, dispatch.height, 1);
        }
        Ok(())
    }
}

fn texture_format(format: u32) -> Result<wgpu::TextureFormat, Error> {
    use wgpu::TextureFormat as F;
    Ok(match format {
        0 => F::R8Unorm,
        1 => F::R8Uint,
        2 => F::Rg8Unorm,
        3 => F::R16Float,
        4 => F::Rg16Float,
        5 => F::Rgba16Float,
        6 => F::R32Float,
        7 => F::Rg32Float,
        8 => F::Rgba32Float,
        9 => F::R16Uint,
        10 => F::Rg16Uint,
        11 => F::Rgba16Uint,
        12 => F::R32Uint,
        13 => F::Rg32Uint,
        14 => F::Rgba32Uint,
        15 => F::Rg8Uint,
        16 => F::Rgba8Unorm,
        17 => F::R16Unorm,
        18 => F::Rg16Unorm,
        19 => F::Rgba16Unorm,
        20 => F::Rgb10a2Unorm,
        _ => return Err(Error::Format(format)),
    })
}
