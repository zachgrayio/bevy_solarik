//! Additional `FidelityFX` SDK **2.4.0** effect descriptors and constants,
//! beyond the temporal upscaler in [`crate::native`].
//!
//! The 2.4.0 signed binaries split the SDK into a loader plus per-effect DLLs
//! and add three effect families used by the hybrid renderer plan:
//!
//! * **Denoiser** (`0x0005_0000`) — FSR Ray Regeneration on DX12.
//! * **Radiance Cache** (`0x0006_0000`) — neural radiance caching on DX12.
//! * **Frame Generation** (`0x0002_0000`) — FSR FG on Vulkan and DX12.
//!
//! These are `#[repr(C)]` mirrors of the AMD MIT headers plus their SDK version
//! and descriptor-type constants. Every layout is asserted at compile time and
//! cross-checked against the MSVC-compiled C declarations in `src/abi.h`.
//! Nothing here links against an AMD binary; the runtime loader resolves the
//! same five `FfxApi` exports with `libloading`.

use crate::native::{Dimensions2D, FfxApiHeader, FloatCoords2D, Resource, StructType};
use std::ffi::{c_char, c_void};

/// Compose an SDK version from major/minor/patch, mirroring the
/// `FFX_*_MAKE_VERSION` macros (`major << 22 | minor << 12 | patch`).
pub const fn make_version(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 22) | (minor << 12) | patch
}

/// Compose a backend descriptor type from a backend id and sub-version,
/// mirroring `FFX_API_MAKE_BACKEND_SUB_ID`.
pub const fn backend_sub_id(backend: u32, sub: u32) -> StructType {
    const BACKEND_MASK: u64 = 0xff00_0000;
    ((backend as u64) & BACKEND_MASK) | ((sub as u64) & !BACKEND_MASK)
}

/// `FFX_API_EFFECT_ID_*` values from `ffx_api.h` (SDK 2.4.0).
pub mod effect_ids {
    /// Upscaling (the temporal upscaler).
    pub const UPSCALE: u32 = 0x0001_0000;
    /// Frame generation.
    pub const FRAMEGENERATION: u32 = 0x0002_0000;
    /// Frame-generation swapchain (DX12).
    pub const FRAMEGENERATIONSWAPCHAIN: u32 = 0x0003_0000;
    /// Frame-generation swapchain (Vulkan).
    pub const FRAMEGENERATIONSWAPCHAIN_VK: u32 = 0x0004_0000;
    /// Denoiser / FSR Ray Regeneration.
    pub const DENOISER: u32 = 0x0005_0000;
    /// Radiance cache.
    pub const RADIANCECACHE: u32 = 0x0006_0000;
}

/// `FFX_API_BACKEND_ID_*` values from `ffx_api.h` (SDK 2.4.0).
pub mod backend_ids {
    /// DirectX 12 backend.
    pub const DX12: u32 = 0x0000_0000;
    /// Vulkan backend.
    pub const VK: u32 = 0x0100_0000;
    /// Xbox backend.
    pub const XBOX: u32 = 0x0200_0000;
}

/// Pinned SDK effect versions (SDK 2.4.0).
pub mod versions {
    use super::make_version;
    /// `FFX_UPSCALER_VERSION` — FSR Upscaling 4.1.2.
    pub const UPSCALER: u32 = make_version(4, 1, 2);
    /// `FFX_DENOISER_VERSION` — FSR Ray Regeneration 1.2.1.
    pub const DENOISER: u32 = make_version(1, 2, 1);
    /// `FFX_RADIANCECACHE_VERSION` — 0.9.0 technical preview.
    pub const RADIANCECACHE: u32 = make_version(0, 9, 0);
    /// `FFX_FRAMEGENERATION_VERSION` — 4.0.2.
    pub const FRAMEGENERATION: u32 = make_version(4, 0, 2);
}

