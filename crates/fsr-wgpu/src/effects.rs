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

use crate::native::StructType;
pub use crate::native::{Dimensions2D, FfxApiHeader, FloatCoords2D, Resource};
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
    /// `FFX_FRAMEGENERATION_SWAPCHAIN_VK_VERSION` — 4.0.0
    /// (`ffx_api_framegeneration_vk.h`).
    pub const FRAMEGENERATION_SWAPCHAIN_VK: u32 = make_version(4, 0, 0);
    /// `FFX_FRAMEGENERATION_SWAPCHAIN_DX12_VERSION` — 4.0.0
    /// (`ffx_api_framegeneration_dx12.h`).
    pub const FRAMEGENERATION_SWAPCHAIN_DX12: u32 = make_version(4, 0, 0);
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

    // ---- Frame-generation swapchain (Vulkan), `ffx_api_framegeneration_vk.h`.
    // `FFX_API_EFFECT_ID_FRAMEGENERATIONSWAPCHAIN_VK` = `0x00040000`; the sub ids
    // below are the literal `FFX_API_MAKE_EFFECT_SUB_ID(..., 0xNN)` values.
    /// `ffxCreateContextDescFrameGenerationSwapChainVK` (`0x00040001`).
    pub const FGSWAPCHAIN_VK_CREATE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN_VK, 0x01);
    /// `ffxConfigureDescFrameGenerationSwapChainRegisterUiResourceVK` (`0x00040002`).
    pub const FGSWAPCHAIN_VK_REGISTER_UI_RESOURCE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN_VK, 0x02);
    /// `ffxQueryDescFrameGenerationSwapChainInterpolationCommandListVK` (`0x00040003`).
    pub const FGSWAPCHAIN_VK_INTERPOLATION_COMMANDLIST: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN_VK, 0x03);
    /// `ffxQueryDescFrameGenerationSwapChainInterpolationTextureVK` (`0x00040004`).
    pub const FGSWAPCHAIN_VK_INTERPOLATION_TEXTURE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN_VK, 0x04);
    /// `ffxQueryDescSwapchainReplacementFunctionsVK` (`0x00040005`).
    pub const FGSWAPCHAIN_VK_REPLACEMENT_FUNCTIONS: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN_VK, 0x05);
    /// `ffxDispatchDescFrameGenerationSwapChainWaitForPresentsVK` (`0x00040007`).
    pub const FGSWAPCHAIN_VK_WAIT_FOR_PRESENTS: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN_VK, 0x07);
    /// `ffxConfigureDescFrameGenerationSwapChainKeyValueVK` (`0x00040008`).
    pub const FGSWAPCHAIN_VK_KEYVALUE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN_VK, 0x08);
    /// `ffxQueryFrameGenerationSwapChainGetGPUMemoryUsageVK` (`0x00040009`).
    pub const FGSWAPCHAIN_VK_GPU_MEMORY_USAGE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN_VK, 0x09);
    /// `ffxCreateContextDescFrameGenerationSwapChainVersionVK` (`0x0004000a`).
    pub const FGSWAPCHAIN_VK_VERSION_DESC: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN_VK, 0x0a);
    /// `ffxQueryFrameGenerationSwapChainGetGPUMemoryUsageVKV2` (`0x0004000b`).
    pub const FGSWAPCHAIN_VK_GPU_MEMORY_USAGE_V2: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN_VK, 0x0b);
    /// `ffxCreateContextDescFrameGenerationSwapChainModeVK` (`0x00040010`).
    pub const FGSWAPCHAIN_VK_MODE_CREATE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN_VK, 0x10);

    // ---- Frame-generation swapchain (DX12), `ffx_api_framegeneration_dx12.h`.
    // The header uses `FFX_API_MAKE_BACKEND_EFFECT_SUB_ID(BACKEND_ID_DX12, ...)`,
    // but `DX12` backend id is zero, so these equal `effect_sub_id(FGSWAPCHAIN, ...)`.
    /// `ffxCreateContextDescFrameGenerationSwapChainWrapDX12` (`0x00030001`).
    pub const FGSWAPCHAIN_DX12_WRAP_CREATE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN, 0x01);
    /// `ffxConfigureDescFrameGenerationSwapChainRegisterUiResourceDX12` (`0x00030002`).
    pub const FGSWAPCHAIN_DX12_REGISTER_UI_RESOURCE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN, 0x02);
    /// `ffxQueryDescFrameGenerationSwapChainInterpolationCommandListDX12` (`0x00030003`).
    pub const FGSWAPCHAIN_DX12_INTERPOLATION_COMMANDLIST: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN, 0x03);
    /// `ffxQueryDescFrameGenerationSwapChainInterpolationTextureDX12` (`0x00030004`).
    pub const FGSWAPCHAIN_DX12_INTERPOLATION_TEXTURE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN, 0x04);
    /// `ffxCreateContextDescFrameGenerationSwapChainNewDX12` (`0x00030005`).
    pub const FGSWAPCHAIN_DX12_NEW_CREATE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN, 0x05);
    /// `ffxCreateContextDescFrameGenerationSwapChainForHwndDX12` (`0x00030006`).
    pub const FGSWAPCHAIN_DX12_FOR_HWND_CREATE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN, 0x06);
    /// `ffxDispatchDescFrameGenerationSwapChainWaitForPresentsDX12` (`0x00030007`).
    pub const FGSWAPCHAIN_DX12_WAIT_FOR_PRESENTS: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN, 0x07);
    /// `ffxConfigureDescFrameGenerationSwapChainKeyValueDX12` (`0x00030008`).
    pub const FGSWAPCHAIN_DX12_KEYVALUE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN, 0x08);
    /// `ffxQueryFrameGenerationSwapChainGetGPUMemoryUsageDX12` (`0x00030009`).
    pub const FGSWAPCHAIN_DX12_GPU_MEMORY_USAGE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN, 0x09);
    /// `ffxQueryFrameGenerationSwapChainGetGPUMemoryUsageDX12V2` (`0x0003000a`).
    pub const FGSWAPCHAIN_DX12_GPU_MEMORY_USAGE_V2: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN, 0x0a);
    /// `ffxCreateContextDescFrameGenerationSwapChainVersionDX12` (`0x0003000b`).
    pub const FGSWAPCHAIN_DX12_VERSION: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATIONSWAPCHAIN, 0x0b);

    // ---- Frame generation, `ffx_framegeneration.h`.
    // NOTE: the guessed sub id `0x02` is actually the configure descriptor;
    // the deprecated `PREPARE` is `0x04` and its replacement `PREPARE_V2` is `0x0c`.
    /// `ffxConfigureDescFrameGeneration` (`0x00020002`).
    pub const FRAMEGENERATION_CONFIGURE: u64 = effect_sub_id(effect_ids::FRAMEGENERATION, 0x02);
    /// `ffxDispatchDescFrameGenerationPrepare` (`0x00020004`, deprecated).
    pub const FRAMEGENERATION_PREPARE: u64 = effect_sub_id(effect_ids::FRAMEGENERATION, 0x04);
    /// `ffxCallbackDescFrameGenerationPresent` (`0x00020005`).
    pub const FRAMEGENERATION_PRESENT_CALLBACK: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATION, 0x05);
    /// `ffxConfigureDescFrameGenerationKeyValue` (`0x00020006`).
    pub const FRAMEGENERATION_KEYVALUE: u64 = effect_sub_id(effect_ids::FRAMEGENERATION, 0x06);
    /// `ffxQueryDescFrameGenerationGetGPUMemoryUsage` (`0x00020007`).
    pub const FRAMEGENERATION_GPU_MEMORY_USAGE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATION, 0x07);
    /// `ffxConfigureDescFrameGenerationRegisterDistortionFieldResource` (`0x00020008`).
    pub const FRAMEGENERATION_REGISTER_DISTORTION_RESOURCE: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATION, 0x08);
    /// `ffxCreateContextDescFrameGenerationHudless` (`0x00020009`).
    pub const FRAMEGENERATION_HUDLESS: u64 = effect_sub_id(effect_ids::FRAMEGENERATION, 0x09);
    /// `ffxDispatchDescFrameGenerationPrepareCameraInfo` (`0x0002000a`, deprecated).
    pub const FRAMEGENERATION_PREPARE_CAMERA_INFO: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATION, 0x0a);
    /// `ffxQueryDescFrameGenerationGetGPUMemoryUsageV2` (`0x0002000b`).
    pub const FRAMEGENERATION_GPU_MEMORY_USAGE_V2: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATION, 0x0b);
    /// `ffxDispatchDescFrameGenerationPrepareV2` (`0x0002000c`).
    pub const FRAMEGENERATION_PREPARE_V2: u64 = effect_sub_id(effect_ids::FRAMEGENERATION, 0x0c);
    /// `ffxCallbackDescFrameGenerationPresentPremulAlpha` (`0x0002000d`).
    pub const FRAMEGENERATION_PRESENT_PREMUL_ALPHA: u64 =
        effect_sub_id(effect_ids::FRAMEGENERATION, 0x0d);
    /// `FFX_API_FRAME_GENERATION_CONFIG` (`0x0002000f`).
    pub const FRAMEGENERATION_CONFIG: u64 = effect_sub_id(effect_ids::FRAMEGENERATION, 0x0f);

    // ---- Denoiser, `ffx_denoiser.h` (2.4.0). The 2.4.0 header uses sub ids
    // `0x42..=0x49` for the `pNext`-chained per-signal dispatch descriptors, and
    // `0x81`/`0x82` for the key-value and GPU-memory queries. The pre-existing
    // `DENOISER_QUERY_GPU_MEMORY_USAGE` (`0x07`) is retained for compatibility
    // but is NOT a 2.4.0 descriptor; `0x82` is the real value.
    /// `ffxQueryDescDenoiserGetDefaultKeyValue` (`0x00050081`).
    pub const DENOISER_QUERY_DEFAULT_KEYVALUE: u64 = effect_sub_id(effect_ids::DENOISER, 0x81);
    /// `ffxQueryDescDenoiserGetGPUMemoryUsage` (`0x00050082`).
    pub const DENOISER_QUERY_GPU_MEMORY_USAGE_2: u64 = effect_sub_id(effect_ids::DENOISER, 0x82);
    /// `ffxDispatchDescDenoiserDebugView` (`0x00050042`).
    pub const DENOISER_DISPATCH_DEBUG_VIEW: u64 = effect_sub_id(effect_ids::DENOISER, 0x42);
    /// `ffxDispatchDescDenoiserAmbientOcclusion` (`0x00050043`).
    pub const DENOISER_DISPATCH_AMBIENT_OCCLUSION: u64 = effect_sub_id(effect_ids::DENOISER, 0x43);
    /// `ffxDispatchDescDenoiserDirectDiffuse` (`0x00050044`).
    pub const DENOISER_DISPATCH_DIRECT_DIFFUSE: u64 = effect_sub_id(effect_ids::DENOISER, 0x44);
    /// `ffxDispatchDescDenoiserDirectSpecular` (`0x00050045`).
    pub const DENOISER_DISPATCH_DIRECT_SPECULAR: u64 = effect_sub_id(effect_ids::DENOISER, 0x45);
    /// `ffxDispatchDescDenoiserDominantLight` (`0x00050046`).
    pub const DENOISER_DISPATCH_DOMINANT_LIGHT: u64 = effect_sub_id(effect_ids::DENOISER, 0x46);
    /// `ffxDispatchDescDenoiserIndirectDiffuse` (`0x00050047`).
    pub const DENOISER_DISPATCH_INDIRECT_DIFFUSE: u64 = effect_sub_id(effect_ids::DENOISER, 0x47);
    /// `ffxDispatchDescDenoiserIndirectSpecular` (`0x00050048`).
    pub const DENOISER_DISPATCH_INDIRECT_SPECULAR: u64 = effect_sub_id(effect_ids::DENOISER, 0x48);
    /// `ffxDispatchDescDenoiserSpecularOcclusion` (`0x00050049`).
    pub const DENOISER_DISPATCH_SPECULAR_OCCLUSION: u64 = effect_sub_id(effect_ids::DENOISER, 0x49);

    // ---- Radiance cache, `ffx_radiancecache.h` (2.4.0). The header defines NO
    // GPU-memory query at sub id `0x07`; the only extra dispatch is the debug
    // dispatch at `0x05`.
    /// `ffxDebugDispatchDescRadianceCache` (`0x00060005`).
    pub const RADIANCECACHE_DEBUG_DISPATCH: u64 = effect_sub_id(effect_ids::RADIANCECACHE, 0x05);
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

