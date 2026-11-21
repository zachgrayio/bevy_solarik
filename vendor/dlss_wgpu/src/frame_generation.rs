//! # DLSS Frame Generation (DLSS-G / NGX) — data-only layer
//!
//! Feature-gated (`frame_generation`) NGX Frame Generation support.
//!
//! Everything in this module is **CPU-verifiable pure data**. It never opens a
//! GPU device, loads a DLL, or calls any NGX function at runtime (including in
//! tests). The NGX items it refers to are compile-time constants and type
//! aliases taken verbatim from the crate's generated bindings in
//! `crate::nvsdk_ngx` (`src/bindings/x86_64-pc-windows-msvc.rs` /
//! `x86_64-unknown-linux-gnu.rs`). No C function signatures are invented.
//!
//! ## What the pinned bindings actually expose
//! The bindings were generated from `nvsdk_ngx_defs.h` / `nvsdk_ngx.h` and the
//! Vulkan headers, **not** from NVIDIA's `nvsdk_ngx_defs_dlssg.h`. They contain:
//! - the feature id `NVSDK_NGX_Feature_NVSDK_NGX_Feature_FrameGeneration = 11`
//!   ([`FEATURE_FRAME_GENERATION`]),
//! - three `FrameInterpolation.*` capability parameter keys
//!   ([`FRAME_INTERPOLATION_PARAM_KEYS`]),
//! - the `NVSDK_NGX_Feature_Support_Result` values (used by
//!   [`FrameGenerationSupport::as_ngx_feature_support_result`]),
//! - generic `NVSDK_NGX_VULKAN_CreateFeature` / `EvaluateFeature_C` /
//!   `GetFeatureInstanceExtensionRequirements` functions, plus the opaque
//!   `NVSDK_NGX_Handle` / `NVSDK_NGX_Parameter` types and the
//!   `NVSDK_NGX_Feature` / `NVSDK_NGX_Result` (`c_int`) aliases.
//!
//! They contain **no** `FrameGeneration.*` or `DLSSG.*` parameter keys, no
//! `NVSDK_NGX_DLSSG_*` enums/typedefs, and — critically — **no callable
//! DLSS-G-specific create/evaluate symbol** (no `PFN_NVSDK_NGX_*FrameGeneration*`,
//! no DLSSG create/evaluate function). The generic VULKAN create/evaluate
//! functions do exist, but they are not Frame-Generation-specific and calling
//! them requires a live GPU device / command buffer / loaded NGX DLL, which is
//! outside this CPU-verifiable layer; they are intentionally not wrapped.
//! See [`MISSING_FG_PARAMETER_KEY_NAMES`] and [`create_frame_generation`] for
//! the documented gaps and the resulting `Unsupported` runtime path.
//!
//! ## Header-verified parameter keys
//! Although the *generated* bindings omit DLSS-G keys, the parameter keys are
//! plain string macros in NVIDIA's public `nvsdk_ngx_defs_dlssg.h` (v310.9.16).
//! [`dlssg_keys`] reproduces them verbatim as NUL-terminated C strings so the
//! NGX `Set`/`Get` parameter API can address them without regenerating bindings
//! (which would require libclang + the NGX SDK headers at build time). The
//! `NVSDK_NGX_DLSSG_*` enums are likewise reproduced as bit/int constants.
//! Still absent are the `PFN_NVSDK_NGX_DLSSG_*` callback typedefs and any
//! DLSS-G-specific create/evaluate *symbol* — hence the `Unsupported` path.
//!
//! The module is compiled only with `feature = "frame_generation"`; under
//! `feature = "mock"` it is empty, mirroring `crate::dx12` (there are no real
//! FFI bindings to adapt in a mock build).
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
/// annotated with the `nvsdk_ngx_defs_dlssg.h` parameter key it corresponds to.
/// Those keys are **absent from the pinned bindings**
/// (see [`MISSING_FG_PARAMETER_KEY_NAMES`]), so the fields are stored as raw
/// numeric values rather than as NGX types that do not exist here.
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

/// Header-verified DLSS-G parameter-key constants and enums from NVIDIA's
/// public `nvsdk_ngx_defs_dlssg.h` (DLSS SDK v310.9.16).
///
/// The generated bindings in `crate::nvsdk_ngx` were produced **without** this
/// header, so none of these `FrameGeneration.*` / `DLSSG.*` keys exist there.
/// Each key is nevertheless a plain `#define`d string literal, so reproducing it
/// verbatim as a NUL-terminated byte string is exact — no signature is invented
/// and no GPU/DLL/device work is performed. `ngx_param_key_ptr` turns any of
/// them into an ABI-safe `*const c_char` for the NGX `Set`/`Get` API.
pub mod dlssg_keys {
    /// Build a `&[u8]` NUL-terminated C-string constant from a string literal.
    macro_rules! key {
        ($name:ident, $lit:literal, $doc:literal) => {
            #[doc = $doc]
            pub const $name: &[u8] = concat!($lit, "\0").as_bytes();
        };
    }

