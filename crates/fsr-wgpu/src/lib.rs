//! # solarik-fsr-wgpu
//!
//! AMD FidelityFX Super Resolution (FSR 3.x) for wgpu 30's Vulkan backend, bound
//! to the FidelityFX SDK's five-function **FfxApi** C ABI and loaded **at runtime**
//! with `libloading`. This is the AMD counterpart of the fork's
//! `vendor/dlss_wgpu`; see `docs/fsr.md` in the parent crate for integration.
//!
//! Nothing is linked against an AMD binary: `cargo build`/`cargo test` succeed
//! without an SDK, a developer account, a signed DLL or any environment variable.
//! At runtime [`Library::load`] dlopens `amd_fidelityfx_vk.dll` (Windows) or
//! `libamd_fidelityfx_vk.so` (Linux), or the SDK 2.x loader/upscaler pair, and
//! resolves `ffxCreateContext`, `ffxDestroyContext`, `ffxConfigure`, `ffxQuery`
//! and `ffxDispatch`. If no provider is present the caller simply keeps native
//! resolution, mirroring the unavailable-DLSS fallbacks in consuming applications.
//!
//! Pinned ABI: FidelityFX-SDK `ffx_api.h`, `ffx_api_types.h`, `ffx_api_loader.h`,
//! `vk/ffx_api_vk.h` and `upscalers/include/ffx_upscale.h`. The C declarations
//! these bindings mirror are recorded in `src/abi.h`; layout assertions in
//! `native.rs` and `tests/abi.rs` guard against drift.
//!
//! **Vulkan device setup is the application's responsibility.** The FFX Vulkan
//! provider needs specific device extensions/features enabled when the
//! `VkDevice` is created (the same class of requirement as
//! `dlss_wgpu::register_device_extensions`). [`Context::new`] passes wgpu's
//! `VkDevice`/`VkPhysicalDevice` to the provider; on a device created without
//! those extensions, SDK 1.1.x `ffxCreateContext` fails inside the provider.
//! Register them in the application's device-creation callback before enabling
//! FSR, and see `docs/fsr.md` in the parent crate for the measured behaviour.

pub mod effects;
#[allow(
    unsafe_code,
    reason = "The FfxApi ABI and raw Vulkan handles are isolated behind owned safe Rust values"
)]
mod native;
pub mod quality;
#[allow(
    unsafe_code,
    reason = "wgpu 30 exposes raw Vulkan handles only behind unsafe as_hal accessors"
)]
mod vulkan;

pub use quality::{
    QualityMode, dispatch_flags, effect_sub_id, flags, formats, resource_types, states,
    surface_format, usage,
};
/// Vulkan device-extension helpers for the application's device-creation
/// callback (see `docs/fsr.md`). Re-exported from the private `vulkan` module.
pub use vulkan::{
    device_extensions::{DEVICE_EXTENSION_CSTRS, DEVICE_EXTENSIONS, DEVICE_FEATURES},
    register_device_extensions, register_device_extensions_for_adapter,
};

use std::sync::Arc;

/// Failures detected before submitting FSR work.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No FidelityFX provider module could be loaded.
    #[error(
        "AMD FidelityFX FSR runtime not found (set FSR_DLL / FSR_RUNTIME_DIR / FSR_NATIVE_LIBS): {0}"
    )]
    Library(String),
    /// A module loaded but was missing one of the five FfxApi exports.
    #[error("FidelityFX provider is missing symbol {0}: {1}")]
    Symbol(&'static str, String),
    /// `ffxReturnCode_t` other than `FFX_API_RETURN_OK`.
    #[error("FfxApi returned status {0}")]
    Status(u32),
    /// wgpu is not using the Vulkan backend (Metal/DX12 providers are not bound).
    #[error("FSR requires wgpu's Vulkan backend; no VkDevice/VkImage handles available")]
    NoVulkan,
    /// Zero or oversized dimensions were requested.
    #[error("FSR requires nonzero dimensions no larger than 65535")]
    Dimensions,
}

impl From<native::LoadError> for Error {
    fn from(error: native::LoadError) -> Self {
        match error {
            native::LoadError::NotFound(paths) => Self::Library(paths.join("; ")),
            native::LoadError::Symbol(name, message) => {
                Self::Symbol(name.trim_end_matches('\0'), message)
            }
        }
    }
}

/// A loaded FidelityFX provider module. Create one per application (or per
/// render device) and share it through [`Arc`](std::sync::Arc); it is cheap to
/// clone and owns the dynamic library handle.
#[derive(Clone)]
pub struct Library {
    module: Arc<native::Module>,
}

