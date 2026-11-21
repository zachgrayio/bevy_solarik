//! # D3D12 NGX (DLSS) adapter
//!
//! Feature-gated (`dx12`) D3D12 counterpart to the crate's Vulkan
//! [`initialization`](crate::initialization) / [`super_resolution`] /
//! [`ray_reconstruction`] modules.
//!
//! Everything here is built on the **already-generated NGX D3D12 bindings** in
//! `crate::nvsdk_ngx` (which come from `src/bindings/x86_64-pc-windows-msvc.rs`)
//! and the `__extern` wrappers compiled from `wrap_static_fns.c`. No function
//! signatures are invented here: every FFI call below resolves to a symbol that
//! already exists in those bindings.
//!
//! ## Scope
//! - Initializes NGX from a raw `ID3D12Device*` (`init_with_d3d12_device`).
//! - Thin wrappers for DLSS Super Resolution create/evaluate and DLSS Ray
//!   Reconstruction (DLSSD) create/evaluate, mirroring the *call shapes* of
//!   `super_resolution.rs` / `ray_reconstruction.rs`.
//! - Pure helpers (descriptor defaulting, feature ids, capability mapping) with
//!   unit tests that need no GPU, DLL or window.
//!
//! This does **not** attempt swapchain / frame generation (separate scope), and
//! it does **not** rebuild the wgpu-`TextureView` → `ID3D12Resource*` conversion:
//! the D3D12 evaluate wrappers take an already-built `*_Eval_Params` holding raw
//! `ID3D12Resource*` pointers, because extracting those from a wgpu texture
//! requires wgpu's DX12 HAL (not enabled by this crate's `wgpu` dependency) and
//! is owned by the integration layer. The create wrappers do build the create
//! params, since those need no GPU resources.
//!
//! The module is compiled only with `feature = "dx12"`; under `feature = "mock"`
//! it is empty (there are no real FFI bindings to adapt in a mock build).
#![cfg(not(feature = "mock"))]

use crate::feature_info::with_feature_info;
use crate::nvsdk_ngx::*;
use std::ffi::{c_int, c_void};
use std::ptr;
use uuid::Uuid;

/// Opaque raw `ID3D12Device*`. NGX receives the COM pointer as `void*`, so it
/// is carried as `*mut c_void` to avoid a hard dependency on a windows-sys crate.
pub type D3D12DevicePtr = *mut c_void;

/// The NGX graphics-API id for D3D12 (`NVSDK_NGX_GRAPHICS_API_D3D12`).
pub const D3D12_GRAPHICS_API: NVSDK_NGX_GraphicsAPI =
    NVSDK_NGX_GraphicsAPI_NVSDK_NGX_GRAPHICS_API_D3D12;

/// Which NGX DLSS feature a D3D12 context is created/evaluated for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dx12Feature {
    /// DLSS Super Resolution.
    SuperResolution,
    /// DLSS Ray Reconstruction (DLSSD).
    RayReconstruction,
}

impl Dx12Feature {
    /// The `NVSDK_NGX_Feature` id passed to NGX for this feature.
    pub fn feature_id(self) -> NVSDK_NGX_Feature {
        match self {
            Self::SuperResolution => NVSDK_NGX_Feature_NVSDK_NGX_Feature_SuperSampling,
            Self::RayReconstruction => NVSDK_NGX_Feature_NVSDK_NGX_Feature_RayReconstruction,
        }
    }

    /// The NUL-terminated NGX parameter name that reports this feature's
    /// availability (queried via `NVSDK_NGX_Parameter_GetI`).
    pub fn capability_parameter(self) -> &'static [u8] {
        match self {
            Self::SuperResolution => &NVSDK_NGX_Parameter_SuperSampling_Available[..],
            Self::RayReconstruction => &NVSDK_NGX_Parameter_SuperSamplingDenoising_Available[..],
        }
    }
}

/// `(creation_node_mask, visibility_node_mask)` for a single-adapter D3D12
/// device — the values NGX feature-creation calls use for the common case.
pub fn single_gpu_node_masks() -> (u32, u32) {
    (1, 1)
}