    // ---- Capability / init keys (FrameGeneration.* / FrameInterpolation.*) ----
    key!(
        FRAME_GENERATION_AVAILABLE,
        "FrameGeneration.Available",
        "Max multi-frame count / availability (GetCapabilityParameters)."
    );
    key!(
        FRAME_GENERATION_FEATURE_INIT_RESULT,
        "FrameGeneration.FeatureInitResult",
        "Result of FG feature initialization."
    );
    key!(
        FRAME_GENERATION_MIN_DRIVER_VERSION_MAJOR,
        "FrameGeneration.MinDriverVersionMajor",
        "Minimum driver major version for FG."
    );
    key!(
        FRAME_GENERATION_MIN_DRIVER_VERSION_MINOR,
        "FrameGeneration.MinDriverVersionMinor",
        "Minimum driver minor version for FG."
    );
    key!(
        FRAME_GENERATION_NEEDS_UPDATED_DRIVER,
        "FrameGeneration.NeedsUpdatedDriver",
        "Whether the driver must be updated for FG."
    );
    key!(
        FRAME_INTERPOLATION_AVAILABLE,
        "FrameInterpolation.Available",
        "Frame-interpolation availability."
    );
    key!(
        FRAME_INTERPOLATION_FEATURE_INIT_RESULT,
        "FrameInterpolation.FeatureInitResult",
        "Frame-interpolation init result."
    );
    key!(
        FRAME_INTERPOLATION_MIN_DRIVER_VERSION_MAJOR,
        "FrameInterpolation.MinDriverVersionMajor",
        "Frame-interpolation min driver major."
    );
    key!(
        FRAME_INTERPOLATION_MIN_DRIVER_VERSION_MINOR,
        "FrameInterpolation.MinDriverVersionMinor",
        "Frame-interpolation min driver minor."
    );
    key!(
        FRAME_INTERPOLATION_NEEDS_UPDATED_DRIVER,
        "FrameInterpolation.NeedsUpdatedDriver",
        "Frame-interpolation driver-update flag."
    );

    // ---- Required / optional resource tags (DLSSG.*) ----
    key!(
        BACKBUFFER_FORMAT,
        "DLSSG.BackbufferFormat",
        "Format of the backbuffer resource."
    );
    key!(
        BACKBUFFER,
        "DLSSG.Backbuffer",
        "Input color/backbuffer resource."
    );
    key!(MVECS, "DLSSG.MVecs", "Input motion-vector resource.");
    key!(DEPTH, "DLSSG.Depth", "Input depth resource.");
    key!(HUDLESS, "DLSSG.HUDLess", "Scene color before UI/HUD (UIR).");
    key!(UI, "DLSSG.UI", "UI color + alpha resource.");
    key!(UI_ALPHA, "DLSSG.UIAlpha", "UI alpha-only resource.");
    key!(
        BIDIRECTIONAL_DISTORTION_FIELD,
        "DLSSG.BidirectionalDistortionField",
        "Lens-distortion mapping resource."
    );
    key!(
        OUTPUT_INTERPOLATED,
        "DLSSG.OutputInterpolated",
        "Required output: interpolated frame."
    );
    key!(
        OUTPUT_REAL,
        "DLSSG.OutputReal",
        "Optional output: real frame (debug text)."
    );
    key!(
        OUTPUT_DISABLE_INTERPOLATION,
        "DLSSG.OutputDisableInterpolation",
        "Snippet hint to skip interpolation this frame."
    );

