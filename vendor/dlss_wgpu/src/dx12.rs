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
//! - A pure, headless builder for the SR / RR `*_Eval_Params`:
//!   [`Dx12EvalParams::to_sr_eval_params`] / [`Dx12EvalParams::to_rr_eval_params`]
//!   turn plain-Rust runtime data + opaque resource pointers into the exact NGX
//!   structs the `evaluate_dlss_sr` / `evaluate_dlss_rr` wrappers consume.
//!
//! This does **not** attempt swapchain / frame generation (separate scope), and
//! it does **not** rebuild the wgpu-`TextureView` → `ID3D12Resource*` conversion
//! (that needs wgpu's DX12 HAL and belongs to the integration layer): the eval
//! builders take opaque `ID3D12Resource*` values and are exhaustively unit
//! tested — closing the CPU/FFI-data half of the D3D12 SR/RR render-dispatch
//! path. Only the raw-pointer extraction from wgpu textures and the render-graph
//! node that feeds a live command list remain GPU-runtime work.
//!
//! The module is compiled only with `feature = "dx12"`; under `feature = "mock"`
//! it is empty (there are no real FFI bindings to adapt in a mock build).
#![cfg(not(feature = "mock"))]

use crate::feature_info::with_feature_info;
use crate::nvsdk_ngx::*;
use std::ffi::{c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

static D3D12_NGX_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Whether [`init_with_d3d12_device`] has already initialized NGX on D3D12 in
/// this process (so `DlssSdk::new` can obtain capability parameters without
/// calling `NVSDK_NGX_D3D12_Init_with_ProjectID` or `Shutdown1` twice).
pub fn is_d3d12_ngx_initialized() -> bool {
    D3D12_NGX_INITIALIZED.load(Ordering::Acquire)
}

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

/// Opaque raw `ID3D12Resource*`. NGX receives COM pointers as `void*`, so this
/// stays a plain `void*` to avoid a windows-sys dependency in this crate. The
/// integration layer obtains these from wgpu's D3D12 HAL
/// (`texture.as_hal::<Dx12>()` -> `raw_resource()`), cast to `void*`; see the
/// `TextureView -> ID3D12Resource*` seam documented in the module header.
pub type D3D12ResourcePtr = *mut c_void;

/// The D3D12 resources consumed by one DLSS SR / RR evaluation. The four
/// "required" resources drive the core algorithm; the rest are optional
/// guide / mask inputs (null when unused). Pure pointer values — never
/// dereferenced on CPU — so this is fully headless and testable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dx12EvalResources {
    /// Input color (required for SR + RR).
    pub color: D3D12ResourcePtr,
    /// Upscaled output (required).
    pub output: D3D12ResourcePtr,
    /// Linear depth (required).
    pub depth: D3D12ResourcePtr,
    /// Motion vectors (required).
    pub motion_vectors: D3D12ResourcePtr,
    /// Exposure texture (optional).
    pub exposure: D3D12ResourcePtr,
    /// Transparency mask (optional).
    pub transparency_mask: D3D12ResourcePtr,
    /// Bias / current-color mask (optional).
    pub bias_current_color_mask: D3D12ResourcePtr,
    /// Diffuse albedo guide (RR only, optional-for-SR).
    pub diffuse_albedo: D3D12ResourcePtr,
    /// Specular albedo guide (RR only).
    pub specular_albedo: D3D12ResourcePtr,
    /// Normals guide (RR only).
    pub normals: D3D12ResourcePtr,
    /// Roughness guide (RR only).
    pub roughness: D3D12ResourcePtr,
}

impl Default for Dx12EvalResources {
    fn default() -> Self {
        Self {
            color: ptr::null_mut(),
            output: ptr::null_mut(),
            depth: ptr::null_mut(),
            motion_vectors: ptr::null_mut(),
            exposure: ptr::null_mut(),
            transparency_mask: ptr::null_mut(),
            bias_current_color_mask: ptr::null_mut(),
            diffuse_albedo: ptr::null_mut(),
            specular_albedo: ptr::null_mut(),
            normals: ptr::null_mut(),
            roughness: ptr::null_mut(),
        }
    }
}

impl Dx12EvalResources {
    fn as_ngx(p: D3D12ResourcePtr) -> *mut ID3D12Resource {
        p.cast()
    }