/// Interpret an NGX capability integer: `0` means unavailable, any non-zero
/// value means the feature is available.
pub fn capability_to_support(value: c_int) -> bool {
    value != 0
}

/// Aggregated DLSS feature support derived from two NGX capability integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dx12FeatureSupport {
    /// DLSS Super Resolution is available.
    pub super_resolution: bool,
    /// DLSS Ray Reconstruction is available.
    pub ray_reconstruction: bool,
}

impl Dx12FeatureSupport {
    /// Map the SR and RR `NVSDK_NGX_Parameter_GetI` results to support flags.
    pub fn from_capabilities(super_resolution: c_int, ray_reconstruction: c_int) -> Self {
        Self {
            super_resolution: capability_to_support(super_resolution),
            ray_reconstruction: capability_to_support(ray_reconstruction),
        }
    }
}

/// Resolution / flag descriptor used to build NGX D3D12 create params.
///
/// Kept as plain Rust data (not an NGX struct) so its defaults can be unit
/// tested without initializing the SDK. Mirror of the create-param construction
/// in `super_resolution.rs` / `ray_reconstruction.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dx12FeatureDescriptor {
    /// Render (input) width.
    pub width: u32,
    /// Render (input) height.
    pub height: u32,
    /// Upscaled (output/target) width.
    pub target_width: u32,
    /// Upscaled (output/target) height.
    pub target_height: u32,
    /// NGX perf-quality value.
    pub perf_quality_value: NVSDK_NGX_PerfQuality_Value,
    /// `NVSDK_NGX_DLSS_Feature_Flags` forwarded to the feature.
    pub feature_create_flags: c_int,
    /// D3D12 creation node mask.
    pub creation_node_mask: u32,
    /// D3D12 visibility node mask.
    pub visibility_node_mask: u32,
    /// Whether output subrects are enabled.
    pub enable_output_subrects: bool,
}

impl Default for Dx12FeatureDescriptor {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            target_width: 0,
            target_height: 0,
            perf_quality_value: NVSDK_NGX_PerfQuality_Value_NVSDK_NGX_PerfQuality_Value_MaxQuality,
            feature_create_flags: 0,
            creation_node_mask: 1,
            visibility_node_mask: 1,
            enable_output_subrects: false,
        }
    }
}

impl Dx12FeatureDescriptor {
    /// Build `NVSDK_NGX_DLSS_Create_Params` for DLSS Super Resolution.
    pub fn to_dlss_create_params(&self) -> NVSDK_NGX_DLSS_Create_Params {
        NVSDK_NGX_DLSS_Create_Params {
            Feature: NVSDK_NGX_Feature_Create_Params {
                InWidth: self.width,
                InHeight: self.height,
                InTargetWidth: self.target_width,
                InTargetHeight: self.target_height,
                InPerfQualityValue: self.perf_quality_value,
            },
            InFeatureCreateFlags: self.feature_create_flags,
            InEnableOutputSubrects: self.enable_output_subrects,
        }
    }

    /// Build `NVSDK_NGX_DLSSD_Create_Params` for DLSS Ray Reconstruction.
    ///
    /// Uses the same fixed denoise (DLUnified), roughness (Unpacked) and depth
    /// (Linear) defaults as `ray_reconstruction.rs`'s hardware-independent path.
    pub fn to_dlssd_create_params(&self) -> NVSDK_NGX_DLSSD_Create_Params {
        NVSDK_NGX_DLSSD_Create_Params {
            InDenoiseMode: NVSDK_NGX_DLSS_Denoise_Mode_NVSDK_NGX_DLSS_Denoise_Mode_DLUnified,
            InRoughnessMode: NVSDK_NGX_DLSS_Roughness_Mode_NVSDK_NGX_DLSS_Roughness_Mode_Unpacked,
            InUseHWDepth: NVSDK_NGX_DLSS_Depth_Type_NVSDK_NGX_DLSS_Depth_Type_Linear,
            InWidth: self.width,
            InHeight: self.height,
            InTargetWidth: self.target_width,
            InTargetHeight: self.target_height,
            InPerfQualityValue: self.perf_quality_value,
            InFeatureCreateFlags: self.feature_create_flags,
            InEnableOutputSubrects: self.enable_output_subrects,
        }
    }
}

