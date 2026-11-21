//! Native link and deployment requirements for optional DLSS/FSR consumers.

#[path = "build_support/native.rs"]
mod native;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_support/native.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var_os("CARGO_FEATURE_DLSS").is_some()
    {
        // NGX's registry imports are not declared by dlss_wgpu 6. Standalone
        // consumers must not rely on a window plugin linking them incidentally.
        println!("cargo:rustc-link-lib=advapi32");
    }
    // FSR needs no link libraries: solarik-fsr-wgpu dlopens the provider. This
    // only deploys the optional pinned runtime modules beside built binaries.
    native::configure();
}
