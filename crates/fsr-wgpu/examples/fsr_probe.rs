//! Probe an installed AMD FidelityFX FSR provider.
//!
//! ```text
//! cargo run -p solarik-fsr-wgpu --example fsr_probe
//! FSR_DLL=C:\path\to\amd_fidelityfx_vk.dll cargo run -p solarik-fsr-wgpu --example fsr_probe
//! ```
//!
//! Set `FSR_DLL`, `FSR_RUNTIME_DIR` or `FSR_NATIVE_LIBS` to locate the provider.
//! Without one, the probe reports FSR unavailable — the same graceful fallback a
//! consuming application uses. This example opens no window or GPU device, so it
//! cannot create an FSR context; the upscaler's `ffxQuery` helpers live on
//! `Context` and need a live Vulkan device.

use solarik_fsr_wgpu::{Library, QualityMode};

fn main() {
    match Library::load() {
        Ok(_) => println!("FSR provider loaded"),
        Err(error) => {
            println!("FSR provider unavailable: {error}");
            println!("Set FSR_DLL / FSR_RUNTIME_DIR / FSR_NATIVE_LIBS to enable FSR.");
            return;
        }
    }
    let display = [1920, 1080];
    for mode in [
        QualityMode::NativeAA,
        QualityMode::Quality,
        QualityMode::Balanced,
        QualityMode::Performance,
        QualityMode::UltraPerformance,
    ] {
        println!(
            "{mode:?}: ratio={:.1}x, render={:?}, mip_bias={:.2}, jitter_phases={}",
            mode.ratio(),
            mode.render_resolution(display),
            mode.suggested_mip_bias(display[0]),
            mode.jitter_phase_count(display[0]),
        );
    }
}
