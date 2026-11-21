//! # DLSS Frame Generation (DLSS-G / NGX) — data-only layer
//!
//! Feature-gated (`frame_generation`) NGX Frame Generation support.
//!
//! Everything in this module is **CPU-verifiable pure data**. It never opens a
//! GPU device, loads a DLL, or calls any NGX function at runtime (including in
//! tests). No C function signatures are invented.
//!
//! ## Bindings are regenerated, not hand-copied
//! The pinned main bindings (`crate::nvsdk_ngx`, generated from `wrapper.h`)
//! predate the DLSS-G headers. Rather than transcribe `#define`d strings by
//! hand, [`dlssg_keys`] `include!`s real `rust-bindgen 0.73.2` output checked
//! into `src/bindings/frame_generation.rs`, generated from NVIDIA's public
//! `nvsdk_ngx_defs_dlssg.h` (DLSS SDK v310.9.16). It contains all 126
//! `FrameGeneration.*` / `FrameInterpolation.*` / `DLSSG.*` parameter keys, the
//! `NVSDK_NGX_DLSSG_ResourceFlags` / `EvalFlags` / `FullscreenMode` enums, and
//! the `PFN_NVSDK_NGX_DLSSG_*` callback typedefs as real generated types.
//! `src/wrapper.h` now includes `nvsdk_ngx_defs_dlssg.h` (it is C-safe), so a
//! full SDK regeneration naturally includes the FG keys/enums too. See
//! `vendor/dlss_wgpu/README.md` for the exact regeneration command.
//!
//! ## What is still absent
//! The callable DLSS-G create/evaluate helpers (`NGX_VK_CREATE_DLSSG` /
//! `NGX_VK_EVALUATE_DLSSG` and their `*_Eval_Params` structs) live in the
//! **C++-only** `nvsdk_ngx_helpers_dlssg_vk.h` / `nvsdk_ngx_params_dlssg.h`
//! (default member initializers), while `wrap_static_fns.c` compiles as C —
//! that is precisely why FG was omitted from the original `wrapper.h`. They
//! need `__extern` C++ shims plus NVIDIA Streamline present interception.
//! Consequently [`create_frame_generation`] / [`evaluate_frame_generation`]
//! remain documented `Unsupported` stubs; see
//! [`BINDINGS_REGENERATION_REQUIRED_FOR`].
//!
//! The module is compiled only with `feature = "frame_generation"`; under
//! `feature = "mock"` it is empty, mirroring `crate::dx12`.
#![cfg(not(feature = "mock"))]

use crate::nvsdk_ngx::{NVSDK_NGX_Feature, NVSDK_NGX_Feature_Support_Result};

/// The NGX feature id for DLSS Frame Generation.
///
/// This uses the real bindgen constant
/// (`NVSDK_NGX_Feature_NVSDK_NGX_Feature_FrameGeneration`) from the generated
/// bindings; its value is `11`.
pub const FEATURE_FRAME_GENERATION: NVSDK_NGX_Feature =
    crate::nvsdk_ngx::NVSDK_NGX_Feature_NVSDK_NGX_Feature_FrameGeneration;

/// High-level, CPU-side Frame Generation policy (`Off` / `On` / `Auto`).
///
/// This is a convenience/config enum for application code; it is **not** an NGX
/// enum. The public `nvsdk_ngx_defs_dlssg.h` header instead defines
/// `NVSDK_NGX_DLSSG_FullscreenMode`, `NVSDK_NGX_DLSSG_EvalFlags` and
/// `NVSDK_NGX_DLSSG_ResourceFlags`; none of those are present in the pinned
/// bindings, so they are intentionally not reproduced here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FrameGenerationMode {
    /// Frame generation disabled (no interpolated frames).
    #[default]
    Off,
    /// Frame generation enabled.
    On,
    /// Let the application / driver decide.
    Auto,
}

