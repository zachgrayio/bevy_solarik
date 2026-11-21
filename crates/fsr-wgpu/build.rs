//! Build-time artifact discovery for the optional FSR provider.
//!
//! No AMD binary is linked here. The runtime loader in `native.rs` searches, in
//! order: `FSR_DLL`, `FSR_RUNTIME_DIR`, the pinned
//! `FSR_NATIVE_LIBS/<TARGET>/ffx` directory and finally the platform loader
//! path (executable directory). Missing artifacts are a runtime capability
//! signal, never a build failure — unlike DLSS/NRD, FSR is dynamically loaded.

use std::path::PathBuf;

fn main() {
    for key in [
        "FSR_NATIVE_LIBS",
        "FSR_DLL",
        "FSR_RUNTIME_DIR",
        "FSR_BACKEND_VK_TYPE",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/abi.h");

    let target = std::env::var("TARGET").unwrap_or_default();
    if let Ok(root) = std::env::var("FSR_NATIVE_LIBS") {
        let dir = PathBuf::from(root).join(&target).join("ffx");
        println!("cargo:rustc-env=FSR_NATIVE_DIR={}", dir.display());
    }
    if let Ok(dir) = std::env::var("FSR_RUNTIME_DIR") {
        println!("cargo:rustc-env=FSR_RUNTIME_DIR={dir}");
    }
    if let Ok(dll) = std::env::var("FSR_DLL") {
        println!("cargo:rustc-env=FSR_DLL={dll}");
    }
}