    /// The four required resources must be present for any evaluate call.
    pub fn validate(&self) -> Result<(), Dx12EvalError> {
        if self.color.is_null() {
            return Err(Dx12EvalError::MissingColor);
        }
        if self.output.is_null() {
            return Err(Dx12EvalError::MissingOutput);
        }
        if self.depth.is_null() {
            return Err(Dx12EvalError::MissingDepth);
        }
        if self.motion_vectors.is_null() {
            return Err(Dx12EvalError::MissingMotionVectors);
        }
        Ok(())
    }
}

/// Per-frame parameters for one D3D12 DLSS SR / RR evaluation. Plain Rust data;
/// mirrors the field set that `super_resolution.rs` builds for the Vulkan path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dx12EvalParams {
    /// Render resources for this frame.
    pub resources: Dx12EvalResources,
    /// Input (render) width / height of the subrect being upscaled.
    pub render_width: u32,
    /// Input (render) height.
    pub render_height: u32,
    /// Halton jitter offset `[x, y]`.
    pub jitter: [f32; 2],
    /// Motion-vector scale `[x, y]`; `[1.0, 1.0]` when vectors are in pixels.
    pub motion_vector_scale: [f32; 2],
    /// DLSS sharpness (SR only).
    pub sharpness: f32,
    /// Reset temporal history (e.g. after a camera cut).
    pub reset: bool,
    /// Pre-exposure applied to color.
    pub pre_exposure: f32,
    /// Exposure scale.
    pub exposure_scale: f32,
    /// Frame time in milliseconds (for NGX pacing).
    pub frame_time_ms: f32,
}

impl Default for Dx12EvalParams {
    fn default() -> Self {
        Self {
            resources: Dx12EvalResources::default(),
            render_width: 0,
            render_height: 0,
            jitter: [0.0, 0.0],
            motion_vector_scale: [1.0, 1.0],
            sharpness: 0.0,
            reset: false,
            pre_exposure: 1.0,
            exposure_scale: 1.0,
            frame_time_ms: 0.0,
        }
    }
}

/// Why a D3D12 DLSS evaluation cannot be built. Pure CPU validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dx12EvalError {
    /// Input color resource is null.
    MissingColor,
    /// Output resource is null.
    MissingOutput,
    /// Depth resource is null.
    MissingDepth,
    /// Motion-vector resource is null.
    MissingMotionVectors,
    /// Render subrect dimensions are zero.
    ZeroRenderSize,
}

impl core::fmt::Display for Dx12EvalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let message = match self {
            Self::MissingColor => "missing input color resource",
            Self::MissingOutput => "missing output resource",
            Self::MissingDepth => "missing depth resource",
            Self::MissingMotionVectors => "missing motion-vector resource",
            Self::ZeroRenderSize => "zero render subrect dimensions",
        };
        f.write_str(message)
    }
}

impl Dx12EvalParams {
    /// Validate resources + dimensions before building the NGX struct.
    pub fn validate(&self) -> Result<(), Dx12EvalError> {
        self.resources.validate()?;
        if self.render_width == 0 || self.render_height == 0 {
            return Err(Dx12EvalError::ZeroRenderSize);
        }
        Ok(())
    }

    fn zero_coords() -> NVSDK_NGX_Coordinates {
        NVSDK_NGX_Coordinates { X: 0, Y: 0 }
    }

    fn dims(&self) -> NVSDK_NGX_Dimensions {
        NVSDK_NGX_Dimensions {
            Width: self.render_width,
            Height: self.render_height,
        }
    }