/// Pure-data description of a DLSS-G feature create.
///
/// **DATA ONLY:** every field is a plain integer / bool / enum. Constructing or
/// inspecting this struct performs no NGX, DLL, or GPU work. Each field is
/// annotated with the `nvsdk_ngx_defs_dlssg.h` parameter key it corresponds to;
/// those keys now exist as generated bindings in [`dlssg_keys`]. The fields are
/// stored as raw numeric values because this descriptor is CPU-side policy data,
/// not an NGX `*_Create_Params` struct (which would require a live GPU path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameGenerationDescriptor {
    /// CPU-side policy (`Off` / `On` / `Auto`).
    pub mode: FrameGenerationMode,
    /// Render width — header key `DLSSG.Width` (or generic `Width`).
    pub width: u32,
    /// Render height — header key `DLSSG.Height` (or generic `Height`).
    pub height: u32,
    /// Feature create flags bitmask — `DLSS.Feature.Create.Flags`.
    pub create_flags: u32,
    /// Resource-presence flags bitmask — `DLSSG.ResourceAlwaysProvidedFlags`
    /// / `NVSDK_NGX_DLSSG_ResourceFlags`.
    pub resource_flags: u32,
    /// `NVSDK_NGX_PerfQuality_Value` — `PerfQualityValue`.
    pub perf_quality: u32,
    /// Number of frames to interpolate per real frame
    /// (`DLSSG.MultiFrameCount`; 1 for 2x, 3 for 4x).
    pub num_frames_to_generate: u32,
    /// Maximum supported generated frames (`DLSSG.MultiFrameCountMax`).
    pub multi_frame_count_max: u32,
    /// `DLSSG.DepthInverted`.
    pub depth_inverted: bool,
    /// `DLSSG.ColorBuffersHDR`.
    pub color_buffers_hdr: bool,
    /// `DLSSG.AsyncCreateEnabled`.
    pub enable_async_create: bool,
    /// `DLSSG.Reset`.
    pub reset: bool,
}

impl Default for FrameGenerationDescriptor {
    fn default() -> Self {
        Self {
            mode: FrameGenerationMode::Off,
            width: 0,
            height: 0,
            create_flags: 0,
            resource_flags: 0,
            perf_quality: 0,
            num_frames_to_generate: 1,
            multi_frame_count_max: 1,
            depth_inverted: false,
            color_buffers_hdr: false,
            enable_async_create: false,
            reset: false,
        }
    }
}

/// Result of a Frame Generation capability check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameGenerationSupport {
    /// The feature is available.
    Supported,
    /// The feature is unavailable.
    NotSupported,
}

impl FrameGenerationSupport {
    /// Map a boolean "available" flag (e.g. a capability query result) into a
    /// [`FrameGenerationSupport`] value. Pure CPU mapping, no NGX calls.
    pub fn from_available(available: bool) -> Self {
        if available {
            Self::Supported
        } else {
            Self::NotSupported
        }
    }

    /// Map into the NGX `NVSDK_NGX_Feature_Support_Result` value that exists in
    /// the pinned bindings.
    ///
    /// `Supported` becomes `...FeatureSupportResult_Supported` (`0`). The NGX
    /// enum is a bitfield of *reasons* and has no plain "not supported" value;
    /// `NotSupported` is therefore reported as
    /// `...FeatureSupportResult_NotImplemented` (`16`), documented here rather
    /// than invented.
    pub fn as_ngx_feature_support_result(self) -> NVSDK_NGX_Feature_Support_Result {
        match self {
            Self::Supported => {
                crate::nvsdk_ngx::NVSDK_NGX_Feature_Support_Result_NVSDK_NGX_FeatureSupportResult_Supported
            }
            Self::NotSupported => {
                crate::nvsdk_ngx::NVSDK_NGX_Feature_Support_Result_NVSDK_NGX_FeatureSupportResult_NotImplemented
            }
        }
    }
}

// ---- FG-related parameter keys that ACTUALLY exist in the pinned bindings ----
//
// These are the only Frame-Generation-adjacent `NVSDK_NGX_Parameter_*` keys
// present in `src/bindings/*.rs`. Each aliases, verbatim, the generated
// bindings' `&[u8; N]` NUL-terminated C string, re-exposed here as `&[u8]`.

/// `NVSDK_NGX_Parameter_FrameInterpolation_NeedsUpdatedDriver`
/// (`"FrameInterpolation.NeedsUpdatedDriver"`).
pub const PARAM_FRAME_INTERPOLATION_NEEDS_UPDATED_DRIVER: &[u8] =
    crate::nvsdk_ngx::NVSDK_NGX_Parameter_FrameInterpolation_NeedsUpdatedDriver;
/// `NVSDK_NGX_Parameter_FrameInterpolation_MinDriverVersionMajor`
/// (`"FrameInterpolation.MinDriverVersionMajor"`).
pub const PARAM_FRAME_INTERPOLATION_MIN_DRIVER_VERSION_MAJOR: &[u8] =
    crate::nvsdk_ngx::NVSDK_NGX_Parameter_FrameInterpolation_MinDriverVersionMajor;