/// Descriptor-type discriminators. `effect_sub_id` comes from
/// [`crate::quality`]; backend types use [`backend_sub_id`].
pub mod desc_types {
    use super::{backend_ids, backend_sub_id, effect_ids};
    use crate::quality::effect_sub_id;
    /// `ffxCreateContextDescUpscaleVersion` (`0x0001000b`).
    pub const UPSCALE_VERSION: u64 = effect_sub_id(effect_ids::UPSCALE, 0x0b);
    /// `ffxCreateBackendVKDesc` (`0x01000003` on SDK 2.4.0).
    pub const BACKEND_VK: u64 = backend_sub_id(backend_ids::VK, 0x03);
    /// `ffxCreateBackendDX12Desc` (`0x00000002`).
    pub const BACKEND_DX12: u64 = backend_sub_id(backend_ids::DX12, 0x02);
    /// `ffxCreateContextDescDenoiser` (`0x00050001`).
    pub const DENOISER_CREATE: u64 = effect_sub_id(effect_ids::DENOISER, 0x01);
    /// `ffxQueryDescDenoiserGetGpuMemoryUsage` (`0x00050007`).
    pub const DENOISER_QUERY_GPU_MEMORY_USAGE: u64 = effect_sub_id(effect_ids::DENOISER, 0x07);
    /// `ffxConfigureDescDenoiserKeyValue` (`0x00050021`).
    pub const DENOISER_CONFIGURE_KEYVALUE: u64 = effect_sub_id(effect_ids::DENOISER, 0x21);
    /// `ffxDispatchDescDenoiser` (`0x00050041`).
    pub const DENOISER_DISPATCH: u64 = effect_sub_id(effect_ids::DENOISER, 0x41);
    /// `ffxCreateContextDescRadianceCache` (`0x00060002`).
    pub const RADIANCECACHE_CREATE: u64 = effect_sub_id(effect_ids::RADIANCECACHE, 0x02);
    /// `ffxDispatchDescRadianceCache` (`0x00060004`).
    pub const RADIANCECACHE_DISPATCH: u64 = effect_sub_id(effect_ids::RADIANCECACHE, 0x04);
    /// `ffxCreateContextDescFrameGeneration` (`0x00020001`).
    pub const FRAMEGENERATION_CREATE: u64 = effect_sub_id(effect_ids::FRAMEGENERATION, 0x01);
    /// `ffxDispatchDescFrameGeneration` (`0x00020003`).
    pub const FRAMEGENERATION_DISPATCH: u64 = effect_sub_id(effect_ids::FRAMEGENERATION, 0x03);
    /// `ffxCreateContextDescFrameGenerationVersion` (`0x0002000e`).
    pub const FRAMEGENERATION_VERSION: u64 = effect_sub_id(effect_ids::FRAMEGENERATION, 0x0e);
    /// `ffxQueryDescGetVersions` (general, `4`).
    pub const GET_VERSIONS: u64 = 4;
    /// `ffxOverrideVersion` (general, `5`).
    pub const OVERRIDE_VERSION: u64 = 5;
}

/// `FfxApiCreateContextRadianceCacheFlags` and dispatch flags.
pub mod radiance_cache_flags {
    /// Try to force the WMMA backend (`1 << 0`).
    pub const TRY_FORCE_WMMA: u32 = 1 << 0;
    /// Dispatch the inference pass (before training).
    pub const DISPATCH_INFERENCE: u32 = 1 << 0;
    /// Dispatch the training pass (after inference).
    pub const DISPATCH_TRAINING: u32 = 1 << 1;
    /// Clear inference counters after inference/training.
    pub const CLEAR_INFERENCE_COUNTER: u32 = 1 << 2;
    /// Clear training counters after inference/training.
    pub const CLEAR_TRAINING_COUNTER: u32 = 1 << 3;
    /// Reset the cache to default start-up parameters.
    pub const RESET: u32 = 1 << 4;
    /// Override the learning rate.
    pub const OVERRIDE_LEARNING_RATE: u32 = 1 << 5;
    /// Override weight smoothing.
    pub const OVERRIDE_WEIGHT_SMOOTHING: u32 = 1 << 6;
}