/// Application-wide NGX D3D12 state created by [`init_with_d3d12_device`].
///
/// Owns the capability `NVSDK_NGX_Parameter*` handle and shuts NGX down on drop.
pub struct D3D12Sdk {
    /// Raw capability-parameter handle returned by NGX.
    pub parameters: *mut NVSDK_NGX_Parameter,
    /// The raw `ID3D12Device*` NGX was initialized with (kept for shutdown).
    pub device: D3D12DevicePtr,
}

impl D3D12Sdk {
    /// Query whether `feature` is available on the current system.
    pub fn feature_supported(&self, feature: Dx12Feature) -> Result<bool, DlssError> {
        let mut value: c_int = 0;
        // SAFETY: `parameters` comes from GetCapabilityParameters and stays
        // alive for this D3D12Sdk; `value` is a valid out-param.
        unsafe {
            check_ngx_result(NVSDK_NGX_Parameter_GetI(
                self.parameters,
                feature.capability_parameter().as_ptr().cast(),
                &mut value,
            ))?;
        }
        Ok(capability_to_support(value))
    }
}

impl Drop for D3D12Sdk {
    fn drop(&mut self) {
        // SAFETY: both handles come from a successful NGX D3D12 init and are
        // destroyed exactly once, in the correct order (params then device).
        unsafe {
            let _ = check_ngx_result(NVSDK_NGX_D3D12_DestroyParameters(self.parameters));
            let _ = check_ngx_result(NVSDK_NGX_D3D12_Shutdown1(
                self.device.cast::<ID3D12Device>(),
            ));
        }
    }
}

/// Initialize NGX for D3D12 with a raw `ID3D12Device*`.
///
/// This is the D3D12 counterpart of the Vulkan `DlssSdk::new` init shape: it
/// calls `NVSDK_NGX_D3D12_Init_with_ProjectID` and then
/// `NVSDK_NGX_D3D12_GetCapabilityParameters`, returning the capability handle.
///
/// # Errors
/// Propagates any NGX failure via [`check_ngx_result`].
pub fn init_with_d3d12_device(
    project_id: Uuid,
    device: D3D12DevicePtr,
) -> Result<D3D12Sdk, DlssError> {
    with_feature_info(project_id, Default::default(), |feature_info| {
        // SAFETY: `device` is a live ID3D12Device* supplied by the caller; the
        // strings and feature-info pointers come from with_feature_info and
        // outlive this call. Reading the ProjectId union field is safe here.
        unsafe {
            check_ngx_result(NVSDK_NGX_D3D12_Init_with_ProjectID(
                feature_info.Identifier.v.ProjectDesc.ProjectId,
                NVSDK_NGX_EngineType_NVSDK_NGX_ENGINE_TYPE_CUSTOM,
                feature_info.Identifier.v.ProjectDesc.EngineVersion,
                feature_info.ApplicationDataPath,
                device.cast::<ID3D12Device>(),
                feature_info.FeatureInfo,
                NVSDK_NGX_Version_NVSDK_NGX_Version_API,
            ))
        }
    })?;

    let mut parameters = ptr::null_mut();
    // SAFETY: NGX has been initialized above; `parameters` is a valid out-param.
    unsafe {
        check_ngx_result(NVSDK_NGX_D3D12_GetCapabilityParameters(&mut parameters))?;
    }
    Ok(D3D12Sdk { parameters, device })
}

/// Thin wrapper over `NGX_D3D12_CREATE_DLSS_EXT` (DLSS Super Resolution).
///
/// Builds `NVSDK_NGX_DLSS_Create_Params` from `descriptor` and returns the
/// created NGX feature handle.
///
/// # Safety
/// - `command_list` must be a live `ID3D12GraphicsCommandList*` opened for recording.
/// - `parameters` must come from [`init_with_d3d12_device`] and stay alive.
pub unsafe fn create_dlss_sr(
    command_list: *mut ID3D12GraphicsCommandList,
    descriptor: &Dx12FeatureDescriptor,
    parameters: *mut NVSDK_NGX_Parameter,
) -> Result<*mut NVSDK_NGX_Handle, DlssError> {
    let mut create_params = descriptor.to_dlss_create_params();
    let mut handle = ptr::null_mut();
    // SAFETY: all pointers are forwarded straight to NGX; the caller guarantees
    // they are live and correctly typed, and create_params is a valid local.
    unsafe {
        check_ngx_result(NGX_D3D12_CREATE_DLSS_EXT(
            command_list,
            descriptor.creation_node_mask,
            descriptor.visibility_node_mask,
            &mut handle,
            parameters,
            &mut create_params,
        ))?;
    }
    Ok(handle)
}