// ---------------------------------------------------------------------------
// Frame-generation swapchain (Vulkan). `VkSwapchainCreateInfoKHR`,
// `VkAllocationCallbacks` and every `VkQueue` / `PFN_vkQueueSubmitFFXAPI` are
// modeled as opaque pointers on a 64-bit target (8 bytes each), matching the
// MSVC `sizeof` probe rather than expanding Vulkan's by-value structs.
// ---------------------------------------------------------------------------

/// `VkQueueInfoFFXAPI` (`ffx_api_vk.h`): a `VkQueue` handle, its family index
/// and an optional submit function pointer, all opaque on a 64-bit target.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct QueueInfoVk {
    /// `VkQueue` handle (opaque).
    pub queue: *mut c_void,
    /// Queue family index used for ownership transfer.
    pub family_index: u32,
    /// `PFN_vkQueueSubmitFFXAPI` (opaque function pointer).
    pub submit_func: *mut c_void,
}

/// `ffxCreateContextDescFrameGenerationSwapChainVK` (`0x00040001`).
#[repr(C)]
pub struct CreateContextDescFrameGenerationSwapChainVk {
    /// Descriptor header (`desc_types::FGSWAPCHAIN_VK_CREATE`).
    pub header: FfxApiHeader,
    /// `VkPhysicalDevice` (opaque).
    pub physical_device: *mut c_void,
    /// `VkDevice` (opaque).
    pub device: *mut c_void,
    /// `VkSwapchainKHR*` (in/out handle). The swapchain it names is destroyed
    /// during creation; on return it holds the frame-generation swapchain.
    pub swapchain: *mut u64,
    /// `VkAllocationCallbacks*` (opaque, optional).
    pub allocator: *mut c_void,
    /// `VkSwapchainCreateInfoKHR`, held **by value** in the v2.4.0 header (104
    /// bytes on x64), not by pointer.
    pub create_info: ash::vk::SwapchainCreateInfoKHR<'static>,
    /// Main graphics queue (`VkQueueInfoFFXAPI`).
    pub game_queue: QueueInfoVk,
    /// Async compute queue (`VkQueueInfoFFXAPI`).
    pub async_compute_queue: QueueInfoVk,
    /// Present queue (`VkQueueInfoFFXAPI`).
    pub present_queue: QueueInfoVk,
    /// Image-acquire queue (`VkQueueInfoFFXAPI`).
    pub image_acquire_queue: QueueInfoVk,
}

