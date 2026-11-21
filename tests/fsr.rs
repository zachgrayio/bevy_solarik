//! CPU-only FSR checks. These open no renderer, window or GPU device.

#![cfg(feature = "fsr")]

use bevy_solarik::realtime::{FsrQualityMode, SolarikLighting};

#[test]
fn fsr_is_opt_in_and_defaults_to_performance() {
    let lighting = SolarikLighting::default();
    assert!(
        !lighting.fsr,
        "FSR must not change existing views by default"
    );
    assert_eq!(lighting.fsr_quality, FsrQualityMode::Performance);
    assert_eq!(lighting.fsr_sharpness, 0.0);
}

#[test]
fn quality_presets_match_the_dlss_mode_ladder() {
    use FsrQualityMode::{Balanced, NativeAA, Performance, Quality, UltraPerformance};
    let display = [1920, 1080];
    assert_eq!(NativeAA.render_resolution(display), display);
    assert_eq!(Quality.render_resolution(display), [1280, 720]);
    assert_eq!(Balanced.render_resolution(display)[0], 1129);
    assert_eq!(Performance.render_resolution(display), [960, 540]);
    assert_eq!(UltraPerformance.render_resolution(display), [640, 360]);

    // Jitter stays inside Bevy's TemporalJitter range and mip bias is negative.
    for frame in 0..32 {
        let [x, y] = Performance.suggested_jitter(frame, display[0]);
        assert!((-0.5..=0.5).contains(&x));
        assert!((-0.5..=0.5).contains(&y));
    }
    assert!(Performance.suggested_mip_bias(display[0]) < 0.0);
    assert_eq!(NativeAA.suggested_mip_bias(display[0]), -1.0);
}

#[test]
fn compose_shader_is_valid_wgsl() {
    let source = include_str!("../src/realtime/fsr.wesl");
    let module = wgpu::naga::front::wgsl::parse_str(source).expect("fsr.wesl must be valid WGSL");
    assert!(
        module
            .entry_points
            .iter()
            .any(|entry| entry.name == "compose_fsr")
    );
}