/// Thin wrapper over `NGX_D3D12_EVALUATE_DLSS_EXT` (DLSS Super Resolution).
///
/// # Safety
/// - Every pointer must be live and correctly typed for the frame being evaluated.
pub unsafe fn evaluate_dlss_sr(
    command_list: *mut ID3D12GraphicsCommandList,
    handle: *mut NVSDK_NGX_Handle,
    parameters: *mut NVSDK_NGX_Parameter,
    eval_params: *mut NVSDK_NGX_D3D12_DLSS_Eval_Params,
) -> Result<(), DlssError> {
    // SAFETY: forwarded straight to NGX; caller guarantees pointer validity.
    unsafe {
        check_ngx_result(NGX_D3D12_EVALUATE_DLSS_EXT(
            command_list,
            handle,
            parameters,
            eval_params,
        ))
    }
}

/// Thin wrapper over `NGX_D3D12_CREATE_DLSSD_EXT` (DLSS Ray Reconstruction).
///
/// Builds `NVSDK_NGX_DLSSD_Create_Params` from `descriptor` and returns the
/// created NGX feature handle.
///
/// # Safety
/// - `command_list` must be a live `ID3D12GraphicsCommandList*` opened for recording.
/// - `parameters` must come from [`init_with_d3d12_device`] and stay alive.
pub unsafe fn create_dlss_rr(
    command_list: *mut ID3D12GraphicsCommandList,
    descriptor: &Dx12FeatureDescriptor,
    parameters: *mut NVSDK_NGX_Parameter,
) -> Result<*mut NVSDK_NGX_Handle, DlssError> {
    let mut create_params = descriptor.to_dlssd_create_params();
    let mut handle = ptr::null_mut();
    // SAFETY: all pointers are forwarded straight to NGX; the caller guarantees
    // they are live and correctly typed, and create_params is a valid local.
    unsafe {
        check_ngx_result(NGX_D3D12_CREATE_DLSSD_EXT(
            command_list,
            descriptor.creation_node_mask,
            descriptor.visibility_node_mask,
            &mut handle,
            parameters,
            &mut create_params,
        ))?;
    }
    Ok(handle)
}

/// Thin wrapper over `NGX_D3D12_EVALUATE_DLSSD_EXT` (DLSS Ray Reconstruction).
///
/// # Safety
/// - Every pointer must be live and correctly typed for the frame being evaluated.
pub unsafe fn evaluate_dlss_rr(
    command_list: *mut ID3D12GraphicsCommandList,
    handle: *mut NVSDK_NGX_Handle,
    parameters: *mut NVSDK_NGX_Parameter,
    eval_params: *mut NVSDK_NGX_D3D12_DLSSD_Eval_Params,
) -> Result<(), DlssError> {
    // SAFETY: forwarded straight to NGX; caller guarantees pointer validity.
    unsafe {
        check_ngx_result(NGX_D3D12_EVALUATE_DLSSD_EXT(
            command_list,
            handle,
            parameters,
            eval_params,
        ))
    }
}