/// `ffxQueryDescSwapchainReplacementFunctionsVK` (`0x00040005`): header plus
/// seven replaced swapchain function pointers, each modeled as `void*`.
#[repr(C)]
pub struct QueryDescSwapchainReplacementFunctionsVk {
    /// Descriptor header (`desc_types::FGSWAPCHAIN_VK_REPLACEMENT_FUNCTIONS`).
    pub header: FfxApiHeader,
    /// `PFN_vkCreateSwapchainFFXAPI`.
    pub create_swapchain: *mut c_void,
    /// `PFN_vkDestroySwapchainFFXAPI`.
    pub destroy_swapchain: *mut c_void,
    /// Replacement of `vkGetSwapchainImagesKHR`.
    pub get_swapchain_images: *mut c_void,
    /// Replacement of `vkAcquireNextImageKHR`.
    pub acquire_next_image: *mut c_void,
    /// Replacement of `vkQueuePresentKHR`.
    pub queue_present: *mut c_void,
    /// Replacement of `vkSetHdrMetadataEXT`.
    pub set_hdr_metadata: *mut c_void,
    /// `PFN_getLastPresentCountFFXAPI`.
    pub get_last_present_count: *mut c_void,
}

/// `ffxConfigureDescFrameGenerationSwapChainRegisterUiResourceVK`
/// (`0x00040002`): header plus a UI `FfxApiResource` and composition flags.
#[repr(C)]
pub struct ConfigureDescFrameGenerationSwapChainRegisterUiResourceVk {
    /// Descriptor header (`desc_types::FGSWAPCHAIN_VK_REGISTER_UI_RESOURCE`).
    pub header: FfxApiHeader,
    /// UI resource for composition.
    pub ui_resource: Resource,
    /// `FfxApiUiCompositionFlags` bits.
    pub flags: u32,
}

/// `ffxQueryDescFrameGenerationSwapChainInterpolationCommandListVK`
/// (`0x00040003`).
#[repr(C)]
pub struct QueryDescFrameGenerationSwapChainInterpolationCommandListVk {
    /// Descriptor header (`desc_types::FGSWAPCHAIN_VK_INTERPOLATION_COMMANDLIST`).
    pub header: FfxApiHeader,
    /// Out pointer to a command buffer (opaque `void**`).
    pub p_out_command_list: *mut *mut c_void,
}

/// `ffxQueryDescFrameGenerationSwapChainInterpolationTextureVK`
/// (`0x00040004`).
#[repr(C)]
pub struct QueryDescFrameGenerationSwapChainInterpolationTextureVk {
    /// Descriptor header (`desc_types::FGSWAPCHAIN_VK_INTERPOLATION_TEXTURE`).
    pub header: FfxApiHeader,
    /// Out pointer to an `FfxApiResource`.
    pub p_out_texture: *mut Resource,
}

/// `ffxDispatchDescFrameGenerationSwapChainWaitForPresentsVK`
/// (`0x00040007`): header only.
#[repr(C)]
pub struct DispatchDescFrameGenerationSwapChainWaitForPresentsVk {
    /// Descriptor header (`desc_types::FGSWAPCHAIN_VK_WAIT_FOR_PRESENTS`).
    pub header: FfxApiHeader,
}

/// `ffxCreateContextDescFrameGenerationSwapChainModeVK` (`0x00040010`),
/// chained on the create descriptor.
#[repr(C)]
pub struct CreateContextDescFrameGenerationSwapChainModeVk {
    /// Descriptor header (`desc_types::FGSWAPCHAIN_VK_MODE_CREATE`).
    pub header: FfxApiHeader,
    /// Whether composition happens on the present queue.
    pub compose_on_present_queue: bool,
}

/// `ffxCreateContextDescFrameGenerationSwapChainVersionVK` (`0x0004000a`),
/// chained on the create descriptor.
#[repr(C)]
pub struct CreateContextDescFrameGenerationSwapChainVersionVk {
    /// Descriptor header (`desc_types::FGSWAPCHAIN_VK_VERSION_DESC`).
    pub header: FfxApiHeader,
    /// Must equal [`versions::FRAMEGENERATION_SWAPCHAIN_VK`].
    pub version: u32,
}

/// `ffxConfigureDescFrameGenerationSwapChainKeyValueVK` (`0x00040008`).
#[repr(C)]
pub struct ConfigureDescFrameGenerationSwapChainKeyValueVk {
    /// Descriptor header (`desc_types::FGSWAPCHAIN_VK_KEYVALUE`).
    pub header: FfxApiHeader,
    /// Configuration key (`FfxApiConfigureFrameGenerationSwapChainKeyVK`).
    pub key: u64,
    /// Integer or enum value (`u64` in the C header).
    pub value: u64,
    /// Pointer (or pointer-to-value).
    pub ptr: *mut c_void,
}

// ---------------------------------------------------------------------------
// Frame generation (Vulkan + DX12), `ffx_framegeneration.h`.
// ---------------------------------------------------------------------------

/// `ffxDispatchDescFrameGenerationPrepare` (`0x00020004`, deprecated in favor
/// of [`DispatchDescFrameGenerationPrepareV2`]). Field order is transcribed
/// from `ffx_framegeneration.h`.
#[repr(C)]
pub struct DispatchDescFrameGenerationPrepare {
    /// Descriptor header (`desc_types::FRAMEGENERATION_PREPARE`).
    pub header: FfxApiHeader,
    /// Monotonic frame identifier.
    pub frame_id: u64,
    /// `FfxApiDispatchFramegenerationFlags` bits.
    pub flags: u32,
    /// Command list cast to `void*`.
    pub command_list: *mut c_void,
    /// Dimensions used to render game content.
    pub render_size: Dimensions2D,
    /// Subpixel jitter offset.
    pub jitter_offset: FloatCoords2D,
    /// Motion-vector scale to pixel space.
    pub motion_vector_scale: FloatCoords2D,
    /// Frame time in milliseconds.
    pub frame_time_delta: f32,
    /// Unused reset field (kept for ABI compatibility).
    pub unused_reset: bool,
    /// Near plane distance.
    pub camera_near: f32,
    /// Far plane distance.
    pub camera_far: f32,
    /// Vertical field of view, in radians.
    pub camera_fov_angle_vertical: f32,
    /// View-space-to-meters scale.
    pub view_space_to_meters_factor: f32,
    /// Depth buffer.
    pub depth: Resource,
    /// Motion vectors.
    pub motion_vectors: Resource,
}