/// `FfxApiFloatCoords3D`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FloatCoords3D {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
}

/// `FfxApiFloatBounds` — inclusive `[min, max]`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FloatBounds {
    /// Lower bound.
    pub min: f32,
    /// Upper bound.
    pub max: f32,
}

/// `FfxApiFloat4`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Float4 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
    /// W component.
    pub w: f32,
}

/// `FfxApiMatrix4x4` — row-major, row-vector convention.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Matrix4x4 {
    /// Four rows.
    pub rows: [Float4; 4],
}

/// `FfxApiRect2D`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Rect2D {
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

/// `FfxApiDenoiserSignal` — one input/output signal pair.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DenoiserSignal {
    /// Noisy input signal.
    pub input: Resource,
    /// Denoised output signal.
    pub output: Resource,
    /// Checkerboard origin for the traced half (`frameIndex & 1`).
    pub checkerboard_origin: u32,
}

/// `ffxCreateContextDescUpscaleVersion`, chained on the upscaler create desc.
#[repr(C)]
pub struct CreateContextDescUpscaleVersion {
    /// Descriptor header (`desc_types::UPSCALE_VERSION`).
    pub header: FfxApiHeader,
    /// Must equal [`versions::UPSCALER`].
    pub version: u32,
}

/// `ffxCreateBackendDX12Desc`, chained on any effect create desc on DX12.
#[repr(C)]
pub struct CreateBackendDx12Desc {
    /// Descriptor header (`desc_types::BACKEND_DX12`).
    pub header: FfxApiHeader,
    /// `ID3D12Device*`.
    pub device: *mut c_void,
}

/// `ffxCreateContextDescDenoiser`.
#[repr(C)]
pub struct CreateContextDescDenoiser {
    /// Descriptor header (`desc_types::DENOISER_CREATE`).
    pub header: FfxApiHeader,
    /// Must equal [`versions::DENOISER`].
    pub version: u32,
    /// Maximum render resolution.
    pub max_render_size: Dimensions2D,
    /// Enabled denoiser signals.
    pub signal_flags: u32,
    /// Checkerboard-traced signals.
    pub checkerboard_signal_flags: u32,
    /// Creation flags.
    pub flags: u32,
}

/// `ffxDispatchDescDenoiser`.
#[repr(C)]
pub struct DispatchDescDenoiser {
    /// Descriptor header (`desc_types::DENOISER_DISPATCH`).
    pub header: FfxApiHeader,
    /// `VkCommandBuffer` / `ID3D12GraphicsCommandList` cast to `void*`.
    pub command_list: *mut c_void,
    /// Linear depth.
    pub linear_depth: Resource,
    /// Motion vectors.
    pub motion_vectors: Resource,
    /// Octahedral-encoded normals.
    pub normals: Resource,
    /// Specular albedo.
    pub specular_albedo: Resource,
    /// Diffuse albedo.
    pub diffuse_albedo: Resource,
    /// Motion-vector scale to pixel space.
    pub motion_vector_scale: FloatCoords3D,
    /// Subpixel jitter offsets.
    pub jitter_offsets: FloatCoords2D,
    /// Camera position delta since the previous frame.
    pub camera_position_delta: FloatCoords3D,
    /// View matrix.
    pub view: Matrix4x4,
    /// Projection matrix.
    pub projection: Matrix4x4,
    /// Min/max linear depth bounds.
    pub linear_depth_bounds: FloatBounds,
    /// Render resolution.
    pub render_size: Dimensions2D,
    /// Monotonic frame index.
    pub frame_index: u32,
    /// Dispatch flags.
    pub flags: u32,
}