/// `NVSDK_NGX_Parameter_FrameInterpolation_FeatureInitResult`
/// (`"FrameInterpolation.FeatureInitResult"`).
pub const PARAM_FRAME_INTERPOLATION_FEATURE_INIT_RESULT: &[u8] =
    crate::nvsdk_ngx::NVSDK_NGX_Parameter_FrameInterpolation_FeatureInitResult;

/// Every FG-related parameter key that actually exists in the pinned bindings.
///
/// Used by the unit tests to assert that each exposed key is a valid,
/// NUL-terminated C string.
pub const FRAME_INTERPOLATION_PARAM_KEYS: [&[u8]; 3] = [
    PARAM_FRAME_INTERPOLATION_NEEDS_UPDATED_DRIVER,
    PARAM_FRAME_INTERPOLATION_MIN_DRIVER_VERSION_MAJOR,
    PARAM_FRAME_INTERPOLATION_FEATURE_INIT_RESULT,
];

/// Bindgen-generated NGX DLSS-G parameter keys, enums and callback typedefs,
/// regenerated (not hand-copied) from NVIDIA's public `nvsdk_ngx_defs_dlssg.h`
/// (DLSS SDK v310.9.16) with `rust-bindgen 0.73.2`. The generated file is
/// checked in at `src/bindings/frame_generation.rs`; see
/// `vendor/dlss_wgpu/README.md` for the regeneration command.
///
/// This is pure compile-time data: 126 NUL-terminated `&[u8; N]` C-string
/// constants, 3 DLSSG enums (`c_uint` aliases + constants) and 2 PFN callback
/// typedefs. `ngx_param_key_ptr` turns any key into an ABI-safe `*const c_char`
/// for the NGX `Set`/`Get` parameter API. No GPU/DLL/device work is performed.
#[allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    dead_code
)]
pub mod dlssg_keys {
    include!("bindings/frame_generation.rs");

