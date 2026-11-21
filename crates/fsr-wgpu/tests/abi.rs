//! CPU-only checks that never open a renderer, window or GPU device. The pinned
//! `#[repr(C)]` layout assertions live in `src/native.rs` and fail compilation.

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
