//! Per-backend upscaler/denoiser capability probes for the hybrid renderer.
//!
//! These resources mirror `bevy_anti_alias::dlss`'s `Dlss*Supported` flags but
//! are split by backend (Vulkan / DX12) and provider (DLSS / FSR). They are
//! inserted by a backend-specific init plugin *before* device creation is
//! finalized; the values are populated from `ffxQuery` / NGX capability queries.
//!
//! All flags default to `false`: probing a provider that is not present must
//! never advertise it as available. The consuming application inserts these
//! resources and its combo resolver degrades gracefully (see the design
//! writeup §2.4). No GPU device, window or provider is opened by this module,
//! so its unit tests run headless.

use bevy_ecs::prelude::Resource;

/// Which render backend the active device was created with. One wgpu device has
/// exactly one backend; switching requires a process restart.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActiveBackend {
    /// Vulkan (the validated Solari/DLSS path; also Linux and Apple fallback).
    #[default]
    Vulkan,
    /// DirectX 12 (unlocks AMD Ray Regeneration / radiance cache / native NGX).
    Dx12,
    /// Metal (Apple, raster only — no Solari plugins).
    Metal,
}

/// Aggregate, backend-agnostic capability set. Individual flags are grouped by
/// backend below; this resource is what the Options UI reads for its status
/// line and what the combo resolver consumes as `Capabilities`.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RendererCapabilities {
    /// Vulkan DLSS Super Resolution.
    pub vulkan_dlss_sr: bool,
    /// Vulkan DLSS Ray Reconstruction (joint denoiser + upscaler).
    pub vulkan_dlss_rr: bool,
    /// Vulkan FSR Upscaling 4.
    pub vulkan_fsr_upscaler: bool,
    /// Vulkan NRD (RELAX) denoiser.
    pub vulkan_nrd: bool,
    /// Vulkan FSR frame generation.
    pub vulkan_fsr_frame_generation: bool,
    /// Vulkan DLSS-G frame generation (via Streamline).
    pub vulkan_dlss_frame_generation: bool,
    /// DX12 DLSS Super Resolution (D3D12 NGX).
    pub dx12_dlss_sr: bool,
    /// DX12 DLSS Ray Reconstruction (D3D12 NGX).
    pub dx12_dlss_rr: bool,
    /// DX12 FSR Upscaling 4.
    pub dx12_fsr_upscaler: bool,
    /// DX12 FSR Ray Regeneration denoiser.
    pub dx12_fsr_ray_regeneration: bool,
    /// DX12 FSR Radiance Caching.
    pub dx12_fsr_radiance_cache: bool,
    /// DX12 FSR frame generation.
    pub dx12_fsr_frame_generation: bool,
    /// DX12 DLSS-G / multi-frame generation.
    pub dx12_dlss_frame_generation: bool,
}

impl RendererCapabilities {
    /// True when any frame-generation provider is available on either backend.
    pub fn frame_generation_supported(self) -> bool {
        self.vulkan_fsr_frame_generation
            || self.vulkan_dlss_frame_generation
            || self.dx12_fsr_frame_generation
            || self.dx12_dlss_frame_generation
    }
}

/// Named, per-flag resources matching the D4 naming from the design writeup.
/// They are thin wrappers so existing `Res<...>`-style consumers can require a
/// single capability; `RendererCapabilities` remains the aggregate source.
macro_rules! capability_resource {
    ($(#[$meta:meta])* $name:ident, $field:ident) => {
        $(#[$meta])*
        #[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct $name(pub bool);
    };
}

capability_resource!(
    /// Vulkan DLSS Super Resolution availability.
    VulkanDlssSuperResolutionSupported, vulkan_dlss_sr
);
capability_resource!(
    /// Vulkan DLSS Ray Reconstruction availability.
    VulkanDlssRayReconstructionSupported, vulkan_dlss_rr
);
capability_resource!(
    /// Vulkan FSR Upscaling 4 availability.
    VulkanFsrUpscalerSupported, vulkan_fsr_upscaler
);
capability_resource!(
    /// Vulkan NRD denoiser availability.
    VulkanNrdSupported, vulkan_nrd
);
capability_resource!(
    /// Vulkan FSR frame generation availability.
    VulkanFsrFrameGenerationSupported, vulkan_fsr_frame_generation
);
capability_resource!(
    /// Vulkan DLSS-G frame generation availability.
    VulkanDlssFrameGenerationSupported, vulkan_dlss_frame_generation
);
capability_resource!(
    /// DX12 DLSS Super Resolution availability.
    Dx12DlssSuperResolutionSupported, dx12_dlss_sr
);
capability_resource!(
    /// DX12 DLSS Ray Reconstruction availability.
    Dx12DlssRayReconstructionSupported, dx12_dlss_rr
);
capability_resource!(
    /// DX12 FSR Upscaling 4 availability.
    Dx12FsrUpscalerSupported, dx12_fsr_upscaler
);
capability_resource!(
    /// DX12 FSR Ray Regeneration availability.
    Dx12FsrRayRegenerationSupported, dx12_fsr_ray_regeneration
);
capability_resource!(
    /// DX12 FSR Radiance Caching availability.
    Dx12FsrRadianceCacheSupported, dx12_fsr_radiance_cache
);
capability_resource!(
    /// DX12 FSR frame generation availability.
    Dx12FsrFrameGenerationSupported, dx12_fsr_frame_generation
);
capability_resource!(
    /// DX12 DLSS-G / multi-frame generation availability.
    Dx12DlssFrameGenerationSupported, dx12_dlss_frame_generation
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_capabilities_default_to_unavailable() {
        let caps = RendererCapabilities::default();
        assert!(!caps.vulkan_dlss_sr);
        assert!(!caps.dx12_fsr_ray_regeneration);
        assert!(!caps.frame_generation_supported());
    }

    #[test]
    fn frame_generation_is_the_union_of_all_providers() {
        let none = RendererCapabilities::default();
        for caps in [
            RendererCapabilities {
                vulkan_fsr_frame_generation: true,
                ..none
            },
            RendererCapabilities {
                vulkan_dlss_frame_generation: true,
                ..none
            },
            RendererCapabilities {
                dx12_fsr_frame_generation: true,
                ..none
            },
            RendererCapabilities {
                dx12_dlss_frame_generation: true,
                ..none
            },
        ] {
            assert!(caps.frame_generation_supported());
        }
    }

    #[test]
    fn active_backend_defaults_to_vulkan() {
        assert_eq!(ActiveBackend::default(), ActiveBackend::Vulkan);
        assert_ne!(ActiveBackend::Dx12, ActiveBackend::Metal);
    }
}