    /// All 126 generated parameter keys (built from the included bindings).
    pub const ALL: &[&[u8]] = &[
        NVSDK_NGX_Parameter_FrameInterpolation_NeedsUpdatedDriver,
        NVSDK_NGX_Parameter_FrameInterpolation_MinDriverVersionMajor,
        NVSDK_NGX_Parameter_FrameInterpolation_FeatureInitResult,
        NVSDK_NGX_Parameter_FrameGeneration_Available,
        NVSDK_NGX_Parameter_FrameGeneration_FeatureInitResult,
        NVSDK_NGX_Parameter_FrameGeneration_MinDriverVersionMajor,
        NVSDK_NGX_Parameter_FrameGeneration_MinDriverVersionMinor,
        NVSDK_NGX_Parameter_FrameGeneration_NeedsUpdatedDriver,
        NVSDK_NGX_Parameter_FrameInterpolation_Available,
        NVSDK_NGX_Parameter_FrameInterpolation_MinDriverVersionMinor,
        NVSDK_NGX_DLSSG_Parameter_BackbufferFormat,
        NVSDK_NGX_DLSSG_Parameter_Backbuffer,
        NVSDK_NGX_DLSSG_Parameter_MVecs,
        NVSDK_NGX_DLSSG_Parameter_Depth,
        NVSDK_NGX_DLSSG_Parameter_BidirectionalDistortionField,
        NVSDK_NGX_DLSSG_Parameter_HUDLess,
        NVSDK_NGX_DLSSG_Parameter_UI,
        NVSDK_NGX_DLSSG_Parameter_UIAlpha,
        NVSDK_NGX_DLSSG_Parameter_OutputInterpolated,
        NVSDK_NGX_DLSSG_Parameter_OutputReal,
        NVSDK_NGX_DLSSG_Parameter_ResourceAlwaysProvided_Flags,
        NVSDK_NGX_DLSSG_Parameter_ResourceNeverProvided_Flags,
        NVSDK_NGX_DLSSG_Parameter_Width,
        NVSDK_NGX_DLSSG_Parameter_Height,
        NVSDK_NGX_DLSSG_Parameter_OutputDisableInterpolation,
        NVSDK_NGX_DLSSG_Parameter_CameraViewToClip,
        NVSDK_NGX_DLSSG_Parameter_ClipToCameraView,
        NVSDK_NGX_DLSSG_Parameter_ClipToLensClip,
        NVSDK_NGX_DLSSG_Parameter_ClipToPrevClip,
        NVSDK_NGX_DLSSG_Parameter_PrevClipToClip,
        NVSDK_NGX_DLSSG_Parameter_JitterOffsetX,
        NVSDK_NGX_DLSSG_Parameter_JitterOffsetY,
        NVSDK_NGX_DLSSG_Parameter_MvecScaleX,
        NVSDK_NGX_DLSSG_Parameter_MvecScaleY,
        NVSDK_NGX_DLSSG_Parameter_CameraPinholeOffsetX,
        NVSDK_NGX_DLSSG_Parameter_CameraPinholeOffsetY,
        NVSDK_NGX_DLSSG_Parameter_CameraPosX,
        NVSDK_NGX_DLSSG_Parameter_CameraPosY,
        NVSDK_NGX_DLSSG_Parameter_CameraPosZ,
        NVSDK_NGX_DLSSG_Parameter_CameraUpX,
        NVSDK_NGX_DLSSG_Parameter_CameraUpY,
        NVSDK_NGX_DLSSG_Parameter_CameraUpZ,
        NVSDK_NGX_DLSSG_Parameter_CameraRightX,
        NVSDK_NGX_DLSSG_Parameter_CameraRightY,
        NVSDK_NGX_DLSSG_Parameter_CameraRightZ,
        NVSDK_NGX_DLSSG_Parameter_CameraFwdX,
        NVSDK_NGX_DLSSG_Parameter_CameraFwdY,
        NVSDK_NGX_DLSSG_Parameter_CameraFwdZ,
        NVSDK_NGX_DLSSG_Parameter_CameraNear,
        NVSDK_NGX_DLSSG_Parameter_CameraFar,
        NVSDK_NGX_DLSSG_Parameter_CameraFOV,
        NVSDK_NGX_DLSSG_Parameter_CameraAspectRatio,
        NVSDK_NGX_DLSSG_Parameter_ColorBuffersHDR,
        NVSDK_NGX_DLSSG_Parameter_DepthInverted,
        NVSDK_NGX_DLSSG_Parameter_CameraMotionIncluded,
        NVSDK_NGX_DLSSG_Parameter_Reset,
        NVSDK_NGX_DLSSG_Parameter_AutomodeOverrideReset,
        NVSDK_NGX_DLSSG_Parameter_NotRenderingGameFrames,
        NVSDK_NGX_DLSSG_Parameter_OrthoProjection,
        NVSDK_NGX_DLSSG_Parameter_EstimateVRAMCallback,
        NVSDK_NGX_DLSSG_Parameter_InvertXAxis,
        NVSDK_NGX_DLSSG_Parameter_InvertYAxis,
        NVSDK_NGX_DLSSG_Parameter_UserDebugText,
        NVSDK_NGX_DLSSG_Parameter_MvecInvalidValue,
        NVSDK_NGX_DLSSG_Parameter_MvecDilated,
        NVSDK_NGX_DLSSG_Parameter_MvecJittered,
        NVSDK_NGX_DLSSG_Parameter_MenuDetectionEnabled,
        NVSDK_NGX_DLSSG_Parameter_AsyncCreateEnabled,
        NVSDK_NGX_DLSSG_Parameter_LinearizedDepth_Scale,
        NVSDK_NGX_DLSSG_Parameter_LinearizedDepth_NearFarPartition,
        NVSDK_NGX_DLSSG_Parameter_MinRelativeLinearDepthObjectSeparation,
        NVSDK_NGX_DLSSG_Parameter_InputBackbufferSubrectBaseX,
        NVSDK_NGX_DLSSG_Parameter_InputBackbufferSubrectBaseY,
        NVSDK_NGX_DLSSG_Parameter_InputBackbufferSubrectWidth,
        NVSDK_NGX_DLSSG_Parameter_InputBackbufferSubrectHeight,
        NVSDK_NGX_DLSSG_Parameter_MVecsSubrectBaseX,
        NVSDK_NGX_DLSSG_Parameter_MVecsSubrectBaseY,
        NVSDK_NGX_DLSSG_Parameter_MVecsSubrectWidth,
        NVSDK_NGX_DLSSG_Parameter_MVecsSubrectHeight,
        NVSDK_NGX_DLSSG_Parameter_DepthSubrectBaseX,
        NVSDK_NGX_DLSSG_Parameter_DepthSubrectBaseY,
        NVSDK_NGX_DLSSG_Parameter_DepthSubrectWidth,
        NVSDK_NGX_DLSSG_Parameter_DepthSubrectHeight,
        NVSDK_NGX_DLSSG_Parameter_HUDLessSubrectBaseX,
        NVSDK_NGX_DLSSG_Parameter_HUDLessSubrectBaseY,
        NVSDK_NGX_DLSSG_Parameter_HUDLessSubrectWidth,
        NVSDK_NGX_DLSSG_Parameter_HUDLessSubrectHeight,
        NVSDK_NGX_DLSSG_Parameter_UISubrectBaseX,
        NVSDK_NGX_DLSSG_Parameter_UISubrectBaseY,
        NVSDK_NGX_DLSSG_Parameter_UISubrectWidth,
        NVSDK_NGX_DLSSG_Parameter_UISubrectHeight,
        NVSDK_NGX_DLSSG_Parameter_UIAlphaSubrectBaseX,
        NVSDK_NGX_DLSSG_Parameter_UIAlphaSubrectBaseY,
        NVSDK_NGX_DLSSG_Parameter_UIAlphaSubrectWidth,
        NVSDK_NGX_DLSSG_Parameter_UIAlphaSubrectHeight,
        NVSDK_NGX_DLSSG_Parameter_BidirectionalDistortionFieldSubrectBaseX,
        NVSDK_NGX_DLSSG_Parameter_BidirectionalDistortionFieldSubrectBaseY,
        NVSDK_NGX_DLSSG_Parameter_BidirectionalDistortionFieldSubrectWidth,
        NVSDK_NGX_DLSSG_Parameter_BidirectionalDistortionFieldSubrectHeight,
        NVSDK_NGX_DLSSG_Parameter_BidirectionalDistortionField_LowPrecision_IsLowPrecision,
        NVSDK_NGX_DLSSG_Parameter_BidirectionalDistortionField_LowPrecision_Bias,
        NVSDK_NGX_DLSSG_Parameter_BidirectionalDistortionField_LowPrecision_Scale,
        NVSDK_NGX_DLSSG_Parameter_OutputInterpolatedSubrectBaseX,
        NVSDK_NGX_DLSSG_Parameter_OutputInterpolatedSubrectBaseY,
        NVSDK_NGX_DLSSG_Parameter_OutputInterpolatedSubrectWidth,
        NVSDK_NGX_DLSSG_Parameter_OutputInterpolatedSubrectHeight,
        NVSDK_NGX_DLSSG_Parameter_OutputRealSubrectBaseX,
        NVSDK_NGX_DLSSG_Parameter_OutputRealSubrectBaseY,
        NVSDK_NGX_DLSSG_Parameter_OutputRealSubrectWidth,
        NVSDK_NGX_DLSSG_Parameter_OutputRealSubrectHeight,
        NVSDK_NGX_DLSSG_Parameter_BackbufferSubrectBaseX,
        NVSDK_NGX_DLSSG_Parameter_BackbufferSubrectBaseY,
        NVSDK_NGX_DLSSG_Parameter_BackbufferSubrectWidth,
        NVSDK_NGX_DLSSG_Parameter_BackbufferSubrectHeight,
        NVSDK_NGX_DLSSG_Parameter_InternalWidth,
        NVSDK_NGX_DLSSG_Parameter_InternalHeight,
        NVSDK_NGX_DLSSG_Parameter_DynamicResolution,
        NVSDK_NGX_DLSSG_Parameter_EvalFlags,
        NVSDK_NGX_DLSSG_Parameter_FullscreenMode,
        NVSDK_NGX_DLSSG_Parameter_TargetFrameRate,
        NVSDK_NGX_DLSSG_Parameter_MultiFrameCountMax,
        NVSDK_NGX_DLSSG_Parameter_MultiFrameCount,
        NVSDK_NGX_DLSSG_Parameter_MultiFrameIndex,
        NVSDK_NGX_DLSSG_Parameter_BackbufferFrameID,
        NVSDK_NGX_DLSSG_Parameter_UserInterfaceRecompositionEnabled,
        NVSDK_NGX_DLSSG_Parameter_LiteCandidateBlendingEnabled,
    ];