impl Library {
    /// Load the FSR provider from the first available candidate path.
    pub fn load() -> Result<Self, Error> {
        Ok(Self {
            module: Arc::new(native::Module::load()?),
        })
    }

    /// Whether an FSR provider is available on this machine. Useful for a
    /// renderer's capability resource (the DLSS pattern).
    pub fn is_available() -> bool {
        Self::load().is_ok()
    }
}

// Note: the FfxApi `ffxQuery` export is resolved as part of the pinned five-function
// ABI, but upscaler queries are only issued on a live [`Context`]. SDK 1.1.x
// providers dereference the context handle for these descriptors, so a
// null-context (global) query is invalid usage and must never be exposed here.
// [`QualityMode`] supplies the documented ratios, jitter and mip bias instead.

fn status_ok(status: u32) -> Result<(), Error> {
    if status == 0 {
        Ok(())
    } else {
        Err(Error::Status(status))
    }
}

/// `FFX_API_CREATE_CONTEXT_DESC_TYPE_BACKEND_VK` differs between SDK generations
/// while the struct layout is identical. Retry all known values; `FSR_BACKEND_VK_TYPE`
/// (hex, e.g. `0x02000003`) pins one for a specific provider build.
fn backend_types() -> Vec<u64> {
    if let Ok(value) = std::env::var("FSR_BACKEND_VK_TYPE") {
        let trimmed = value.trim_start_matches("0x");
        if let Ok(kind) = u64::from_str_radix(trimmed, 16) {
            return vec![kind];
        }
        tracing::warn!(value = %value, "Ignoring malformed FSR_BACKEND_VK_TYPE");
    }
    native::BACKEND_VK_TYPES.to_vec()
}

/// One camera's FSR upscaler context. Controllers and the chained create/backend
/// descriptors are retained for the context's lifetime; a resize or quality
/// change requires a new context, exactly like DLSS Super Resolution.
pub struct Context {
    library: Arc<native::Module>,
    handle: native::Handle,
    _descriptors: Box<native::PinnedDescriptors>,
    max_render_size: [u32; 2],
    max_upscale_size: [u32; 2],
}

impl Context {
    /// Create an FSR context for the wgpu Vulkan device.
    ///
    /// `flags` is a combination of [`flags`] (`HIGH_DYNAMIC_RANGE` for linear HDR
    /// render targets). `max_render_size`/`max_upscale_size` bound the resolutions the
    /// context will ever see; per-frame sizes are supplied to [`Context::dispatch`].
    pub fn new(
        library: &Library,
        device: &wgpu::Device,
        max_render_size: [u32; 2],
        max_upscale_size: [u32; 2],
        flags: u32,
    ) -> Result<Self, Error> {
        for size in [max_render_size, max_upscale_size] {
            if size[0] == 0
                || size[1] == 0
                || size[0] > u16::MAX as u32
                || size[1] > u16::MAX as u32
            {
                return Err(Error::Dimensions);
            }
        }
        let device_context = vulkan::device_context(device).ok_or(Error::NoVulkan)?;
        let create = native::CreateContextDescUpscale {
            header: native::FfxApiHeader::new(native::CREATE_CONTEXT_DESC_UPSCALE),
            flags,
            max_render_size: native::Dimensions2D {
                width: max_render_size[0],
                height: max_render_size[1],
            },
            max_upscale_size: native::Dimensions2D {
                width: max_upscale_size[0],
                height: max_upscale_size[1],
            },
            message: std::ptr::null_mut(),
        };
        let backend = native::CreateBackendVkDesc {
            header: native::FfxApiHeader::new(native::BACKEND_VK_TYPES[0]),
            device_context,
        };
        let mut descriptors = native::pin_descriptors(create, backend);
        let mut handle: native::Context = std::ptr::null_mut();
        let mut status = 0u32;
        for kind in backend_types() {
            // The descriptor chain is pinned on the heap; only the backend
            // discriminator changes between attempts across SDK generations.
            descriptors.backend.header.kind = kind;
            let header = std::ptr::from_mut(&mut descriptors.create.header);
            status = library.module.create_context(&mut handle, header);
            if status == 0 {
                break;
            }
            // Descriptor/provider mismatches are expected across SDK generations.
            if !matches!(status, 1 | 2 | 4 | 6) {
                return Err(Error::Status(status));
            }
            handle = std::ptr::null_mut();
        }
        if handle.is_null() {
            return Err(Error::Status(status));
        }
        Ok(Self {
            library: Arc::clone(&library.module),
            handle: native::Handle(handle),
            _descriptors: descriptors,
            max_render_size,
            max_upscale_size,
        })
    }