/// `ffxCreateContextDescRadianceCache`.
#[repr(C)]
pub struct CreateContextDescRadianceCache {
    /// Descriptor header (`desc_types::RADIANCECACHE_CREATE`).
    pub header: FfxApiHeader,
    /// Creation flags (see [`radiance_cache_flags`]).
    pub flags: u32,
    /// Must equal [`versions::RADIANCECACHE`].
    pub version: u32,
    /// Max inference samples (usually viewport resolution).
    pub max_inference_sample_count: u32,
    /// Max training samples.
    pub max_training_sample_count: u32,
}

/// The `overrides` sub-struct of `ffxDispatchDescRadianceCache`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RadianceCacheOverrides {
    /// Learning-rate override.
    pub learning_rate: f32,
    /// Weight-smoothing override.
    pub weight_smoothing: f32,
}

/// `ffxDispatchDescRadianceCache`.
#[repr(C)]
pub struct DispatchDescRadianceCache {
    /// Descriptor header (`desc_types::RADIANCECACHE_DISPATCH`).
    pub header: FfxApiHeader,
    /// Open command list cast to `void*`.
    pub command_list: *mut c_void,
    /// Prediction input buffer.
    pub prediction_inputs: Resource,
    /// Prediction output buffer.
    pub prediction_outputs: Resource,
    /// Training input buffer.
    pub train_inputs: Resource,
    /// Training target buffer.
    pub train_targets: Resource,
    /// Sample counters buffer.
    pub sample_counters: Resource,
    /// Dispatch flags (see [`radiance_cache_flags`]).
    pub flags: u32,
    /// Optional overrides.
    pub overrides: RadianceCacheOverrides,
}

/// `ffxCreateContextDescFrameGeneration`.
#[repr(C)]
pub struct CreateContextDescFrameGeneration {
    /// Descriptor header (`desc_types::FRAMEGENERATION_CREATE`).
    pub header: FfxApiHeader,
    /// Creation flags.
    pub flags: u32,
    /// Display resolution.
    pub display_size: Dimensions2D,
    /// Maximum render resolution.
    pub max_render_size: Dimensions2D,
    /// Backbuffer surface format.
    pub backbuffer_format: u32,
}

/// `ffxDispatchDescFrameGeneration`.
#[repr(C)]
pub struct DispatchDescFrameGeneration {
    /// Descriptor header (`desc_types::FRAMEGENERATION_DISPATCH`).
    pub header: FfxApiHeader,
    /// Command list cast to `void*`.
    pub command_list: *mut c_void,
    /// Current presentation color buffer.
    pub present_color: Resource,
    /// Destinations for generated frames.
    pub outputs: [Resource; 4],
    /// Number of frames to generate.
    pub num_generated_frames: u32,
    /// Camera cut / discontinuous motion reset.
    pub reset: bool,
    /// Backbuffer transfer function.
    pub backbuffer_transfer_function: u32,
    /// Min/max luminance in nits.
    pub min_max_luminance: [f32; 2],
    /// Active generation rectangle.
    pub generation_rect: Rect2D,
    /// Monotonic frame identifier.
    pub frame_id: u64,
}

/// `ffxQueryDescGetVersions` — enumerate provider versions for an effect.
#[repr(C)]
pub struct QueryDescGetVersions {
    /// Descriptor header (`desc_types::GET_VERSIONS`).
    pub header: FfxApiHeader,
    /// Create-descriptor type of the effect to enumerate.
    pub create_desc_type: u64,
    /// `ID3D12Device*` / `VkDeviceContext*`.
    pub device: *mut c_void,
    /// In: array capacity; out: number of versions.
    pub output_count: *mut u64,
    /// Optional out array of version ids.
    pub version_ids: *mut u64,
    /// Optional out array of version names.
    pub version_names: *mut *const c_char,
}

/// `ffxOverrideVersion` — force a provider version from a query.
#[repr(C)]
pub struct OverrideVersion {
    /// Descriptor header (`desc_types::OVERRIDE_VERSION`).
    pub header: FfxApiHeader,
    /// Version id returned by [`QueryDescGetVersions`].
    pub version_id: u64,
}

