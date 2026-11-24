//! Quality modes and the camera-side numbers an application needs to drive FSR.
//!
//! This mirrors `dlss_wgpu::DlssPerfQualityMode`: the upscaler tells the camera
//! what resolution to render at, what subpixel jitter to apply and what mip bias
//! to use. The engine owns `TemporalJitter` and `MipBias`; this crate only computes.

/// FSR upscaling preset. Values are the `FfxApiUpscaleQualityMode` enum from
/// `Kits/FidelityFX/upscalers/include/ffx_upscale.h`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QualityMode {
    /// 1.0x per dimension: temporal anti-aliasing at native resolution (FSR "NativeAA").
    NativeAA = 0,
    /// 1.5x per dimension.
    Quality = 1,
    /// 1.7x per dimension.
    Balanced = 2,
    /// 2.0x per dimension.
    #[default]
    Performance = 3,
    /// 3.0x per dimension.
    UltraPerformance = 4,
}

impl QualityMode {
    /// The raw `FfxApiUpscaleQualityMode` value passed to `ffxQuery`.
    pub const fn as_raw(self) -> u32 {
        self as u32
    }

    /// The documented per-dimension upscaling ratio for this preset.
    pub const fn ratio(self) -> f32 {
        match self {
            Self::NativeAA => 1.0,
            Self::Quality => 1.5,
            Self::Balanced => 1.7,
            Self::Performance => 2.0,
            Self::UltraPerformance => 3.0,
        }
    }

    /// CPU-side render resolution for a display size. A live context's
    /// `ffxQuery` answer is authoritative; see [`crate::Context::query_render_resolution`].
    pub fn render_resolution(self, display: [u32; 2]) -> [u32; 2] {
        let ratio = self.ratio();
        [
            (display[0] as f32 / ratio).floor().max(1.0) as u32,
            (display[1] as f32 / ratio).floor().max(1.0) as u32,
        ]
    }

    /// Halton jitter for a frame, in `[-0.5, 0.5]` pixel units, matching Bevy's
    /// `TemporalJitter` convention. `phase_count` comes from
    /// [`crate::Context::query_jitter_phase_count`] or [`Self::jitter_phase_count`].
    pub fn suggested_jitter(self, frame_index: u32, phase_count: u32) -> [f32; 2] {
        let phase_count = phase_count.max(1);
        let index = (frame_index % phase_count) + 1;
        [halton(index, 2) - 0.5, halton(index, 3) - 0.5]
    }

    /// Texture mip bias that keeps surface detail stable at the lower render
    /// resolution. Identical to DLSS Super Resolution's `suggested_mip_bias`.
    pub fn suggested_mip_bias(self, display_width: u32) -> f32 {
        let render_width = self.render_resolution([display_width, 1])[0].max(1);
        (render_width as f32 / display_width.max(1) as f32).log2() - 1.0
    }

    /// Number of jitter phases for the given render/display width pair, used when
    /// the provider's `ffxQuery` answer is unavailable. FSR uses 8 phases for a
    /// 2x upscale and grows quadratically from there; NativeAA keeps 8.
    pub fn jitter_phase_count(self, display_width: u32) -> u32 {
        let render_width = self.render_resolution([display_width, 1])[0].max(1);
        let ratio = display_width.max(1) as f32 / render_width as f32;
        let exponent = ratio.log2().ceil().max(0.0) as u32;
        (1u32 << (2 * exponent + 1).min(8)).max(8)
    }
}

fn halton(mut index: u32, base: u32) -> f32 {
    let mut f = 1.0;
    let mut result = 0.0;
    while index > 0 {
        f /= base as f32;
        result += f * (index % base) as f32;
        index /= base;
    }
    result
}

/// FfxApi create-context flags, from `FfxApiCreateContextUpscaleFlags`.
pub mod flags {
    /// Input color is high-dynamic-range linear (typical for HDR render targets).
    pub const HIGH_DYNAMIC_RANGE: u32 = 1 << 0;
    /// Motion vectors are supplied at display resolution.
    pub const DISPLAY_RESOLUTION_MOTION_VECTORS: u32 = 1 << 1;
    /// Motion vectors already include the jitter cancellation.
    pub const MOTION_VECTORS_JITTER_CANCELLATION: u32 = 1 << 2;
    /// Depth is inverted `[1..0]`.
    pub const DEPTH_INVERTED: u32 = 1 << 3;
    /// Depth uses an infinite far plane.
    pub const DEPTH_INFINITE: u32 = 1 << 4;
    /// Let the upscaler compute exposure from the color buffer.
    pub const AUTO_EXPOSURE: u32 = 1 << 5;
    /// Application changes render resolution at runtime.
    pub const DYNAMIC_RESOLUTION: u32 = 1 << 6;
    /// Provider validates descriptor values (development only).
    pub const DEBUG_CHECKING: u32 = 1 << 7;
    /// Input color is perceptual (gamma-corrected).
    pub const NON_LINEAR_COLORSPACE: u32 = 1 << 8;
    /// Allow debug visualizations (uses more GPU memory).
    pub const DEBUG_VISUALIZATION: u32 = 1 << 9;
}