/// `ffxDispatchDescFrameGenerationPrepareV2` (`0x0002000c`): adds the four
/// camera basis vectors to [`DispatchDescFrameGenerationPrepare`].
#[repr(C)]
pub struct DispatchDescFrameGenerationPrepareV2 {
    /// Descriptor header (`desc_types::FRAMEGENERATION_PREPARE_V2`).
    pub header: FfxApiHeader,
    /// Monotonic frame identifier.
    pub frame_id: u64,
    /// `FfxApiDispatchFramegenerationFlags` bits.
    pub flags: u32,
    /// Command list cast to `void*`.
    pub command_list: *mut c_void,
    /// Dimensions used to render game content.
    pub render_size: Dimensions2D,
    /// Subpixel jitter offset.
    pub jitter_offset: FloatCoords2D,
    /// Motion-vector scale to pixel space.
    pub motion_vector_scale: FloatCoords2D,
    /// Frame time in milliseconds.
    pub frame_time_delta: f32,
    /// Camera-cut / discontinuous-motion reset.
    pub reset: bool,
    /// Near plane distance.
    pub camera_near: f32,
    /// Far plane distance.
    pub camera_far: f32,
    /// Vertical field of view, in radians.
    pub camera_fov_angle_vertical: f32,
    /// View-space-to-meters scale.
    pub view_space_to_meters_factor: f32,
    /// Depth buffer.
    pub depth: Resource,
    /// Motion vectors.
    pub motion_vectors: Resource,
    /// Camera position in world space.
    pub camera_position: [f32; 3],
    /// Camera up vector (normalized).
    pub camera_up: [f32; 3],
    /// Camera right vector (normalized).
    pub camera_right: [f32; 3],
    /// Camera forward vector (normalized).
    pub camera_forward: [f32; 3],
}

/// `ffxCallbackDescFrameGenerationPresent` (`0x00020005`).
#[repr(C)]
pub struct CallbackDescFrameGenerationPresent {
    /// Descriptor header (`desc_types::FRAMEGENERATION_PRESENT_CALLBACK`).
    pub header: FfxApiHeader,
    /// GPU device cast to `void*` (`ID3D12Device` or `VkDevice`).
    pub device: *mut c_void,
    /// Command list executed before presentation, cast to `void*`.
    pub command_list: *mut c_void,
    /// Backbuffer image (rendered or generated).
    pub current_back_buffer: Resource,
    /// UI image for composition (may be empty).
    pub current_ui: Resource,
    /// Output swapchain image that will be presented.
    pub output_swap_chain_buffer: Resource,
    /// Whether this frame is generated.
    pub is_generated_frame: bool,
    /// Monotonic frame identifier.
    pub frame_id: u64,
}

/// `FfxApiPresentCallbackFunc`: UI composition for each presented frame.
pub type PresentCallbackFn =
    unsafe extern "C" fn(params: *mut CallbackDescFrameGenerationPresent, user: *mut c_void) -> u32;

/// `FfxApiFrameGenerationDispatchFunc`: called by the frame-generation swapchain
/// at present time with a filled [`DispatchDescFrameGeneration`] (command list,
/// present color and outputs owned by the swapchain).
pub type FrameGenerationDispatchFn =
    unsafe extern "C" fn(params: *mut DispatchDescFrameGeneration, user: *mut c_void) -> u32;

/// `ffxConfigureDescFrameGeneration` (`0x00020002`), issued once per frame.
#[repr(C)]
pub struct ConfigureDescFrameGeneration {
    /// Descriptor header (`desc_types::FRAMEGENERATION_CONFIGURE`).
    pub header: FfxApiHeader,
    /// `IDXGISwapChain*` (DX12) or `VkSwapchainKHR` (Vulkan) of the
    /// frame-generation swapchain.
    pub swap_chain: *mut c_void,
    /// Optional UI composition callback; `None` selects the default copy.
    pub present_callback: Option<PresentCallbackFn>,
    /// User pointer for `present_callback`.
    pub present_callback_user_context: *mut c_void,
    /// Frame-generation dispatch callback invoked by the swapchain at present.
    pub frame_generation_callback: Option<FrameGenerationDispatchFn>,
    /// User pointer for `frame_generation_callback`.
    pub frame_generation_callback_user_context: *mut c_void,
    /// Enables frame generation for this frame.
    pub frame_generation_enabled: bool,
    /// Allows generation work on async compute.
    pub allow_async_workloads: bool,
    /// Optional HUD-less backbuffer used for UI extraction.
    pub hudless_color: Resource,
    /// `FfxApiDispatchFrameGenerationFlags` bits.
    pub flags: u32,
    /// Present only generated frames.
    pub only_present_generated: bool,
    /// Backbuffer area that is generated.
    pub generation_rect: Rect2D,
    /// Must increment by exactly one per frame.
    pub frame_id: u64,
}

/// `ffxCreateContextDescFrameGenerationVersion` (`0x0002000e`), chained on the
/// frame-generation create descriptor.
#[repr(C)]
pub struct CreateContextDescFrameGenerationVersion {
    /// Descriptor header (`desc_types::FRAMEGENERATION_VERSION`).
    pub header: FfxApiHeader,
    /// Must equal [`versions::FRAMEGENERATION`].
    pub version: u32,
}

/// `ffxCreateContextDescFrameGenerationSwapChainForHwndDX12` (`0x00030006`).
#[repr(C)]
pub struct CreateContextDescFrameGenerationSwapChainForHwndDx12 {
    /// Descriptor header (`desc_types::FGSWAPCHAIN_DX12_FOR_HWND_CREATE`).
    pub header: FfxApiHeader,
    /// Out: `IDXGISwapChain4*` carrying one reference for the caller.
    pub swapchain: *mut *mut c_void,
    /// Target `HWND`.
    pub hwnd: *mut c_void,
    /// `DXGI_SWAP_CHAIN_DESC1*`.
    pub desc: *mut c_void,
    /// `DXGI_SWAP_CHAIN_FULLSCREEN_DESC*`, NULL for windowed.
    pub fullscreen_desc: *mut c_void,
    /// `IDXGIFactory*` used to create the real swapchain.
    pub dxgi_factory: *mut c_void,
    /// `ID3D12CommandQueue*` the game presents on.
    pub game_queue: *mut c_void,
}