// Pinned 64-bit ABI. Sizes were cross-checked against the MSVC-compiled C
// declarations (see the C probe recorded in docs/fsr.md). A mismatch here
// means the SDK headers changed; regenerate src/abi.h and these bindings.
const _: () = assert!(size_of::<FloatCoords3D>() == 12);
const _: () = assert!(size_of::<FloatBounds>() == 8);
const _: () = assert!(size_of::<Float4>() == 16);
const _: () = assert!(size_of::<Matrix4x4>() == 64);
const _: () = assert!(size_of::<Rect2D>() == 16);
const _: () = assert!(size_of::<DenoiserSignal>() == 104);
const _: () = assert!(size_of::<CreateContextDescUpscaleVersion>() == 24);
const _: () = assert!(size_of::<CreateBackendDx12Desc>() == 24);
const _: () = assert!(size_of::<CreateContextDescDenoiser>() == 40);
const _: () = assert!(size_of::<DispatchDescDenoiser>() == 448);
const _: () = assert!(size_of::<CreateContextDescRadianceCache>() == 32);
const _: () = assert!(size_of::<RadianceCacheOverrides>() == 8);
const _: () = assert!(size_of::<DispatchDescRadianceCache>() == 280);
const _: () = assert!(size_of::<CreateContextDescFrameGeneration>() == 40);
const _: () = assert!(size_of::<DispatchDescFrameGeneration>() == 312);
const _: () = assert!(size_of::<QueryDescGetVersions>() == 56);
const _: () = assert!(size_of::<OverrideVersion>() == 24);

#[cfg(test)]
#[allow(
    unsafe_code,
    reason = "the in-process stub provider must use the exact unsafe FfxApi C ABI signatures"
)]
mod tests {
    //! CPU-only checks: constants match the SDK macros and the FfxApi stub
    //! chain accepts the new 2.4.0 descriptors. No DLL, GPU or window is used.

    use super::*;
    use crate::native::{AllocationCallbacks, Context, Functions, Module, ReturnCode};
    use std::cell::RefCell;

    // One recording list per test thread: the harness runs each #[test] on its
    // own thread, so this isolates tests that share the extern "C" stubs.
    thread_local! {
        static SEEN: RefCell<Vec<StructType>> = const { RefCell::new(Vec::new()) };
    }

    #[test]
    fn versions_match_sdk_macros() {
        assert_eq!(make_version(4, 1, 2), (4 << 22) | (1 << 12) | 2);
        assert_eq!(versions::UPSCALER, 0x0100_1002);
        assert_eq!(versions::DENOISER, 0x0040_2001);
        assert_eq!(versions::RADIANCECACHE, 0x0000_9000);
        assert_eq!(versions::FRAMEGENERATION, 0x0100_0002);
    }

    #[test]
    fn descriptor_types_match_sdk_macros() {
        assert_eq!(desc_types::UPSCALE_VERSION, 0x0001_000b);
        assert_eq!(desc_types::BACKEND_VK, 0x0100_0003);
        assert_eq!(desc_types::BACKEND_DX12, 0x0000_0002);
        assert_eq!(desc_types::DENOISER_CREATE, 0x0005_0001);
        assert_eq!(desc_types::DENOISER_DISPATCH, 0x0005_0041);
        assert_eq!(desc_types::RADIANCECACHE_CREATE, 0x0006_0002);
        assert_eq!(desc_types::RADIANCECACHE_DISPATCH, 0x0006_0004);
        assert_eq!(desc_types::FRAMEGENERATION_CREATE, 0x0002_0001);
        assert_eq!(desc_types::FRAMEGENERATION_DISPATCH, 0x0002_0003);
        assert_eq!(desc_types::GET_VERSIONS, 4);
        assert_eq!(backend_sub_id(backend_ids::XBOX, 0x03), 0x0200_0003);
    }

    fn record(kind: StructType) {
        SEEN.with(|seen| seen.borrow_mut().push(kind));
    }

    fn assert_seen(kind: StructType) {
        SEEN.with(|seen| {
            assert!(
                seen.borrow().contains(&kind),
                "stub provider never saw descriptor kind {kind:#x}"
            );
        });
    }

