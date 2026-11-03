use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=NRD_SDK");
    println!("cargo:rerun-if-env-changed=NRD_LIB_DIR");
    println!("cargo:rerun-if-changed=src/bridge.h");
    println!("cargo:rerun-if-changed=src/bridge.cpp");
    let sdk = PathBuf::from(
        env::var_os("NRD_SDK")
            .expect("Set NRD_SDK to the v4.18.0 source checkout; see docs/nrd.md"),
    );
    assert!(sdk.is_absolute(), "NRD_SDK must be absolute");
    let lib = env::var_os("NRD_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| sdk.join("_Bin"));
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .debug(false)
        .file("src/bridge.cpp")
        .include(sdk.join("Include"))
        .compile("solarik_nrd_bridge");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=NRD");
    println!("cargo:rustc-link-lib=static=ShaderMakeBlob");
}