/// `ffxCreateContextDescFrameGenerationSwapChainVersionDX12` (`0x0003000b`),
/// chained on the swapchain create descriptor (required for the 4.0 API).
#[repr(C)]
pub struct CreateContextDescFrameGenerationSwapChainVersionDx12 {
    /// Descriptor header (`desc_types::FGSWAPCHAIN_DX12_VERSION`).
    pub header: FfxApiHeader,
    /// Must equal [`versions::FRAMEGENERATION_SWAPCHAIN_DX12`].
    pub version: u32,
}

/// `FfxApiDispatchFramegenerationFlags`.
pub mod framegeneration_dispatch_flags {
    /// Draw tear-line bars on generated frames (`1 << 0`).
    pub const DRAW_DEBUG_TEAR_LINES: u32 = 1 << 0;
    /// Draw reset indicators (`1 << 1`).
    pub const DRAW_DEBUG_RESET_INDICATORS: u32 = 1 << 1;
    /// Output debug views (`1 << 2`).
    pub const DRAW_DEBUG_VIEW: u32 = 1 << 2;
    /// Draw pacing lines (`1 << 4`).
    pub const DRAW_DEBUG_PACING_LINES: u32 = 1 << 4;
}

/// `FfxApiCreateContextFramegenerationFlags`.
pub mod framegeneration_create_flags {
    /// Async compute workload support (`1 << 0`).
    pub const ENABLE_ASYNC_WORKLOAD_SUPPORT: u32 = 1 << 0;
    /// Motion vectors are at display resolution (`1 << 1`).
    pub const ENABLE_DISPLAY_RESOLUTION_MOTION_VECTORS: u32 = 1 << 1;
    /// Motion vectors include the jitter pattern (`1 << 2`).
    pub const ENABLE_MOTION_VECTORS_JITTER_CANCELLATION: u32 = 1 << 2;
    /// Depth is inverted `[1..0]` (`1 << 3`).
    pub const ENABLE_DEPTH_INVERTED: u32 = 1 << 3;
    /// Depth uses an infinite far plane (`1 << 4`).
    pub const ENABLE_DEPTH_INFINITE: u32 = 1 << 4;
    /// Input color is HDR (`1 << 5`).
    pub const ENABLE_HIGH_DYNAMIC_RANGE: u32 = 1 << 5;
    /// Runtime API value checking (`1 << 6`).
    pub const ENABLE_DEBUG_CHECKING: u32 = 1 << 6;
}

// ---------------------------------------------------------------------------
// Denoiser `pNext`-chained per-signal dispatch descriptors, `ffx_denoiser.h`.
// ---------------------------------------------------------------------------

/// `ffxDispatchDescDenoiserDebugView` (`0x00050042`): header plus an output
/// `FfxApiResource`, its size and the debug-view mode / viewport index.
#[repr(C)]
pub struct DispatchDescDenoiserDebugView {
    /// Descriptor header (`desc_types::DENOISER_DISPATCH_DEBUG_VIEW`).
    pub header: FfxApiHeader,
    /// Target output resource for debug visualization.
    pub output: Resource,
    /// Resolution of the debug visualization.
    pub output_size: Dimensions2D,
    /// `FfxApiDenoiserDebugViewMode`.
    pub mode: u32,
    /// Viewport index for fullscreen-viewport mode.
    pub viewport_index: u32,
}

/// `ffxDispatchDescDenoiserAmbientOcclusion` (`0x00050043`).
#[repr(C)]
pub struct DispatchDescDenoiserAmbientOcclusion {
    /// Descriptor header (`desc_types::DENOISER_DISPATCH_AMBIENT_OCCLUSION`).
    pub header: FfxApiHeader,
    /// Ambient-occlusion input/output signal pair.
    pub signal: DenoiserSignal,
}

/// `ffxDispatchDescDenoiserDirectDiffuse` (`0x00050044`).
#[repr(C)]
pub struct DispatchDescDenoiserDirectDiffuse {
    /// Descriptor header (`desc_types::DENOISER_DISPATCH_DIRECT_DIFFUSE`).
    pub header: FfxApiHeader,
    /// Direct-diffuse input/output signal pair.
    pub signal: DenoiserSignal,
}

/// `ffxDispatchDescDenoiserDirectSpecular` (`0x00050045`).
#[repr(C)]
pub struct DispatchDescDenoiserDirectSpecular {
    /// Descriptor header (`desc_types::DENOISER_DISPATCH_DIRECT_SPECULAR`).
    pub header: FfxApiHeader,
    /// Direct-specular input/output signal pair.
    pub signal: DenoiserSignal,
}

/// `ffxDispatchDescDenoiserDominantLight` (`0x00050046`).
#[repr(C)]
pub struct DispatchDescDenoiserDominantLight {
    /// Descriptor header (`desc_types::DENOISER_DISPATCH_DOMINANT_LIGHT`).
    pub header: FfxApiHeader,
    /// Dominant-light visibility input/output signal pair.
    pub signal: DenoiserSignal,
    /// Dominant light direction (from source to target).
    pub direction: FloatCoords3D,
    /// Dominant light emission.
    pub emission: FloatCoords3D,
    /// Half-angle of the light cone, in radians.
    pub angular_radius: f32,
}

/// `ffxDispatchDescDenoiserIndirectDiffuse` (`0x00050047`).
#[repr(C)]
pub struct DispatchDescDenoiserIndirectDiffuse {
    /// Descriptor header (`desc_types::DENOISER_DISPATCH_INDIRECT_DIFFUSE`).
    pub header: FfxApiHeader,
    /// Indirect-diffuse input/output signal pair.
    pub signal: DenoiserSignal,
}

/// `ffxDispatchDescDenoiserIndirectSpecular` (`0x00050048`).
#[repr(C)]
pub struct DispatchDescDenoiserIndirectSpecular {
    /// Descriptor header (`desc_types::DENOISER_DISPATCH_INDIRECT_SPECULAR`).
    pub header: FfxApiHeader,
    /// Indirect-specular input/output signal pair.
    pub signal: DenoiserSignal,
}