    unsafe extern "C" fn stub_create(
        context: *mut Context,
        desc: *mut FfxApiHeader,
        callbacks: *const AllocationCallbacks,
    ) -> ReturnCode {
        assert!(callbacks.is_null());
        let header = unsafe { &*desc };
        record(header.kind);
        // A version descriptor must be chained on every 2.4.0 effect.
        assert!(!header.next.is_null());
        let next = unsafe { &*header.next };
        record(next.kind);
        unsafe {
            *context = std::ptr::NonNull::<u8>::dangling().as_ptr().cast();
        }
        0
    }

    unsafe extern "C" fn stub_dispatch(
        _context: *mut Context,
        desc: *const FfxApiHeader,
    ) -> ReturnCode {
        record(unsafe { (*desc).kind });
        0
    }

    unsafe extern "C" fn stub_query(_context: *mut Context, desc: *mut FfxApiHeader) -> ReturnCode {
        record(unsafe { (*desc).kind });
        0
    }

    unsafe extern "C" fn stub_destroy(
        _context: *mut Context,
        _callbacks: *const AllocationCallbacks,
    ) -> ReturnCode {
        0
    }

    unsafe extern "C" fn stub_configure(
        _context: *mut Context,
        _desc: *const FfxApiHeader,
    ) -> ReturnCode {
        0
    }

    fn module() -> Module {
        Module::stub(Functions {
            create_context: stub_create,
            destroy_context: stub_destroy,
            configure: stub_configure,
            query: stub_query,
            dispatch: stub_dispatch,
        })
    }

    #[test]
    fn denoiser_create_dispatch_destroy_chain_is_well_formed() {
        SEEN.with(|seen| seen.borrow_mut().clear());
        let module = module();
        let mut version = CreateContextDescUpscaleVersion {
            header: FfxApiHeader::new(desc_types::UPSCALE_VERSION),
            version: versions::UPSCALER,
        };
        let mut create = CreateContextDescDenoiser {
            header: FfxApiHeader::new(desc_types::DENOISER_CREATE),
            version: versions::DENOISER,
            max_render_size: Dimensions2D {
                width: 1920,
                height: 1080,
            },
            signal_flags: 0,
            checkerboard_signal_flags: 0,
            flags: 0,
        };
        create.header.next = std::ptr::from_mut(&mut version.header);

        let mut handle: Context = std::ptr::null_mut();
        assert_eq!(module.create_context(&mut handle, &mut create.header), 0);
        assert!(!handle.is_null());

        let dispatch = DispatchDescDenoiser {
            header: FfxApiHeader::new(desc_types::DENOISER_DISPATCH),
            command_list: std::ptr::null_mut(),
            linear_depth: Resource::default(),
            motion_vectors: Resource::default(),
            normals: Resource::default(),
            specular_albedo: Resource::default(),
            diffuse_albedo: Resource::default(),
            motion_vector_scale: FloatCoords3D::default(),
            jitter_offsets: FloatCoords2D::default(),
            camera_position_delta: FloatCoords3D::default(),
            view: Matrix4x4::default(),
            projection: Matrix4x4::default(),
            linear_depth_bounds: FloatBounds::default(),
            render_size: Dimensions2D::default(),
            frame_index: 0,
            flags: 0,
        };
        assert_eq!(module.dispatch(&mut handle, &dispatch.header), 0);
        assert_eq!(module.destroy_context(&mut handle), 0);

        assert_seen(desc_types::DENOISER_CREATE);
        assert_seen(desc_types::UPSCALE_VERSION);
        assert_seen(desc_types::DENOISER_DISPATCH);
    }