    /// The maximum render resolution this context was created for.
    pub fn render_size(&self) -> [u32; 2] {
        self.max_render_size
    }

    /// The maximum upscaled (presentation) resolution this context was created for.
    pub fn upscale_size(&self) -> [u32; 2] {
        self.max_upscale_size
    }

    /// Issue an upscaler `ffxQuery` against this live context. Null-context
    /// queries are deliberately not offered: SDK 1.1.x providers dereference the
    /// context handle for these descriptor types.
    fn query(&self, header: *mut native::FfxApiHeader) -> Result<(), Error> {
        let mut context = self.handle.0;
        status_ok(self.library.query(&mut context, header))
    }

    /// Provider-computed per-dimension upscaling ratio for a quality preset.
    pub fn query_upscale_ratio(&self, quality: QualityMode) -> Result<f32, Error> {
        let mut ratio = 0.0f32;
        let mut desc = native::QueryDescUpscaleRatio {
            header: native::FfxApiHeader::new(native::QUERY_DESC_UPSCALE_RATIO),
            quality_mode: quality.as_raw(),
            out_ratio: &mut ratio,
        };
        self.query(&mut desc.header)?;
        Ok(ratio)
    }

    /// Provider-computed render resolution for a display size and quality preset.
    pub fn query_render_resolution(
        &self,
        quality: QualityMode,
        display: [u32; 2],
    ) -> Result<[u32; 2], Error> {
        let mut width = 0u32;
        let mut height = 0u32;
        let mut desc = native::QueryDescRenderResolution {
            header: native::FfxApiHeader::new(native::QUERY_DESC_RENDER_RESOLUTION),
            display_width: display[0],
            display_height: display[1],
            quality_mode: quality.as_raw(),
            out_render_width: &mut width,
            out_render_height: &mut height,
        };
        self.query(&mut desc.header)?;
        Ok([width, height])
    }

    /// Provider-computed jitter phase count for a render/display width pair.
    pub fn query_jitter_phase_count(
        &self,
        render_width: u32,
        display_width: u32,
    ) -> Result<u32, Error> {
        let mut count = 0i32;
        let mut desc = native::QueryDescJitterPhaseCount {
            header: native::FfxApiHeader::new(native::QUERY_DESC_JITTER_PHASE_COUNT),
            render_width,
            display_width,
            out_phase_count: &mut count,
        };
        self.query(&mut desc.header)?;
        Ok(count.max(1) as u32)
    }

    /// Provider-computed subpixel jitter for a phase index.
    pub fn query_jitter_offset(&self, index: u32, phase_count: u32) -> Result<[f32; 2], Error> {
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut desc = native::QueryDescJitterOffset {
            header: native::FfxApiHeader::new(native::QUERY_DESC_JITTER_OFFSET),
            index: index as i32,
            phase_count: phase_count as i32,
            out_x: &mut x,
            out_y: &mut y,
        };
        self.query(&mut desc.header)?;
        Ok([x, y])
    }

    /// Build a context around an in-process stub provider for unit tests. No GPU
    /// device or DLL is involved; `stub_destroy` accepts the sentinel handle.
    #[cfg(test)]
    pub(crate) fn stub(module: Arc<native::Module>) -> Self {
        Self {
            library: module,
            handle: native::Handle(std::ptr::NonNull::<u8>::dangling().as_ptr().cast()),
            _descriptors: native::stub_descriptors(),
            max_render_size: [1920, 1080],
            max_upscale_size: [1920, 1080],
        }
    }