/// FfxApi dispatch flags, from `FfxApiDispatchFsrUpscaleFlags`.
pub mod dispatch_flags {
    /// Write debug views into the output resource.
    pub const DRAW_DEBUG_VIEW: u32 = 1 << 0;
    /// Input color is perceptual sRGB.
    pub const NON_LINEAR_COLOR_SRGB: u32 = 1 << 1;
    /// Input color is perceptual PQ.
    pub const NON_LINEAR_COLOR_PQ: u32 = 1 << 2;
}

/// FfxApi resource states, from `FfxApiResourceState`.
pub mod states {
    /// Resource is in `COMPUTE_READ` state (the upscaler reads it).
    pub const COMPUTE_READ: u32 = 1 << 2;
    /// Resource is in `UNORDERED_ACCESS` state (the upscaler writes it).
    pub const UNORDERED_ACCESS: u32 = 1 << 1;
    /// Resource is in `PIXEL_COMPUTE_READ` state.
    pub const PIXEL_COMPUTE_READ: u32 = (1 << 2) | (1 << 3);
}

/// FfxApi resource usage, from `FfxApiResourceUsage`.
pub mod usage {
    /// Read-only resource.
    pub const READ_ONLY: u32 = 0;
    /// Resource used as a render target.
    pub const RENDER_TARGET: u32 = 1 << 0;
    /// Resource used as an unordered-access view (storage texture).
    pub const UAV: u32 = 1 << 1;
    /// Resource used as a depth target (required for depth aspect views on Vulkan).
    pub const DEPTH_TARGET: u32 = 1 << 2;
    /// Resource used as a stencil target.
    pub const STENCIL_TARGET: u32 = 1 << 5;
}

/// FfxApi surface formats, from `FfxApiSurfaceFormat` in `ffx_api_types.h`.
/// Values are sequential enum ordinals; keep them in sync with the pinned header.
pub mod formats {
    /// Unknown/untyped.
    pub const UNKNOWN: u32 = 0;
    /// `R32G32B32A32_FLOAT`.
    pub const R32G32B32A32_FLOAT: u32 = 3;
    /// `R16G16B16A16_FLOAT`.
    pub const R16G16B16A16_FLOAT: u32 = 4;
    /// `R32G32_FLOAT`.
    pub const R32G32_FLOAT: u32 = 6;
    /// `R8G8B8A8_UNORM`.
    pub const R8G8B8A8_UNORM: u32 = 10;
    /// `R11G11B10_FLOAT`.
    pub const R11G11B10_FLOAT: u32 = 16;
    /// `R16G16_FLOAT`.
    pub const R16G16_FLOAT: u32 = 18;
    /// `R16_FLOAT`.
    pub const R16_FLOAT: u32 = 21;
    /// `R8_UNORM`.
    pub const R8_UNORM: u32 = 25;
    /// `R32_FLOAT`.
    pub const R32_FLOAT: u32 = 28;
}

/// FfxApi resource types, from `FfxApiResourceType`.
pub mod resource_types {
    /// A 2D texture.
    pub const TEXTURE_2D: u32 = 2;
}

/// Map a `wgpu::TextureFormat` to a FfxApi surface format. Returns
/// [`formats::UNKNOWN`] for formats FSR does not accept.
pub fn surface_format(format: wgpu::TextureFormat) -> u32 {
    use wgpu::TextureFormat as F;
    match format {
        F::Rgba32Float => formats::R32G32B32A32_FLOAT,
        F::Rgba16Float => formats::R16G16B16A16_FLOAT,
        F::Rg32Float => formats::R32G32_FLOAT,
        F::Rgba8Unorm | F::Rgba8UnormSrgb => formats::R8G8B8A8_UNORM,
        F::Rg11b10Ufloat => formats::R11G11B10_FLOAT,
        F::Rg16Float => formats::R16G16_FLOAT,
        F::R16Float => formats::R16_FLOAT,
        F::R8Unorm => formats::R8_UNORM,
        F::R32Float | F::Depth32Float | F::Depth24Plus | F::Depth16Unorm => formats::R32_FLOAT,
        _ => formats::UNKNOWN,
    }
}

/// FfxApi struct-type constants. Kept here so the ABI tests can assert them.
pub const fn effect_sub_id(effect: u32, sub: u32) -> u64 {
    const EFFECT_MASK: u64 = 0x00ff_0000;
    ((effect as u64) & EFFECT_MASK) | ((sub as u64) & !EFFECT_MASK)
}