    /// `NVSDK_NGX_DLSSG_ResourceFlags` bit values — aliases of the generated
    /// constants (no values transcribed by hand).
    pub mod resource_flags {
        use super::*;
        pub const BACKBUFFER: u32 =
            NVSDK_NGX_DLSSG_ResourceFlags_NVSDK_NGX_DLSSG_ResourceFlags_Backbuffer;
        pub const MVECS: u32 = NVSDK_NGX_DLSSG_ResourceFlags_NVSDK_NGX_DLSSG_ResourceFlags_MVecs;
        pub const DEPTH: u32 = NVSDK_NGX_DLSSG_ResourceFlags_NVSDK_NGX_DLSSG_ResourceFlags_Depth;
        pub const HUDLESS: u32 =
            NVSDK_NGX_DLSSG_ResourceFlags_NVSDK_NGX_DLSSG_ResourceFlags_HUDLess;
        pub const UI: u32 = NVSDK_NGX_DLSSG_ResourceFlags_NVSDK_NGX_DLSSG_ResourceFlags_UI;
        pub const BIDIRECTIONAL_DISTORTION_FIELD: u32 = NVSDK_NGX_DLSSG_ResourceFlags_NVSDK_NGX_DLSSG_ResourceFlags_BidirectionalDistortionField;
        pub const RESERVED6: u32 =
            NVSDK_NGX_DLSSG_ResourceFlags_NVSDK_NGX_DLSSG_ResourceFlags_Reserved6;
        pub const UI_ALPHA: u32 =
            NVSDK_NGX_DLSSG_ResourceFlags_NVSDK_NGX_DLSSG_ResourceFlags_UIAlpha;
        pub const OUTPUT_INTERPOLATED: u32 =
            NVSDK_NGX_DLSSG_ResourceFlags_NVSDK_NGX_DLSSG_ResourceFlags_OutputInterpolated;
        pub const OUTPUT_REAL: u32 =
            NVSDK_NGX_DLSSG_ResourceFlags_NVSDK_NGX_DLSSG_ResourceFlags_OutputReal;
        pub const OUTPUT_DISABLE_INTERPOLATION: u32 =
            NVSDK_NGX_DLSSG_ResourceFlags_NVSDK_NGX_DLSSG_ResourceFlags_OutputDisableInterpolation;
    }