    /// Record FSR's upscaling passes into `params.encoder`.
    pub fn dispatch(&mut self, params: Dispatch<'_>) -> Result<(), Error> {
        let empty = native::Resource::default();
        let mut desc = native::DispatchDescUpscale {
            header: native::FfxApiHeader::new(native::DISPATCH_DESC_UPSCALE),
            command_list: std::ptr::null_mut(),
            color: empty,
            depth: empty,
            motion_vectors: empty,
            exposure: empty,
            reactive: empty,
            transparency_and_composition: empty,
            output: empty,
            jitter_offset: native::FloatCoords2D {
                x: params.jitter[0],
                y: params.jitter[1],
            },
            motion_vector_scale: native::FloatCoords2D {
                x: params.motion_vector_scale[0],
                y: params.motion_vector_scale[1],
            },
            render_size: native::Dimensions2D {
                width: params.render_size[0],
                height: params.render_size[1],
            },
            upscale_size: native::Dimensions2D {
                width: params.upscale_size[0],
                height: params.upscale_size[1],
            },
            enable_sharpening: params.sharpness > 0.0,
            sharpness: params.sharpness.clamp(0.0, 1.0),
            frame_time_delta: params.frame_time_delta_ms,
            pre_exposure: params.pre_exposure.max(f32::EPSILON),
            reset: params.reset,
            camera_near: params.camera_near,
            camera_far: params.camera_far,
            camera_fov_angle_vertical: params.camera_fov_angle_vertical,
            view_space_to_meters_factor: params.view_space_to_meters_factor,
            flags: params.flags,
        };
        desc.color = resource(params.color, states::COMPUTE_READ)?;
        desc.depth = resource(params.depth, states::COMPUTE_READ)?;
        desc.motion_vectors = resource(params.motion_vectors, states::COMPUTE_READ)?;
        desc.exposure = optional_resource(params.exposure, states::COMPUTE_READ)?;
        desc.reactive = optional_resource(params.reactive, states::COMPUTE_READ)?;
        desc.transparency_and_composition =
            optional_resource(params.transparency_and_composition, states::COMPUTE_READ)?;
        desc.output = resource(params.output, states::UNORDERED_ACCESS)?;

        let library = Arc::clone(&self.library);
        let mut handle = self.handle;
        let result = vulkan::with_command_buffer(params.encoder, |command_list| {
            desc.command_list = command_list;
            let status =
                library.dispatch(&mut handle.0, std::ptr::from_ref(&desc.header).cast_mut());
            status_ok(status)
        });
        result.unwrap_or(Err(Error::NoVulkan))
    }
}

fn resource(view: &wgpu::TextureView, state: u32) -> Result<native::Resource, Error> {
    vulkan::resource(view, state).ok_or(Error::NoVulkan)
}

fn optional_resource(
    view: Option<&wgpu::TextureView>,
    state: u32,
) -> Result<native::Resource, Error> {
    match view {
        Some(view) => resource(view, state),
        None => Ok(native::Resource::default()),
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        let mut handle = self.handle.0;
        let status = self.library.destroy_context(&mut handle);
        if status != 0 {
            tracing::warn!(status, "ffxDestroyContext failed");
        }
    }
}

/// Per-frame inputs for [`Context::dispatch`].
pub struct Dispatch<'a> {
    /// Resolved HDR color at render resolution.
    pub color: &'a wgpu::TextureView,
    /// Depth at render resolution.
    pub depth: &'a wgpu::TextureView,
    /// Motion vectors at render resolution.
    pub motion_vectors: &'a wgpu::TextureView,
    /// Output color at presentation resolution.
    pub output: &'a wgpu::TextureView,
    /// Optional 1x1 exposure texture.
    pub exposure: Option<&'a wgpu::TextureView>,
    /// Optional reactive mask.
    pub reactive: Option<&'a wgpu::TextureView>,
    /// Optional transparency/composition mask.
    pub transparency_and_composition: Option<&'a wgpu::TextureView>,
    /// Subpixel jitter in `[-0.5, 0.5]` pixel units.
    pub jitter: [f32; 2],
    /// Converts motion vectors to pixel space (render size for UV-space vectors).
    pub motion_vector_scale: [f32; 2],
    /// Resolution of the input resources.
    pub render_size: [u32; 2],
    /// Resolution of the output resource.
    pub upscale_size: [u32; 2],
    /// `0..1`; zero disables the extra sharpening pass.
    pub sharpness: f32,
    /// Frame time in milliseconds.
    pub frame_time_delta_ms: f32,
    /// Pre-exposure (must be > 0).
    pub pre_exposure: f32,
    /// Drop temporal history after a camera cut or resolution change.
    pub reset: bool,
    /// Camera near plane.
    pub camera_near: f32,
    /// Camera far plane.
    pub camera_far: f32,
    /// Vertical field of view in radians.
    pub camera_fov_angle_vertical: f32,
    /// View-space-to-meters scale.
    pub view_space_to_meters_factor: f32,
    /// Combination of [`dispatch_flags`].
    pub flags: u32,
    /// The wgpu encoder FSR records into.
    pub encoder: &'a mut wgpu::CommandEncoder,
}