    /// Build `NVSDK_NGX_D3D12_DLSS_Eval_Params` (DLSS Super Resolution).
    ///
    /// Explicit full-field literal — same construction as the proven Vulkan
    /// `super_resolution.rs::render` path. Optional inputs are null / zero, and
    /// every required field is forwarded from `self`. Fully CPU-verifiable;
    /// returns a validation error rather than ever emitting an invalid struct.
    ///
    /// # Errors
    /// Propagates [`Dx12EvalError`] if a required resource or size is missing.
    pub fn to_sr_eval_params(&self) -> Result<NVSDK_NGX_D3D12_DLSS_Eval_Params, Dx12EvalError> {
        self.validate()?;
        let r = &self.resources;
        let z = ptr::null_mut::<ID3D12Resource>();
        Ok(NVSDK_NGX_D3D12_DLSS_Eval_Params {
            Feature: NVSDK_NGX_D3D12_Feature_Eval_Params {
                pInColor: Dx12EvalResources::as_ngx(r.color),
                pInOutput: Dx12EvalResources::as_ngx(r.output),
                InSharpness: self.sharpness,
            },
            pInDepth: Dx12EvalResources::as_ngx(r.depth),
            pInMotionVectors: Dx12EvalResources::as_ngx(r.motion_vectors),
            InJitterOffsetX: self.jitter[0],
            InJitterOffsetY: self.jitter[1],
            InRenderSubrectDimensions: self.dims(),
            InReset: c_int::from(self.reset),
            InMVScaleX: self.motion_vector_scale[0],
            InMVScaleY: self.motion_vector_scale[1],
            pInTransparencyMask: if r.transparency_mask.is_null() {
                z
            } else {
                Dx12EvalResources::as_ngx(r.transparency_mask)
            },
            pInExposureTexture: Dx12EvalResources::as_ngx(r.exposure),
            pInBiasCurrentColorMask: Dx12EvalResources::as_ngx(r.bias_current_color_mask),
            InColorSubrectBase: Self::zero_coords(),
            InDepthSubrectBase: Self::zero_coords(),
            InMVSubrectBase: Self::zero_coords(),
            InTranslucencySubrectBase: Self::zero_coords(),
            InBiasCurrentColorSubrectBase: Self::zero_coords(),
            InOutputSubrectBase: Self::zero_coords(),
            InPreExposure: self.pre_exposure,
            InExposureScale: self.exposure_scale,
            InIndicatorInvertXAxis: 0,
            InIndicatorInvertYAxis: 0,
            GBufferSurface: NVSDK_NGX_D3D12_GBuffer { pInAttrib: [z; 17] },
            InToneMapperType: NVSDK_NGX_ToneMapperType_NVSDK_NGX_TONEMAPPER_STRING,
            pInMotionVectors3D: z,
            pInIsParticleMask: z,
            pInAnimatedTextureMask: z,
            pInDepthHighRes: z,
            pInPositionViewSpace: z,
            InFrameTimeDeltaInMsec: self.frame_time_ms,
            pInRayTracingHitDistance: z,
            pInMotionVectorsReflections: z,
        })
    }

    /// Build `NVSDK_NGX_D3D12_DLSSD_Eval_Params` (DLSS Ray Reconstruction).
    ///
    /// The DLSSD struct carries ~90 fields, most of them optional guide /
    /// G-buffer surfaces. Start from a zeroed value (all optional pointers
    /// null, scalars zero), then set the required inputs and the same common
    /// parameters as the SR builder. Fully CPU-verifiable.
    ///
    /// # Errors
    /// Propagates [`Dx12EvalError`] if a required resource or size is missing.
    pub fn to_rr_eval_params(&self) -> Result<NVSDK_NGX_D3D12_DLSSD_Eval_Params, Dx12EvalError> {
        self.validate()?;
        // SAFETY: the DLSSD struct is plain `#[repr(C)]` data — raw pointers,
        // integer / float scalars, and fixed pointer arrays. The all-zero bit
        // pattern is valid (null pointers, 0.0, 0); no Rust invariants, no
        // references, no enums with a non-zero validity requirement are present.
        let mut p: NVSDK_NGX_D3D12_DLSSD_Eval_Params = unsafe { core::mem::zeroed() };
        let r = &self.resources;
        p.pInColor = Dx12EvalResources::as_ngx(r.color);
        p.pInOutput = Dx12EvalResources::as_ngx(r.output);
        p.pInDepth = Dx12EvalResources::as_ngx(r.depth);
        p.pInMotionVectors = Dx12EvalResources::as_ngx(r.motion_vectors);
        p.pInDiffuseAlbedo = Dx12EvalResources::as_ngx(r.diffuse_albedo);
        p.pInSpecularAlbedo = Dx12EvalResources::as_ngx(r.specular_albedo);
        p.pInNormals = Dx12EvalResources::as_ngx(r.normals);
        p.pInRoughness = Dx12EvalResources::as_ngx(r.roughness);
        p.pInExposureTexture = Dx12EvalResources::as_ngx(r.exposure);
        p.pInTransparencyMask = Dx12EvalResources::as_ngx(r.transparency_mask);
        p.pInBiasCurrentColorMask = Dx12EvalResources::as_ngx(r.bias_current_color_mask);
        p.InJitterOffsetX = self.jitter[0];
        p.InJitterOffsetY = self.jitter[1];
        p.InRenderSubrectDimensions = self.dims();
        p.InReset = c_int::from(self.reset);
        p.InMVScaleX = self.motion_vector_scale[0];
        p.InMVScaleY = self.motion_vector_scale[1];
        p.InPreExposure = self.pre_exposure;
        p.InExposureScale = self.exposure_scale;
        p.InFrameTimeDeltaInMsec = self.frame_time_ms;
        Ok(p)
    }
}