    // ---- Create-time flags, dimensions, camera / depth / quality knobs ----
    key!(
        RESOURCE_ALWAYS_PROVIDED_FLAGS,
        "DLSSG.ResourceAlwaysProvidedFlags",
        "Bitmask of always-provided resources."
    );
    key!(
        RESOURCE_NEVER_PROVIDED_FLAGS,
        "DLSSG.ResourceNeverProvidedFlags",
        "Bitmask of never-provided resources."
    );
    key!(
        WIDTH,
        "DLSSG.Width",
        "DLFG width (avoids generic Width conflict)."
    );
    key!(
        HEIGHT,
        "DLSSG.Height",
        "DLFG height (avoids generic Height conflict)."
    );
    key!(RESET, "DLSSG.Reset", "Previous frame disconnected / reset.");
    key!(
        AUTOMODE_OVERRIDE_RESET,
        "DLSSG.AutomodeOverrideReset",
        "Automode forced Reset true."
    );
    key!(
        NOT_RENDERING_GAME_FRAMES,
        "DLSSG.NotRenderingGameFrames",
        "App paused / menu / cutscene."
    );
    key!(
        ASYNC_CREATE_ENABLED,
        "DLSSG.AsyncCreateEnabled",
        "Allow async internal resource creation."
    );
    key!(
        DEPTH_INVERTED,
        "DLSSG.DepthInverted",
        "Inverted depth (near = higher)."
    );
    key!(
        COLOR_BUFFERS_HDR,
        "DLSSG.ColorBuffersHDR",
        "Tagged color buffers are full HDR."
    );
    key!(
        CAMERA_MOTION_INCLUDED,
        "DLSSG.CameraMotionIncluded",
        "Motion vectors include camera motion."
    );
    key!(
        ORTHO_PROJECTION,
        "DLSSG.OrthoProjection",
        "Orthographic projection in use."
    );
    key!(
        MENU_DETECTION_ENABLED,
        "DLSSG.MenuDetectionEnabled",
        "Enable menu detection."
    );
    key!(EVAL_FLAGS, "DLSSG.EvalFlags", "DLSSG_EvalFlags bitmask.");
    key!(
        FULLSCREEN_MODE,
        "DLSSG.FullscreenMode",
        "DLSSG_FullscreenMode value."
    );
    key!(
        TARGET_FRAME_RATE,
        "DLSSG.TargetFrameRate",
        "Target frame rate for pacing."
    );

    // ---- DLSS 4 multi-frame generation ----
    key!(
        MULTI_FRAME_COUNT_MAX,
        "DLSSG.MultiFrameCountMax",
        "Max generated frames supported (capability)."
    );
    key!(
        MULTI_FRAME_COUNT,
        "DLSSG.MultiFrameCount",
        "Total interpolated frames for the real pair (1=2x, 3=4x)."
    );
    key!(
        MULTI_FRAME_INDEX,
        "DLSSG.MultiFrameIndex",
        "Current inner frame index (1..N)."
    );
    key!(
        BACKBUFFER_FRAME_ID,
        "DLSSG.BackbufferFrameID",
        "Monotonic backbuffer frame id."
    );
    key!(
        USER_INTERFACE_RECOMPOSITION_ENABLED,
        "DLSSG.UserInterfaceRecompositionEnabled",
        "Enable UIR frame generation."
    );
    key!(
        LITE_CANDIDATE_BLENDING_ENABLED,
        "DLSSG.LiteCandidateBlendingEnabled",
        "Enable lighter candidate blending."
    );

    /// All reproduced DLSS-G parameter keys (for validation/tests).
    pub const ALL: &[&[u8]] = &[
        FRAME_GENERATION_AVAILABLE,
        FRAME_GENERATION_FEATURE_INIT_RESULT,
        FRAME_GENERATION_MIN_DRIVER_VERSION_MAJOR,
        FRAME_GENERATION_MIN_DRIVER_VERSION_MINOR,
        FRAME_GENERATION_NEEDS_UPDATED_DRIVER,
        FRAME_INTERPOLATION_AVAILABLE,
        FRAME_INTERPOLATION_FEATURE_INIT_RESULT,
        FRAME_INTERPOLATION_MIN_DRIVER_VERSION_MAJOR,
        FRAME_INTERPOLATION_MIN_DRIVER_VERSION_MINOR,
        FRAME_INTERPOLATION_NEEDS_UPDATED_DRIVER,
        BACKBUFFER_FORMAT,
        BACKBUFFER,
        MVECS,
        DEPTH,
        HUDLESS,
        UI,
        UI_ALPHA,
        BIDIRECTIONAL_DISTORTION_FIELD,
        OUTPUT_INTERPOLATED,
        OUTPUT_REAL,
        OUTPUT_DISABLE_INTERPOLATION,
        RESOURCE_ALWAYS_PROVIDED_FLAGS,
        RESOURCE_NEVER_PROVIDED_FLAGS,
        WIDTH,
        HEIGHT,
        RESET,
        AUTOMODE_OVERRIDE_RESET,
        NOT_RENDERING_GAME_FRAMES,
        ASYNC_CREATE_ENABLED,
        DEPTH_INVERTED,
        COLOR_BUFFERS_HDR,
        CAMERA_MOTION_INCLUDED,
        ORTHO_PROJECTION,
        MENU_DETECTION_ENABLED,
        EVAL_FLAGS,
        FULLSCREEN_MODE,
        TARGET_FRAME_RATE,
        MULTI_FRAME_COUNT_MAX,
        MULTI_FRAME_COUNT,
        MULTI_FRAME_INDEX,
        BACKBUFFER_FRAME_ID,
        USER_INTERFACE_RECOMPOSITION_ENABLED,
        LITE_CANDIDATE_BLENDING_ENABLED,
    ];