    /// `NVSDK_NGX_DLSSG_EvalFlags` (only one value in v310.9.16).
    pub mod eval_flags {
        use super::*;
        pub const UPDATE_ONLY_INSIDE_EXTENTS: u32 =
            NVSDK_NGX_DLSSG_EvalFlags_NVSDK_NGX_DLSSG_EvalFlags_UpdateOnlyInsideExtents;
    }

    /// `NVSDK_NGX_DLSSG_FullscreenMode` values.
    pub mod fullscreen_mode {
        use super::*;
        pub const WINDOWED: u32 =
            NVSDK_NGX_DLSSG_FullscreenMode_NVSDK_NGX_DLSSG_FullscreenMode_Windowed;
        pub const BORDERLESS: u32 =
            NVSDK_NGX_DLSSG_FullscreenMode_NVSDK_NGX_DLSSG_FullscreenMode_Borderless;
        pub const EXCLUSIVE: u32 =
            NVSDK_NGX_DLSSG_FullscreenMode_NVSDK_NGX_DLSSG_FullscreenMode_Exclusive;
    }
}

/// `#[repr(C)]`-compatible FFI helper: the `&[u8]` constants above are ABI-safe
/// byte arrays; this turns a NUL-terminated NGX parameter key into a
/// `*const c_char` suitable for the NGX `Set` / `Get` parameter API.
///
/// Returns a null pointer when `key` is not NUL-terminated (so a malformed key
/// can never be handed to C). No NGX function is called here.
pub fn ngx_param_key_ptr(key: &[u8]) -> *const std::os::raw::c_char {
    if key.last() == Some(&0) {
        key.as_ptr().cast::<std::os::raw::c_char>()
    } else {
        std::ptr::null()
    }
}

/// All DLSS-G **data** (parameter keys, `NVSDK_NGX_DLSSG_*` enums, and the
/// `PFN_NVSDK_NGX_DLSSG_*` callback typedefs) is now real generated bindings in
/// [`dlssg_keys`], produced from NVIDIA's public `nvsdk_ngx_defs_dlssg.h` with
/// bindgen 0.73.2 — nothing is transcribed by hand.
///
/// What remains genuinely absent is the **callable** DLSS-G create/evaluate
/// layer: `NGX_VK_CREATE_DLSSG` / `NGX_VK_EVALUATE_DLSSG` and their
/// `*_Eval_Params` / `*_Create_Params` structs. Those live in the C++-only
/// `nvsdk_ngx_helpers_dlssg_vk.h` / `nvsdk_ngx_params_dlssg.h` (default member
/// initializers), which cannot be included by the C-compiled `wrap_static_fns.c`.
/// Exposing them needs `__extern` C++ shims plus NVIDIA Streamline present
/// interception. That is why [`create_frame_generation`] returns `Unsupported`.
pub const BINDINGS_REGENERATION_REQUIRED_FOR: &[&str] = &[
    "NGX_VK_CREATE_DLSSG (C++ static inline helper -> __extern shim)",
    "NGX_VK_EVALUATE_DLSSG (C++ static inline helper -> __extern shim)",
    "NVSDK_NGX_VK_DLSSG_Eval_Params / NVSDK_NGX_DLSSG_Create_Params / Opt_Eval_Params",
    "NGX_D3D12_CREATE_DLSSG / NGX_D3D12_EVALUATE_DLSSG (D3D12 FG swapchain)",
];

