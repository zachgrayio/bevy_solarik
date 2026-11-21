//! CPU-only checks that never open a renderer, window or GPU device. The pinned
//! `#[repr(C)]` layout assertions live in `src/native.rs` and fail compilation.

use solarik_fsr_wgpu::effects::{desc_types, versions};
use solarik_fsr_wgpu::{Error, Library, QualityMode, effect_sub_id, formats, surface_format};

#[test]
fn effect_sub_ids_match_the_sdk_macros() {
    assert_eq!(effect_sub_id(0x0001_0000, 0x00), 0x0001_0000);
    assert_eq!(effect_sub_id(0x0001_0000, 0x01), 0x0001_0001);
    assert_eq!(effect_sub_id(0x0001_0000, 0x05), 0x0001_0005);
}

#[test]
fn quality_modes_match_the_sdk_enum_and_ratios() {
    use QualityMode::{Balanced, NativeAA, Performance, Quality, UltraPerformance};
    assert_eq!(
        [NativeAA, Quality, Balanced, Performance, UltraPerformance].map(QualityMode::as_raw),
        [0, 1, 2, 3, 4]
    );
    assert_eq!(NativeAA.ratio(), 1.0);
    assert_eq!(Quality.ratio(), 1.5);
    assert_eq!(Balanced.ratio(), 1.7);
    assert_eq!(Performance.ratio(), 2.0);
    assert_eq!(UltraPerformance.ratio(), 3.0);
}

#[test]
fn native_aa_renders_at_display_resolution() {
    let display = [1920, 1080];
    assert_eq!(QualityMode::NativeAA.render_resolution(display), display);
    assert_eq!(
        QualityMode::Performance.render_resolution(display),
        [960, 540]
    );
    assert_eq!(
        QualityMode::UltraPerformance.render_resolution(display),
        [640, 360]
    );
}

#[test]
fn mip_bias_and_jitter_stay_bounded() {
    assert_eq!(QualityMode::NativeAA.suggested_mip_bias(1920), -1.0);
    assert!((QualityMode::Performance.suggested_mip_bias(1920) + 2.0).abs() < 1e-4);
    for frame in 0..64 {
        let [x, y] = QualityMode::Quality.suggested_jitter(frame, 16);
        assert!((-0.5..=0.5).contains(&x));
        assert!((-0.5..=0.5).contains(&y));
    }
    assert!(QualityMode::Performance.jitter_phase_count(1920) >= 8);
}

#[test]
fn surface_formats_follow_the_sdk_enum() {
    assert_eq!(
        surface_format(wgpu::TextureFormat::Rgba16Float),
        formats::R16G16B16A16_FLOAT
    );
    assert_eq!(
        surface_format(wgpu::TextureFormat::R32Float),
        formats::R32_FLOAT
    );
    assert_eq!(
        surface_format(wgpu::TextureFormat::Rgba8Unorm),
        formats::R8G8B8A8_UNORM
    );
    assert_eq!(surface_format(wgpu::TextureFormat::Rgba16Float), 4);
    assert_eq!(
        surface_format(wgpu::TextureFormat::R8Snorm),
        formats::UNKNOWN
    );
}

#[test]
fn loading_is_dynamic_and_never_panics_without_a_provider() {
    // No AMD DLL is a build/link input; a missing provider is a plain Result.
    match Library::load() {
        Ok(_) => assert!(Library::is_available()),
        Err(Error::Library(_) | Error::Symbol(_, _)) => {}
        Err(other) => panic!("unexpected FSR load error: {other}"),
    }
}

#[test]
fn frame_generation_swapchain_versions_match_the_sdk_headers() {
    // FFX_*_SWAPCHAIN_*_VERSION = MAKE_VERSION(4, 0, 0) = (4<<22)|(0<<12)|0.
    assert_eq!(versions::FRAMEGENERATION_SWAPCHAIN_VK, 0x0100_0000);
    assert_eq!(versions::FRAMEGENERATION_SWAPCHAIN_DX12, 0x0100_0000);
    // Retained pinned versions used by the original integration.
    assert_eq!(versions::UPSCALER, 0x0100_1002);
    assert_eq!(versions::DENOISER, 0x0040_2001);
    assert_eq!(versions::RADIANCECACHE, 0x0000_9000);
    assert_eq!(versions::FRAMEGENERATION, 0x0100_0002);
}

#[test]
fn fgswapchain_vk_descriptor_types_match_the_sdk_headers() {
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
}

#[test]
fn fgswapchain_dx12_and_frame_generation_descriptor_types_match_the_headers() {
    assert_eq!(desc_types::FGSWAPCHAIN_DX12_WRAP_CREATE, 0x0003_0001);
    assert_eq!(desc_types::FGSWAPCHAIN_DX12_VERSION, 0x0003_000b);
    // 0x02 is the CONFIGURE descriptor in 2.4.0 (not the prepare dispatch).
    assert_eq!(desc_types::FRAMEGENERATION_CONFIGURE, 0x0002_0002);
    assert_eq!(desc_types::FRAMEGENERATION_PREPARE, 0x0002_0004);
    assert_eq!(desc_types::FRAMEGENERATION_PRESENT_CALLBACK, 0x0002_0005);
    assert_eq!(desc_types::FRAMEGENERATION_PREPARE_V2, 0x0002_000c);
}

#[test]
fn denoiser_and_radiance_cache_descriptor_types_match_the_headers() {
    assert_eq!(desc_types::DENOISER_DISPATCH, 0x0005_0041);
    assert_eq!(desc_types::DENOISER_DISPATCH_DEBUG_VIEW, 0x0005_0042);
    assert_eq!(desc_types::DENOISER_DISPATCH_AMBIENT_OCCLUSION, 0x0005_0043);
    assert_eq!(desc_types::DENOISER_DISPATCH_DIRECT_DIFFUSE, 0x0005_0044);
    assert_eq!(desc_types::DENOISER_DISPATCH_DIRECT_SPECULAR, 0x0005_0045);
    assert_eq!(desc_types::DENOISER_DISPATCH_DOMINANT_LIGHT, 0x0005_0046);
    assert_eq!(desc_types::DENOISER_DISPATCH_INDIRECT_DIFFUSE, 0x0005_0047);
    assert_eq!(desc_types::DENOISER_DISPATCH_INDIRECT_SPECULAR, 0x0005_0048);
    assert_eq!(
        desc_types::DENOISER_DISPATCH_SPECULAR_OCCLUSION,
        0x0005_0049
    );
    assert_eq!(desc_types::DENOISER_QUERY_DEFAULT_KEYVALUE, 0x0005_0081);
    assert_eq!(desc_types::DENOISER_QUERY_GPU_MEMORY_USAGE_2, 0x0005_0082);
    assert_eq!(desc_types::RADIANCECACHE_DEBUG_DISPATCH, 0x0006_0005);
}