#[cfg(windows)]
#[repr(C)]
struct UnknownVtable {
    query_interface: *const c_void,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}
#[cfg(windows)]
struct OwnedDeviceRef(D3D12DevicePtr);
#[cfg(windows)]
impl OwnedDeviceRef {
    unsafe fn retain(device: D3D12DevicePtr) -> Self {
        // SAFETY: caller supplies a live ID3D12Device; all COM interfaces begin with the
        // documented IUnknown QueryInterface/AddRef/Release vtable prefix.
        unsafe {
            let table = *device.cast::<*const UnknownVtable>();
            ((*table).add_ref)(device);
        }
        Self(device)
    }
}
#[cfg(windows)]
impl Drop for OwnedDeviceRef {
    fn drop(&mut self) {
        // SAFETY: this owns exactly the COM reference acquired by retain; SDK shutdown has
        // already completed before the reference field is dropped.
        unsafe {
            let table = *self.0.cast::<*const UnknownVtable>();
            ((*table).release)(self.0);
        }
    }
}

/// Application-wide NGX D3D12 state. Owns capability parameters and a native COM device
/// reference, including initialization/error paths before a wgpu device can be retained.
pub struct D3D12Sdk {
    /// Raw capability-parameter handle returned by NGX.
    pub parameters: *mut NVSDK_NGX_Parameter,
    /// The raw `ID3D12Device*` NGX was initialized with (kept for shutdown).
    pub device: D3D12DevicePtr,
    #[cfg(windows)]
    _device_ref: OwnedDeviceRef,
}