/// Errors returned by the (currently unsupported) Frame Generation runtime
/// path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameGenerationError {
    /// The descriptor failed pure CPU-side validation.
    InvalidDescriptor(&'static str),
    /// The runtime operation cannot be performed because the pinned NGX
    /// bindings expose no callable DLSS-G create/evaluate symbol.
    Unsupported(&'static str),
}

impl std::fmt::Display for FrameGenerationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDescriptor(msg) => write!(f, "invalid DLSS-G descriptor: {msg}"),
            Self::Unsupported(msg) => write!(f, "DLSS Frame Generation unsupported: {msg}"),
        }
    }
}

impl std::error::Error for FrameGenerationError {}

const NO_FG_SYMBOL: &str = "the DLSS-G parameter keys, enums and callback \
    typedefs are now real generated bindings (src/bindings/frame_generation.rs), \
    but the callable NGX_VK_CREATE_DLSSG / NGX_VK_EVALUATE_DLSSG helpers and \
    their *_Eval/_Create params live in C++-only NGX helper headers and are not \
    exposed through the C-compiled wrap_static_fns.c; they need __extern C++ \
    shims + NVIDIA Streamline present interception, so no C signature is \
    invented here";

impl FrameGenerationDescriptor {
    /// Pure CPU-side consistency check; performs no NGX / DLL / GPU work.
    pub fn validate(&self) -> Result<(), FrameGenerationError> {
        if self.num_frames_to_generate == 0 {
            return Err(FrameGenerationError::InvalidDescriptor(
                "num_frames_to_generate must be >= 1",
            ));
        }
        if self.multi_frame_count_max != 0
            && self.num_frames_to_generate > self.multi_frame_count_max
        {
            return Err(FrameGenerationError::InvalidDescriptor(
                "num_frames_to_generate exceeds multi_frame_count_max",
            ));
        }
        Ok(())
    }
}

/// Attempt to create a DLSS-G feature instance.
///
/// This is a **documented, `todo!()`-free stub**, not a working create. After
/// running the pure CPU-side [`FrameGenerationDescriptor::validate`], it always
/// returns [`FrameGenerationError::Unsupported`], because the pinned bindings
/// contain no callable DLSS-G create symbol (only the generic, GPU/DLL-dependent
/// `NVSDK_NGX_VULKAN_CreateFeature` / `EvaluateFeature_C`). No NGX function is
/// called and no C signature is invented.
pub fn create_frame_generation(
    descriptor: &FrameGenerationDescriptor,
) -> Result<(), FrameGenerationError> {
    descriptor.validate()?;
    Err(FrameGenerationError::Unsupported(NO_FG_SYMBOL))
}