/// `ffxDispatchDescDenoiserSpecularOcclusion` (`0x00050049`).
#[repr(C)]
pub struct DispatchDescDenoiserSpecularOcclusion {
    /// Descriptor header (`desc_types::DENOISER_DISPATCH_SPECULAR_OCCLUSION`).
    pub header: FfxApiHeader,
    /// Specular-occlusion input/output signal pair.
    pub signal: DenoiserSignal,
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

// Measured with the MSVC C++ `sizeof` probe (see the report / `src/abi.h`).
// Opaque Vulkan handles and function pointers are all 8-byte `void*`.
const _: () = assert!(size_of::<QueueInfoVk>() == 24);
// `VkSwapchainCreateInfoKHR` is embedded by value (104 bytes): 48 + 104 + 4 * 24.
const _: () = assert!(size_of::<ash::vk::SwapchainCreateInfoKHR<'static>>() == 104);
const _: () = assert!(size_of::<CreateContextDescFrameGenerationSwapChainVk>() == 248);
const _: () =
    assert!(std::mem::offset_of!(CreateContextDescFrameGenerationSwapChainVk, game_queue) == 152);
// Frame-generation configure / DX12 swapchain descriptors (x64, v2.4.0 headers).
const _: () = assert!(size_of::<ConfigureDescFrameGeneration>() == 144);
const _: () = assert!(std::mem::offset_of!(ConfigureDescFrameGeneration, hudless_color) == 64);
const _: () = assert!(std::mem::offset_of!(ConfigureDescFrameGeneration, flags) == 112);
const _: () = assert!(std::mem::offset_of!(ConfigureDescFrameGeneration, generation_rect) == 120);
const _: () = assert!(std::mem::offset_of!(ConfigureDescFrameGeneration, frame_id) == 136);
const _: () = assert!(size_of::<CreateContextDescFrameGenerationVersion>() == 24);
const _: () = assert!(size_of::<CreateContextDescFrameGenerationSwapChainForHwndDx12>() == 64);
const _: () = assert!(size_of::<CreateContextDescFrameGenerationSwapChainVersionDx12>() == 24);
const _: () =
    assert!(std::mem::offset_of!(DispatchDescFrameGenerationPrepareV2, camera_position) == 184);
const _: () = assert!(size_of::<QueryDescSwapchainReplacementFunctionsVk>() == 72);
const _: () = assert!(size_of::<ConfigureDescFrameGenerationSwapChainRegisterUiResourceVk>() == 72);
const _: () =
    assert!(size_of::<QueryDescFrameGenerationSwapChainInterpolationCommandListVk>() == 24);
const _: () = assert!(size_of::<QueryDescFrameGenerationSwapChainInterpolationTextureVk>() == 24);
const _: () = assert!(size_of::<DispatchDescFrameGenerationSwapChainWaitForPresentsVk>() == 16);
const _: () = assert!(size_of::<CreateContextDescFrameGenerationSwapChainModeVk>() == 24);
const _: () = assert!(size_of::<CreateContextDescFrameGenerationSwapChainVersionVk>() == 24);
const _: () = assert!(size_of::<ConfigureDescFrameGenerationSwapChainKeyValueVk>() == 40);
const _: () = assert!(size_of::<DispatchDescFrameGenerationPrepare>() == 184);
const _: () = assert!(size_of::<DispatchDescFrameGenerationPrepareV2>() == 232);
const _: () = assert!(size_of::<CallbackDescFrameGenerationPresent>() == 192);
const _: () = assert!(size_of::<DispatchDescDenoiserDebugView>() == 80);
const _: () = assert!(size_of::<DispatchDescDenoiserAmbientOcclusion>() == 120);
const _: () = assert!(size_of::<DispatchDescDenoiserDirectDiffuse>() == 120);
const _: () = assert!(size_of::<DispatchDescDenoiserDirectSpecular>() == 120);
const _: () = assert!(size_of::<DispatchDescDenoiserDominantLight>() == 152);
const _: () = assert!(size_of::<DispatchDescDenoiserIndirectDiffuse>() == 120);
const _: () = assert!(size_of::<DispatchDescDenoiserIndirectSpecular>() == 120);
const _: () = assert!(size_of::<DispatchDescDenoiserSpecularOcclusion>() == 120);

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

    /// Record every descriptor kind along a `pNext` chain, so chained
    /// create/configure/dispatch/query descriptors are all observable.
    unsafe fn record_chain(mut header: *mut FfxApiHeader) {
        while !header.is_null() {
            record(unsafe { (*header).kind });
            header = unsafe { (*header).next };
        }
    }

    unsafe extern "C" fn stub_create(
        context: *mut Context,
        desc: *mut FfxApiHeader,
        callbacks: *const AllocationCallbacks,
    ) -> ReturnCode {
        assert!(callbacks.is_null());
        // A version descriptor must be chained on every 2.4.0 effect.
        assert!(!unsafe { (*desc).next }.is_null());
        unsafe { record_chain(desc) };
        unsafe {
            *context = std::ptr::NonNull::<u8>::dangling().as_ptr().cast();
        }
        0
    }

    unsafe extern "C" fn stub_dispatch(
        _context: *mut Context,
        desc: *const FfxApiHeader,
    ) -> ReturnCode {
        unsafe { record_chain(desc.cast_mut()) };
        0
    }

    unsafe extern "C" fn stub_query(_context: *mut Context, desc: *mut FfxApiHeader) -> ReturnCode {
        unsafe { record_chain(desc) };
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
        desc: *const FfxApiHeader,
    ) -> ReturnCode {
        unsafe { record_chain(desc.cast_mut()) };
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

    #[test]
    fn new_2_4_0_constants_match_the_headers() {
        assert_eq!(versions::FRAMEGENERATION_SWAPCHAIN_VK, 0x0100_0000);
        assert_eq!(versions::FRAMEGENERATION_SWAPCHAIN_DX12, 0x0100_0000);
        assert_eq!(desc_types::FGSWAPCHAIN_VK_CREATE, 0x0004_0001);
        assert_eq!(desc_types::FGSWAPCHAIN_VK_REGISTER_UI_RESOURCE, 0x0004_0002);
        assert_eq!(
            desc_types::FGSWAPCHAIN_VK_INTERPOLATION_COMMANDLIST,
            0x0004_0003
        );
        assert_eq!(
            desc_types::FGSWAPCHAIN_VK_INTERPOLATION_TEXTURE,
            0x0004_0004
        );
        assert_eq!(
            desc_types::FGSWAPCHAIN_VK_REPLACEMENT_FUNCTIONS,
            0x0004_0005
        );
        assert_eq!(desc_types::FGSWAPCHAIN_VK_WAIT_FOR_PRESENTS, 0x0004_0007);
        assert_eq!(desc_types::FGSWAPCHAIN_VK_KEYVALUE, 0x0004_0008);
        assert_eq!(desc_types::FGSWAPCHAIN_VK_GPU_MEMORY_USAGE, 0x0004_0009);
        assert_eq!(desc_types::FGSWAPCHAIN_VK_VERSION_DESC, 0x0004_000a);
        assert_eq!(desc_types::FGSWAPCHAIN_VK_GPU_MEMORY_USAGE_V2, 0x0004_000b);
        assert_eq!(desc_types::FGSWAPCHAIN_VK_MODE_CREATE, 0x0004_0010);
        assert_eq!(desc_types::FGSWAPCHAIN_DX12_WRAP_CREATE, 0x0003_0001);
        assert_eq!(desc_types::FGSWAPCHAIN_DX12_VERSION, 0x0003_000b);
        assert_eq!(desc_types::FRAMEGENERATION_CONFIGURE, 0x0002_0002);
        assert_eq!(desc_types::FRAMEGENERATION_PREPARE, 0x0002_0004);
        assert_eq!(desc_types::FRAMEGENERATION_PRESENT_CALLBACK, 0x0002_0005);
        assert_eq!(desc_types::FRAMEGENERATION_PREPARE_V2, 0x0002_000c);
        assert_eq!(desc_types::DENOISER_DISPATCH_DEBUG_VIEW, 0x0005_0042);
        assert_eq!(desc_types::DENOISER_DISPATCH_DOMINANT_LIGHT, 0x0005_0046);
        assert_eq!(
            desc_types::DENOISER_DISPATCH_SPECULAR_OCCLUSION,
            0x0005_0049
        );
        assert_eq!(desc_types::DENOISER_QUERY_DEFAULT_KEYVALUE, 0x0005_0081);
        assert_eq!(desc_types::DENOISER_QUERY_GPU_MEMORY_USAGE_2, 0x0005_0082);
        assert_eq!(desc_types::RADIANCECACHE_DEBUG_DISPATCH, 0x0006_0005);
    }

    #[test]
    fn frame_generation_swapchain_vk_chain_is_well_formed() {
        SEEN.with(|seen| seen.borrow_mut().clear());
        let module = module();

        let mut version = CreateContextDescFrameGenerationSwapChainVersionVk {
            header: FfxApiHeader::new(desc_types::FGSWAPCHAIN_VK_VERSION_DESC),
            version: versions::FRAMEGENERATION_SWAPCHAIN_VK,
        };
        let mut mode = CreateContextDescFrameGenerationSwapChainModeVk {
            header: FfxApiHeader::new(desc_types::FGSWAPCHAIN_VK_MODE_CREATE),
            compose_on_present_queue: true,
        };
        let mut create = CreateContextDescFrameGenerationSwapChainVk {
            header: FfxApiHeader::new(desc_types::FGSWAPCHAIN_VK_CREATE),
            physical_device: std::ptr::null_mut(),
            device: std::ptr::null_mut(),
            swapchain: std::ptr::null_mut(),
            allocator: std::ptr::null_mut(),
            create_info: ash::vk::SwapchainCreateInfoKHR::default(),
            game_queue: QueueInfoVk::default(),
            async_compute_queue: QueueInfoVk::default(),
            present_queue: QueueInfoVk::default(),
            image_acquire_queue: QueueInfoVk::default(),
        };
        create.header.next = std::ptr::from_mut(&mut version.header);
        version.header.next = std::ptr::from_mut(&mut mode.header);
        // Read back the links so the pinned chain is verified (and stays live).
        assert!(!create.header.next.is_null());
        assert!(!version.header.next.is_null());
        assert!(mode.header.next.is_null());

        let mut handle: Context = std::ptr::null_mut();
        assert_eq!(module.create_context(&mut handle, &mut create.header), 0);
        assert!(!handle.is_null());

        // Configure descriptors go through ffxConfigure.
        let ui = ConfigureDescFrameGenerationSwapChainRegisterUiResourceVk {
            header: FfxApiHeader::new(desc_types::FGSWAPCHAIN_VK_REGISTER_UI_RESOURCE),
            ui_resource: Resource::default(),
            flags: 0,
        };
        assert_eq!(module.configure(&mut handle, &ui.header), 0);
        let kv = ConfigureDescFrameGenerationSwapChainKeyValueVk {
            header: FfxApiHeader::new(desc_types::FGSWAPCHAIN_VK_KEYVALUE),
            key: 0,
            value: 0,
            ptr: std::ptr::null_mut(),
        };
        assert_eq!(module.configure(&mut handle, &kv.header), 0);

        // Query descriptors go through ffxQuery.
        let mut q_cmd = QueryDescFrameGenerationSwapChainInterpolationCommandListVk {
            header: FfxApiHeader::new(desc_types::FGSWAPCHAIN_VK_INTERPOLATION_COMMANDLIST),
            p_out_command_list: std::ptr::null_mut(),
        };
        assert_eq!(module.query(&mut handle, &mut q_cmd.header), 0);
        let mut q_tex = QueryDescFrameGenerationSwapChainInterpolationTextureVk {
            header: FfxApiHeader::new(desc_types::FGSWAPCHAIN_VK_INTERPOLATION_TEXTURE),
            p_out_texture: std::ptr::null_mut(),
        };
        assert_eq!(module.query(&mut handle, &mut q_tex.header), 0);
        let mut replacement = QueryDescSwapchainReplacementFunctionsVk {
            header: FfxApiHeader::new(desc_types::FGSWAPCHAIN_VK_REPLACEMENT_FUNCTIONS),
            create_swapchain: std::ptr::null_mut(),
            destroy_swapchain: std::ptr::null_mut(),
            get_swapchain_images: std::ptr::null_mut(),
            acquire_next_image: std::ptr::null_mut(),
            queue_present: std::ptr::null_mut(),
            set_hdr_metadata: std::ptr::null_mut(),
            get_last_present_count: std::ptr::null_mut(),
        };
        assert_eq!(module.query(&mut handle, &mut replacement.header), 0);

        // Wait-for-presents dispatch goes through ffxDispatch.
        let wait = DispatchDescFrameGenerationSwapChainWaitForPresentsVk {
            header: FfxApiHeader::new(desc_types::FGSWAPCHAIN_VK_WAIT_FOR_PRESENTS),
        };
        assert_eq!(module.dispatch(&mut handle, &wait.header), 0);
        assert_eq!(module.destroy_context(&mut handle), 0);

        assert_seen(desc_types::FGSWAPCHAIN_VK_CREATE);
        assert_seen(desc_types::FGSWAPCHAIN_VK_VERSION_DESC);
        assert_seen(desc_types::FGSWAPCHAIN_VK_MODE_CREATE);
        assert_seen(desc_types::FGSWAPCHAIN_VK_REGISTER_UI_RESOURCE);
        assert_seen(desc_types::FGSWAPCHAIN_VK_KEYVALUE);
        assert_seen(desc_types::FGSWAPCHAIN_VK_INTERPOLATION_COMMANDLIST);
        assert_seen(desc_types::FGSWAPCHAIN_VK_INTERPOLATION_TEXTURE);
        assert_seen(desc_types::FGSWAPCHAIN_VK_REPLACEMENT_FUNCTIONS);
        assert_seen(desc_types::FGSWAPCHAIN_VK_WAIT_FOR_PRESENTS);
    }

    #[test]
    fn frame_generation_prepare_and_present_descriptors_are_well_formed() {
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

        let prepare = DispatchDescFrameGenerationPrepare {
            header: FfxApiHeader::new(desc_types::FRAMEGENERATION_PREPARE),
            frame_id: 1,
            flags: 0,
            command_list: std::ptr::null_mut(),
            render_size: Dimensions2D::default(),
            jitter_offset: FloatCoords2D::default(),
            motion_vector_scale: FloatCoords2D::default(),
            frame_time_delta: 16.6,
            unused_reset: false,
            camera_near: 0.1,
            camera_far: 1000.0,
            camera_fov_angle_vertical: 1.0,
            view_space_to_meters_factor: 1.0,
            depth: Resource::default(),
            motion_vectors: Resource::default(),
        };
        assert_eq!(module.dispatch(&mut handle, &prepare.header), 0);

        let v2 = DispatchDescFrameGenerationPrepareV2 {
            header: FfxApiHeader::new(desc_types::FRAMEGENERATION_PREPARE_V2),
            frame_id: 2,
            flags: 0,
            command_list: std::ptr::null_mut(),
            render_size: Dimensions2D::default(),
            jitter_offset: FloatCoords2D::default(),
            motion_vector_scale: FloatCoords2D::default(),
            frame_time_delta: 16.6,
            reset: false,
            camera_near: 0.1,
            camera_far: 1000.0,
            camera_fov_angle_vertical: 1.0,
            view_space_to_meters_factor: 1.0,
            depth: Resource::default(),
            motion_vectors: Resource::default(),
            camera_position: [0.0, 0.0, 0.0],
            camera_up: [0.0, 1.0, 0.0],
            camera_right: [1.0, 0.0, 0.0],
            camera_forward: [0.0, 0.0, -1.0],
        };
        assert_eq!(module.dispatch(&mut handle, &v2.header), 0);

        let present = CallbackDescFrameGenerationPresent {
            header: FfxApiHeader::new(desc_types::FRAMEGENERATION_PRESENT_CALLBACK),
            device: std::ptr::null_mut(),
            command_list: std::ptr::null_mut(),
            current_back_buffer: Resource::default(),
            current_ui: Resource::default(),
            output_swap_chain_buffer: Resource::default(),
            is_generated_frame: true,
            frame_id: 3,
        };
        assert_eq!(module.dispatch(&mut handle, &present.header), 0);
        assert_eq!(module.destroy_context(&mut handle), 0);

        assert_seen(desc_types::FRAMEGENERATION_CREATE);
        assert_seen(desc_types::FRAMEGENERATION_PREPARE);
        assert_seen(desc_types::FRAMEGENERATION_PREPARE_V2);
        assert_seen(desc_types::FRAMEGENERATION_PRESENT_CALLBACK);
    }

    #[test]
    fn denoiser_per_signal_dispatch_descriptors_are_well_formed() {
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

        let mut debug = DispatchDescDenoiserDebugView {
            header: FfxApiHeader::new(desc_types::DENOISER_DISPATCH_DEBUG_VIEW),
            output: Resource::default(),
            output_size: Dimensions2D::default(),
            mode: 0,
            viewport_index: 0,
        };
        let mut ambient = DispatchDescDenoiserAmbientOcclusion {
            header: FfxApiHeader::new(desc_types::DENOISER_DISPATCH_AMBIENT_OCCLUSION),
            signal: DenoiserSignal::default(),
        };
        let mut direct_diffuse = DispatchDescDenoiserDirectDiffuse {
            header: FfxApiHeader::new(desc_types::DENOISER_DISPATCH_DIRECT_DIFFUSE),
            signal: DenoiserSignal::default(),
        };
        let mut direct_specular = DispatchDescDenoiserDirectSpecular {
            header: FfxApiHeader::new(desc_types::DENOISER_DISPATCH_DIRECT_SPECULAR),
            signal: DenoiserSignal::default(),
        };
        let mut dominant = DispatchDescDenoiserDominantLight {
            header: FfxApiHeader::new(desc_types::DENOISER_DISPATCH_DOMINANT_LIGHT),
            signal: DenoiserSignal::default(),
            direction: FloatCoords3D::default(),
            emission: FloatCoords3D::default(),
            angular_radius: 0.1,
        };
        let mut indirect_diffuse = DispatchDescDenoiserIndirectDiffuse {
            header: FfxApiHeader::new(desc_types::DENOISER_DISPATCH_INDIRECT_DIFFUSE),
            signal: DenoiserSignal::default(),
        };
        let mut indirect_specular = DispatchDescDenoiserIndirectSpecular {
            header: FfxApiHeader::new(desc_types::DENOISER_DISPATCH_INDIRECT_SPECULAR),
            signal: DenoiserSignal::default(),
        };
        let mut specular_occlusion = DispatchDescDenoiserSpecularOcclusion {
            header: FfxApiHeader::new(desc_types::DENOISER_DISPATCH_SPECULAR_OCCLUSION),
            signal: DenoiserSignal::default(),
        };

        // Link every per-signal descriptor behind the main denoiser dispatch.
        let empty = Resource::default();
        let mut dispatch = DispatchDescDenoiser {
            header: FfxApiHeader::new(desc_types::DENOISER_DISPATCH),
            command_list: std::ptr::null_mut(),
            linear_depth: empty,
            motion_vectors: empty,
            normals: empty,
            specular_albedo: empty,
            diffuse_albedo: empty,
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
        dispatch.header.next = std::ptr::from_mut(&mut debug.header);
        debug.header.next = std::ptr::from_mut(&mut ambient.header);
        ambient.header.next = std::ptr::from_mut(&mut direct_diffuse.header);
        direct_diffuse.header.next = std::ptr::from_mut(&mut direct_specular.header);
        direct_specular.header.next = std::ptr::from_mut(&mut dominant.header);
        dominant.header.next = std::ptr::from_mut(&mut indirect_diffuse.header);
        indirect_diffuse.header.next = std::ptr::from_mut(&mut indirect_specular.header);
        indirect_specular.header.next = std::ptr::from_mut(&mut specular_occlusion.header);
        // Read back every link so the pinned chain is verified (and stays live).
        assert!(!dispatch.header.next.is_null());
        assert!(!debug.header.next.is_null());
        assert!(!ambient.header.next.is_null());
        assert!(!direct_diffuse.header.next.is_null());
        assert!(!direct_specular.header.next.is_null());
        assert!(!dominant.header.next.is_null());
        assert!(!indirect_diffuse.header.next.is_null());
        assert!(!indirect_specular.header.next.is_null());
        assert!(specular_occlusion.header.next.is_null());

        assert_eq!(module.dispatch(&mut handle, &dispatch.header), 0);
        assert_eq!(module.destroy_context(&mut handle), 0);

        assert_seen(desc_types::DENOISER_CREATE);
        assert_seen(desc_types::DENOISER_DISPATCH);
        assert_seen(desc_types::DENOISER_DISPATCH_DEBUG_VIEW);
        assert_seen(desc_types::DENOISER_DISPATCH_AMBIENT_OCCLUSION);
        assert_seen(desc_types::DENOISER_DISPATCH_DIRECT_DIFFUSE);
        assert_seen(desc_types::DENOISER_DISPATCH_DIRECT_SPECULAR);
        assert_seen(desc_types::DENOISER_DISPATCH_DOMINANT_LIGHT);
        assert_seen(desc_types::DENOISER_DISPATCH_INDIRECT_DIFFUSE);
        assert_seen(desc_types::DENOISER_DISPATCH_INDIRECT_SPECULAR);
        assert_seen(desc_types::DENOISER_DISPATCH_SPECULAR_OCCLUSION);
    }
}