impl D3D12Sdk {
    /// Release capability parameters before another provider shuts down shared NGX state.
    /// The SDK and its owned device remain alive until normal Drop calls Shutdown1.
    ///
    /// # Safety
    /// All feature and operation clients of these parameters must have finished.
    pub unsafe fn release_parameters(&mut self) -> Result<(), DlssError> {
        let parameters = std::mem::replace(&mut self.parameters, ptr::null_mut());
        if parameters.is_null() {
            return Ok(());
        }
        // SAFETY: caller has ended every client and this takes parameter ownership once.
        unsafe { check_ngx_result(NVSDK_NGX_D3D12_DestroyParameters(parameters)) }
    }

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
            let _ = self.release_parameters();
            let _ = check_ngx_result(NVSDK_NGX_D3D12_Shutdown1(
                self.device.cast::<ID3D12Device>(),
            ));
        }
        D3D12_NGX_INITIALIZED.store(false, Ordering::Release);
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
///
/// # Safety
/// `device` must be a live ID3D12Device COM interface. On Windows the returned SDK acquires
/// its own reference, so the caller's initial reference may be released afterwards.
pub unsafe fn init_with_d3d12_device(
    project_id: Uuid,
    device: D3D12DevicePtr,
) -> Result<D3D12Sdk, DlssError> {
    #[cfg(windows)]
    // SAFETY: guaranteed by this function's live-COM-device precondition.
    let device_ref = unsafe { OwnedDeviceRef::retain(device) };
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
        if let Err(error) =
            check_ngx_result(NVSDK_NGX_D3D12_GetCapabilityParameters(&mut parameters))
        {
            let _ = NVSDK_NGX_D3D12_Shutdown1(device.cast::<ID3D12Device>());
            return Err(error);
        }
    }
    D3D12_NGX_INITIALIZED.store(true, Ordering::Release);
    Ok(D3D12Sdk {
        parameters,
        device,
        #[cfg(windows)]
        _device_ref: device_ref,
    })
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

    #[cfg(windows)]
    #[test]
    fn sdk_device_reference_retains_and_releases_exactly_one_com_reference() {
        use std::sync::atomic::{AtomicU32, Ordering};
        #[repr(C)]
        struct Fake {
            table: *const UnknownVtable,
            references: AtomicU32,
        }
        unsafe extern "system" fn add(device: *mut c_void) -> u32 {
            // SAFETY: this test supplies its live Fake interface to these IUnknown stubs.
            unsafe {
                (*device.cast::<Fake>())
                    .references
                    .fetch_add(1, Ordering::SeqCst)
                    + 1
            }
        }
        unsafe extern "system" fn release(device: *mut c_void) -> u32 {
            // SAFETY: paired with add for the same test-owned interface; no object is freed.
            unsafe {
                (*device.cast::<Fake>())
                    .references
                    .fetch_sub(1, Ordering::SeqCst)
                    - 1
            }
        }
        let table = UnknownVtable {
            query_interface: ptr::null(),
            add_ref: add,
            release,
        };
        let device = Fake {
            table: &table,
            references: AtomicU32::new(1),
        };
        // SAFETY: the interface has a valid live IUnknown prefix for the whole owned reference.
        let retained =
            unsafe { OwnedDeviceRef::retain((&device as *const Fake).cast_mut().cast()) };
        assert_eq!(device.references.load(Ordering::SeqCst), 2);
        drop(retained);
        assert_eq!(device.references.load(Ordering::SeqCst), 1);
    }

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

    // ---- D3D12 eval-params builder (CPU-only; no GPU, DLL, window or FFI) ----

    /// Distinct non-null opaque resource handles. Real integration obtains these
    /// from wgpu's D3D12 HAL; here they are just integers, never dereferenced.
    fn handles() -> Dx12EvalResources {
        Dx12EvalResources {
            color: 0x1111_usize as D3D12ResourcePtr,
            output: 0x2222_usize as D3D12ResourcePtr,
            depth: 0x3333_usize as D3D12ResourcePtr,
            motion_vectors: 0x4444_usize as D3D12ResourcePtr,
            exposure: 0x5555_usize as D3D12ResourcePtr,
            transparency_mask: 0x6666_usize as D3D12ResourcePtr,
            bias_current_color_mask: 0x7777_usize as D3D12ResourcePtr,
            diffuse_albedo: 0x8888_usize as D3D12ResourcePtr,
            specular_albedo: 0x9999_usize as D3D12ResourcePtr,
            normals: 0xaaaa_usize as D3D12ResourcePtr,
            roughness: 0xbbbb_usize as D3D12ResourcePtr,
        }
    }

    fn eval_params() -> Dx12EvalParams {
        Dx12EvalParams {
            resources: handles(),
            render_width: 1280,
            render_height: 720,
            jitter: [-0.25, 0.5],
            motion_vector_scale: [2.0, 3.0],
            sharpness: 0.75,
            reset: true,
            pre_exposure: 1.5,
            exposure_scale: 0.5,
            frame_time_ms: 16.667,
        }
    }

    #[test]
    fn eval_params_defaults_are_sane() {
        let p = Dx12EvalParams::default();
        assert_eq!(p.motion_vector_scale, [1.0, 1.0]);
        assert_eq!(p.pre_exposure, 1.0);
        assert_eq!(p.exposure_scale, 1.0);
        assert_eq!(p.jitter, [0.0, 0.0]);
        assert!(!p.reset);
        assert!(p.resources.color.is_null());
    }

    #[test]
    fn validate_rejects_missing_resources_and_zero_size() {
        let mut p = eval_params();
        assert_eq!(p.validate(), Ok(()));

        p.resources.color = ptr::null_mut();
        assert_eq!(p.validate(), Err(Dx12EvalError::MissingColor));
        p = eval_params();
        p.resources.output = ptr::null_mut();
        assert_eq!(p.validate(), Err(Dx12EvalError::MissingOutput));
        p = eval_params();
        p.resources.depth = ptr::null_mut();
        assert_eq!(p.validate(), Err(Dx12EvalError::MissingDepth));
        p = eval_params();
        p.resources.motion_vectors = ptr::null_mut();
        assert_eq!(p.validate(), Err(Dx12EvalError::MissingMotionVectors));
        p = eval_params();
        p.render_width = 0;
        assert_eq!(p.validate(), Err(Dx12EvalError::ZeroRenderSize));
    }

    #[test]
    fn sr_eval_params_forwards_every_field() {
        let p = eval_params();
        let s = p.to_sr_eval_params().expect("valid params");
        // Required resources are forwarded verbatim (opaque void* cast).
        assert_eq!(s.Feature.pInColor as usize, p.resources.color as usize);
        assert_eq!(s.Feature.pInOutput as usize, p.resources.output as usize);
        assert_eq!(s.pInDepth as usize, p.resources.depth as usize);
        assert_eq!(
            s.pInMotionVectors as usize,
            p.resources.motion_vectors as usize
        );
        assert_eq!(s.pInExposureTexture as usize, p.resources.exposure as usize);
        assert_eq!(
            s.pInTransparencyMask as usize,
            p.resources.transparency_mask as usize
        );
        assert_eq!(
            s.pInBiasCurrentColorMask as usize,
            p.resources.bias_current_color_mask as usize
        );
        // Per-frame scalars.
        assert_eq!(s.Feature.InSharpness, 0.75);
        assert_eq!(s.InJitterOffsetX, -0.25);
        assert_eq!(s.InJitterOffsetY, 0.5);
        assert_eq!(s.InRenderSubrectDimensions.Width, 1280);
        assert_eq!(s.InRenderSubrectDimensions.Height, 720);
        assert_eq!(s.InReset, 1);
        assert_eq!(s.InMVScaleX, 2.0);
        assert_eq!(s.InMVScaleY, 3.0);
        assert_eq!(s.InPreExposure, 1.5);
        assert_eq!(s.InExposureScale, 0.5);
        assert_eq!(s.InFrameTimeDeltaInMsec, 16.667);
        assert_eq!(
            s.InToneMapperType,
            NVSDK_NGX_ToneMapperType_NVSDK_NGX_TONEMAPPER_STRING
        );
        // Optional research / subrect / invert fields default to zero.
        assert_eq!(s.InIndicatorInvertXAxis, 0);
        assert_eq!(s.InIndicatorInvertYAxis, 0);
        assert_eq!(s.InColorSubrectBase.X, 0);
        assert_eq!(s.InOutputSubrectBase.Y, 0);
        assert!(s.pInMotionVectors3D.is_null());
        assert!(s.pInRayTracingHitDistance.is_null());
        assert!(s.GBufferSurface.pInAttrib.iter().all(|p| p.is_null()));
    }

    #[test]
    fn sr_eval_params_optional_inputs_can_be_null() {
        let mut p = eval_params();
        p.resources = Dx12EvalResources {
            exposure: ptr::null_mut(),
            transparency_mask: ptr::null_mut(),
            bias_current_color_mask: ptr::null_mut(),
            diffuse_albedo: ptr::null_mut(),
            specular_albedo: ptr::null_mut(),
            normals: ptr::null_mut(),
            roughness: ptr::null_mut(),
            ..handles()
        };
        let s = p.to_sr_eval_params().expect("valid params");
        assert!(s.pInExposureTexture.is_null());
        assert!(s.pInTransparencyMask.is_null());
        assert!(s.pInBiasCurrentColorMask.is_null());
    }

    #[test]
    fn rr_eval_params_forwards_core_fields_and_nulls_optional() {
        let p = eval_params();
        let d = p.to_rr_eval_params().expect("valid params");
        assert_eq!(d.pInColor as usize, p.resources.color as usize);
        assert_eq!(d.pInOutput as usize, p.resources.output as usize);
        assert_eq!(d.pInDepth as usize, p.resources.depth as usize);
        assert_eq!(
            d.pInMotionVectors as usize,
            p.resources.motion_vectors as usize
        );
        // RR G-buffer guides are forwarded when present.
        assert_eq!(
            d.pInDiffuseAlbedo as usize,
            p.resources.diffuse_albedo as usize
        );
        assert_eq!(d.pInNormals as usize, p.resources.normals as usize);
        // Common scalars, identical to the SR builder.
        assert_eq!(d.InJitterOffsetX, -0.25);
        assert_eq!(d.InRenderSubrectDimensions.Width, 1280);
        assert_eq!(d.InReset, 1);
        assert_eq!(d.InMVScaleX, 2.0);
        assert_eq!(d.InPreExposure, 1.5);
        assert_eq!(d.InFrameTimeDeltaInMsec, 16.667);
        // The ~90-field DLSSD struct is zero-init for every optional surface;
        // a representative sample of the optionals must be null.
        assert!(d.pInAlpha.is_null());
        assert!(d.pInOutputAlpha.is_null());
        assert!(d.pInReflectedAlbedo.is_null());
        assert!(d.pInDisocclusionMask.is_null());
        assert!(d.pInWorldToViewMatrix.is_null());
    }

    #[test]
    fn rr_eval_params_reject_invalid_input_too() {
        let mut p = eval_params();
        p.resources.color = ptr::null_mut();
        assert_eq!(
            p.to_rr_eval_params().unwrap_err(),
            Dx12EvalError::MissingColor
        );
        assert_eq!(
            p.to_sr_eval_params().unwrap_err(),
            Dx12EvalError::MissingColor
        );
    }

    #[test]
    fn eval_error_display_is_descriptive() {
        assert!(Dx12EvalError::MissingColor.to_string().contains("color"));
        assert!(
            Dx12EvalError::ZeroRenderSize
                .to_string()
                .contains("dimensions")
        );
    }
}