    #[test]
    fn radiance_cache_create_dispatch_query_chain_is_well_formed() {
        SEEN.with(|seen| seen.borrow_mut().clear());
        let module = module();
        let mut version = CreateContextDescUpscaleVersion {
            header: FfxApiHeader::new(desc_types::UPSCALE_VERSION),
            version: versions::UPSCALER,
        };
        let mut create = CreateContextDescRadianceCache {
            header: FfxApiHeader::new(desc_types::RADIANCECACHE_CREATE),
            flags: radiance_cache_flags::TRY_FORCE_WMMA,
            version: versions::RADIANCECACHE,
            max_inference_sample_count: 1920 * 1080,
            max_training_sample_count: 1920 * 1080,
        };
        create.header.next = std::ptr::from_mut(&mut version.header);

        let mut handle: Context = std::ptr::null_mut();
        assert_eq!(module.create_context(&mut handle, &mut create.header), 0);
        assert!(!handle.is_null());

        let dispatch = DispatchDescRadianceCache {
            header: FfxApiHeader::new(desc_types::RADIANCECACHE_DISPATCH),
            command_list: std::ptr::null_mut(),
            prediction_inputs: Resource::default(),
            prediction_outputs: Resource::default(),
            train_inputs: Resource::default(),
            train_targets: Resource::default(),
            sample_counters: Resource::default(),
            flags: radiance_cache_flags::DISPATCH_INFERENCE
                | radiance_cache_flags::DISPATCH_TRAINING,
            overrides: RadianceCacheOverrides::default(),
        };
        assert_eq!(module.dispatch(&mut handle, &dispatch.header), 0);

        let mut count: u64 = 0;
        let mut query = QueryDescGetVersions {
            header: FfxApiHeader::new(desc_types::GET_VERSIONS),
            create_desc_type: desc_types::RADIANCECACHE_CREATE,
            device: std::ptr::null_mut(),
            output_count: &mut count,
            version_ids: std::ptr::null_mut(),
            version_names: std::ptr::null_mut(),
        };
        assert_eq!(module.query(&mut handle, &mut query.header), 0);
        assert_eq!(module.destroy_context(&mut handle), 0);

        assert_seen(desc_types::RADIANCECACHE_CREATE);
        assert_seen(desc_types::RADIANCECACHE_DISPATCH);
        assert_seen(desc_types::GET_VERSIONS);
    }

    #[test]
    fn frame_generation_and_dx12_backend_descriptors_are_well_formed() {
        SEEN.with(|seen| seen.borrow_mut().clear());
        let module = module();
        let mut backend = CreateBackendDx12Desc {
            header: FfxApiHeader::new(desc_types::BACKEND_DX12),
            device: std::ptr::NonNull::<u8>::dangling().as_ptr().cast(),
        };
        let mut create = CreateContextDescFrameGeneration {
            header: FfxApiHeader::new(desc_types::FRAMEGENERATION_CREATE),
            flags: 0,
            display_size: Dimensions2D {
                width: 1920,
                height: 1080,
            },
            max_render_size: Dimensions2D {
                width: 1920,
                height: 1080,
            },
            backbuffer_format: 0,
        };
        create.header.next = std::ptr::from_mut(&mut backend.header);

        let mut handle: Context = std::ptr::null_mut();
        assert_eq!(module.create_context(&mut handle, &mut create.header), 0);

        let outputs = [Resource::default(); 4];
        let dispatch = DispatchDescFrameGeneration {
            header: FfxApiHeader::new(desc_types::FRAMEGENERATION_DISPATCH),
            command_list: std::ptr::null_mut(),
            present_color: Resource::default(),
            outputs,
            num_generated_frames: 1,
            reset: false,
            backbuffer_transfer_function: 0,
            min_max_luminance: [0.0, 1.0],
            generation_rect: Rect2D::default(),
            frame_id: 1,
        };
        assert_eq!(module.dispatch(&mut handle, &dispatch.header), 0);
        assert_eq!(module.destroy_context(&mut handle), 0);

        assert_seen(desc_types::FRAMEGENERATION_CREATE);
        assert_seen(desc_types::BACKEND_DX12);
        assert_seen(desc_types::FRAMEGENERATION_DISPATCH);
    }
}