    /// `NVSDK_NGX_DLSSG_ResourceFlags` bit values (v310.9.16 header).
    pub mod resource_flags {
        pub const BACKBUFFER: u32 = 1 << 0;
        pub const MVECS: u32 = 1 << 1;
        pub const DEPTH: u32 = 1 << 2;
        pub const HUDLESS: u32 = 1 << 3;
        pub const UI: u32 = 1 << 4;
        pub const BIDIRECTIONAL_DISTORTION_FIELD: u32 = 1 << 5;
        pub const RESERVED6: u32 = 1 << 6;
        pub const UI_ALPHA: u32 = 1 << 7;
        pub const OUTPUT_INTERPOLATED: u32 = 1 << 24;
        pub const OUTPUT_REAL: u32 = 1 << 25;
        pub const OUTPUT_DISABLE_INTERPOLATION: u32 = 1 << 26;
    }

    /// `NVSDK_NGX_DLSSG_EvalFlags` (only one value in v310.9.16).
    pub mod eval_flags {
        pub const UPDATE_ONLY_INSIDE_EXTENTS: u32 = 1 << 0;
    }

    /// `NVSDK_NGX_DLSSG_FullscreenMode` values.
    pub mod fullscreen_mode {
        pub const WINDOWED: u32 = 0;
        pub const BORDERLESS: u32 = 1;
        pub const EXCLUSIVE: u32 = 2;
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

/// The DLSS-G parameter keys from NVIDIA's public `nvsdk_ngx_defs_dlssg.h` are
/// **absent from the generated bindings** but are reproduced verbatim as
/// NUL-terminated C strings in [`dlssg_keys`] (header-verified, not invented).
///
/// What remains genuinely absent from the pinned bindings (and therefore from
/// this crate) are the `PFN_NVSDK_NGX_DLSSG_*` callback typedefs and any
/// DLSS-G-specific create/evaluate *function symbol*. Those require regenerating
/// bindings against `nvsdk_ngx_defs_dlssg.h` with libclang, or integrating
/// NVIDIA Streamline; they cannot be reconstructed from data alone. This is why
/// [`create_frame_generation`] returns `Unsupported`.
pub const BINDINGS_REGENERATION_REQUIRED_FOR: &[&str] = &[
    "PFN_NVSDK_NGX_DLSSG_GetCurrentSettingsCallback",
    "PFN_NVSDK_NGX_DLSSG_EstimateVRAMCallback",
    "DLSS-G-specific NVSDK_NGX_VULKAN_CreateFeature wrapper",
    "DLSS-G-specific NVSDK_NGX_VULKAN_EvaluateFeature wrapper",
    "NVSDK_NGX_DLSSG_ResourceFlags/EvalFlags/FullscreenMode C enums (constants provided in dlssg_keys)",
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

const NO_FG_SYMBOL: &str = "the pinned NGX bindings were generated without \
    nvsdk_ngx_defs_dlssg.h: they contain the FrameGeneration feature id (11), \
    three FrameInterpolation.* capability keys and generic VULKAN \
    CreateFeature/EvaluateFeature_C, but NO callable DLSS-G-specific \
    create/evaluate symbol and NO DLSSG.* parameter keys; no C signature is \
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
    fn header_verified_dlssg_keys_are_valid_c_strings() {
        use super::dlssg_keys as k;
        assert_eq!(k::ALL.len(), 43);
        for key in k::ALL {
            assert!(!key.is_empty());
            assert_eq!(key.last(), Some(&0), "key must be NUL-terminated: {key:?}");
            assert!(
                !key[..key.len() - 1].contains(&0),
                "key must have no interior NUL: {key:?}",
            );
            assert!(!ngx_param_key_ptr(key).is_null());
        }
        // Spot-check verbatim values from NVIDIA's public header.
        assert_eq!(
            &k::BACKBUFFER[..k::BACKBUFFER.len() - 1],
            b"DLSSG.Backbuffer"
        );
        assert_eq!(
            &k::MULTI_FRAME_COUNT[..k::MULTI_FRAME_COUNT.len() - 1],
            b"DLSSG.MultiFrameCount"
        );
        assert_eq!(
            &k::FRAME_GENERATION_AVAILABLE[..k::FRAME_GENERATION_AVAILABLE.len() - 1],
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