/// Attempt to evaluate (generate a frame with) a DLSS-G feature instance.
///
/// Same documented [`FrameGenerationError::Unsupported`] path as
/// [`create_frame_generation`]: no callable DLSS-G evaluate symbol exists in the
/// pinned bindings, so no NGX function is called.
pub fn evaluate_frame_generation() -> Result<(), FrameGenerationError> {
    Err(FrameGenerationError::Unsupported(NO_FG_SYMBOL))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_id_is_eleven() {
        assert_eq!(FEATURE_FRAME_GENERATION, 11);
        assert_eq!(
            FEATURE_FRAME_GENERATION,
            crate::nvsdk_ngx::NVSDK_NGX_Feature_NVSDK_NGX_Feature_FrameGeneration,
        );
    }

    #[test]
    fn default_descriptor_fields() {
        let d = FrameGenerationDescriptor::default();
        assert_eq!(d.mode, FrameGenerationMode::Off);
        assert_eq!(d.width, 0);
        assert_eq!(d.height, 0);
        assert_eq!(d.create_flags, 0);
        assert_eq!(d.resource_flags, 0);
        assert_eq!(d.perf_quality, 0);
        assert_eq!(d.num_frames_to_generate, 1);
        assert_eq!(d.multi_frame_count_max, 1);
        assert!(!d.depth_inverted);
        assert!(!d.color_buffers_hdr);
        assert!(!d.enable_async_create);
        assert!(!d.reset);
    }

    #[test]
    fn every_exposed_param_key_is_nul_terminated() {
        assert_eq!(FRAME_INTERPOLATION_PARAM_KEYS.len(), 3);
        for key in FRAME_INTERPOLATION_PARAM_KEYS {
            assert!(!key.is_empty());
            assert_eq!(key.last(), Some(&0), "key must end with a NUL byte");
            assert!(
                !key[..key.len() - 1].contains(&0),
                "key must contain no interior NUL byte",
            );
            assert!(!ngx_param_key_ptr(key).is_null());
        }
        // A key without a trailing NUL is rejected by the FFI helper.
        assert!(ngx_param_key_ptr(b"no-nul").is_null());
    }

    #[test]
    fn support_from_bool_mapping() {
        assert_eq!(
            FrameGenerationSupport::from_available(true),
            FrameGenerationSupport::Supported,
        );
        assert_eq!(
            FrameGenerationSupport::from_available(false),
            FrameGenerationSupport::NotSupported,
        );
        assert_eq!(
            FrameGenerationSupport::Supported.as_ngx_feature_support_result(),
            crate::nvsdk_ngx::NVSDK_NGX_Feature_Support_Result_NVSDK_NGX_FeatureSupportResult_Supported,
        );
        assert_eq!(
            FrameGenerationSupport::NotSupported.as_ngx_feature_support_result(),
            crate::nvsdk_ngx::NVSDK_NGX_Feature_Support_Result_NVSDK_NGX_FeatureSupportResult_NotImplemented,
        );
    }

    #[test]
    fn descriptor_validation_is_cpu_only() {
        assert!(FrameGenerationDescriptor::default().validate().is_ok());

        let mut zero_frames = FrameGenerationDescriptor::default();
        zero_frames.num_frames_to_generate = 0;
        assert!(matches!(
            zero_frames.validate(),
            Err(FrameGenerationError::InvalidDescriptor(_)),
        ));

        let mut too_many = FrameGenerationDescriptor::default();
        too_many.num_frames_to_generate = 4;
        too_many.multi_frame_count_max = 3;
        assert!(too_many.validate().is_err());
    }

    #[test]
    fn runtime_path_is_documented_unsupported() {
        let d = FrameGenerationDescriptor::default();
        assert!(matches!(
            create_frame_generation(&d),
            Err(FrameGenerationError::Unsupported(_)),
        ));
        assert!(matches!(
            evaluate_frame_generation(),
            Err(FrameGenerationError::Unsupported(_)),
        ));
    }

    #[test]
    fn generated_dlssg_keys_are_valid_c_strings() {
        use super::dlssg_keys as k;
        // All 126 keys are real bindgen output from the public header.
        assert_eq!(k::ALL.len(), 126);
        for key in k::ALL {
            assert!(!key.is_empty());
            assert_eq!(key.last(), Some(&0), "key must be NUL-terminated: {key:?}");
            assert!(
                !key[..key.len() - 1].contains(&0),
                "key must have no interior NUL: {key:?}",
            );
            assert!(!ngx_param_key_ptr(key).is_null());
        }
        // Spot-check verbatim values from NVIDIA's public header (generated names).
        assert_eq!(
            &k::NVSDK_NGX_DLSSG_Parameter_Backbuffer
                [..k::NVSDK_NGX_DLSSG_Parameter_Backbuffer.len() - 1],
            b"DLSSG.Backbuffer"
        );
        assert_eq!(
            &k::NVSDK_NGX_DLSSG_Parameter_MultiFrameCount
                [..k::NVSDK_NGX_DLSSG_Parameter_MultiFrameCount.len() - 1],
            b"DLSSG.MultiFrameCount"
        );
        assert_eq!(
            &k::NVSDK_NGX_Parameter_FrameGeneration_Available
                [..k::NVSDK_NGX_Parameter_FrameGeneration_Available.len() - 1],
            b"FrameGeneration.Available"
        );
        // No duplicate keys.
        let mut seen = std::collections::BTreeSet::new();
        for key in k::ALL {
            assert!(seen.insert(*key), "duplicate DLSS-G key: {key:?}");
        }
    }

    #[test]
    fn dlssg_enum_constants_match_header() {
        use super::dlssg_keys::{eval_flags, fullscreen_mode, resource_flags};
        assert_eq!(resource_flags::BACKBUFFER, 1);
        assert_eq!(resource_flags::OUTPUT_INTERPOLATED, 1 << 24);
        assert_eq!(resource_flags::OUTPUT_DISABLE_INTERPOLATION, 1 << 26);
        assert_eq!(eval_flags::UPDATE_ONLY_INSIDE_EXTENTS, 1);
        assert_eq!(fullscreen_mode::EXCLUSIVE, 2);
    }
}