/// Thin wrapper over `NVSDK_NGX_D3D12_ReleaseFeature`.
///
/// # Safety
/// - `handle` must be a live feature handle returned by a create wrapper and
///   must not be released twice.
pub unsafe fn release_feature(handle: *mut NVSDK_NGX_Handle) -> Result<(), DlssError> {
    // SAFETY: forwarded straight to NGX; caller guarantees the handle is valid.
    unsafe { check_ngx_result(NVSDK_NGX_D3D12_ReleaseFeature(handle)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_ids_match_ngx() {
        assert_eq!(
            Dx12Feature::SuperResolution.feature_id(),
            NVSDK_NGX_Feature_NVSDK_NGX_Feature_SuperSampling
        );
        assert_eq!(
            Dx12Feature::RayReconstruction.feature_id(),
            NVSDK_NGX_Feature_NVSDK_NGX_Feature_RayReconstruction
        );
        // Documented NGX feature ids: 1 = SuperSampling, 13 = RayReconstruction.
        assert_eq!(NVSDK_NGX_Feature_NVSDK_NGX_Feature_SuperSampling, 1);
        assert_eq!(NVSDK_NGX_Feature_NVSDK_NGX_Feature_RayReconstruction, 13);
    }

    #[test]
    fn graphics_api_is_d3d12() {
        assert_eq!(
            D3D12_GRAPHICS_API,
            NVSDK_NGX_GraphicsAPI_NVSDK_NGX_GRAPHICS_API_D3D12
        );
        assert_eq!(D3D12_GRAPHICS_API, 2);
    }

    #[test]
    fn capability_parameter_names_are_c_strings() {
        for feature in [Dx12Feature::SuperResolution, Dx12Feature::RayReconstruction] {
            let name = feature.capability_parameter();
            assert!(!name.is_empty());
            assert_eq!(name.last(), Some(&0));
        }
        assert_eq!(
            Dx12Feature::SuperResolution.capability_parameter(),
            &NVSDK_NGX_Parameter_SuperSampling_Available[..]
        );
        assert_eq!(
            Dx12Feature::RayReconstruction.capability_parameter(),
            &NVSDK_NGX_Parameter_SuperSamplingDenoising_Available[..]
        );
    }

    #[test]
    fn capability_mapping() {
        assert!(!capability_to_support(0));
        assert!(capability_to_support(1));
        assert!(capability_to_support(-1));
        let support = Dx12FeatureSupport::from_capabilities(1, 0);
        assert!(support.super_resolution);
        assert!(!support.ray_reconstruction);
        assert_eq!(
            Dx12FeatureSupport::from_capabilities(0, 2),
            Dx12FeatureSupport {
                super_resolution: false,
                ray_reconstruction: true,
            }
        );
    }

    #[test]
    fn descriptor_defaults() {
        let d = Dx12FeatureDescriptor::default();
        assert_eq!(
            (d.creation_node_mask, d.visibility_node_mask),
            single_gpu_node_masks()
        );
        assert_eq!(
            d.perf_quality_value,
            NVSDK_NGX_PerfQuality_Value_NVSDK_NGX_PerfQuality_Value_MaxQuality
        );
        assert_eq!(d.feature_create_flags, 0);
        assert!(!d.enable_output_subrects);
        assert_eq!(
            (d.width, d.height, d.target_width, d.target_height),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn descriptor_builds_create_params() {
        let mut d = Dx12FeatureDescriptor::default();
        d.width = 1280;
        d.height = 720;
        d.target_width = 2560;
        d.target_height = 1440;

        let sr = d.to_dlss_create_params();
        assert_eq!(sr.Feature.InWidth, 1280);
        assert_eq!(sr.Feature.InHeight, 720);
        assert_eq!(sr.Feature.InTargetWidth, 2560);
        assert_eq!(sr.Feature.InTargetHeight, 1440);
        assert_eq!(sr.InFeatureCreateFlags, 0);
        assert!(!sr.InEnableOutputSubrects);

        let rr = d.to_dlssd_create_params();
        assert_eq!(rr.InWidth, 1280);
        assert_eq!(rr.InTargetHeight, 1440);
        assert_eq!(
            rr.InDenoiseMode,
            NVSDK_NGX_DLSS_Denoise_Mode_NVSDK_NGX_DLSS_Denoise_Mode_DLUnified
        );
        assert_eq!(
            rr.InRoughnessMode,
            NVSDK_NGX_DLSS_Roughness_Mode_NVSDK_NGX_DLSS_Roughness_Mode_Unpacked
        );
        assert_eq!(
            rr.InUseHWDepth,
            NVSDK_NGX_DLSS_Depth_Type_NVSDK_NGX_DLSS_Depth_Type_Linear
        );
    }
}
