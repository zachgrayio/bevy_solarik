use std::{
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

fn require_artifact(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
    let mut magic = [0; 8];
    let mut file = File::open(path).unwrap_or_else(|error| {
        panic!(
            "Missing NRD artifact {}: {error}. Run git lfs pull in the application checkout.",
            path.display()
        )
    });
    file.read_exact(&mut magic).expect("Native library header");
    assert!(
        &magic != b"version ",
        "{} is a Git LFS pointer. Run git lfs pull in the application checkout.",
        path.display()
    );
}

fn main() {
    println!("cargo:rerun-if-env-changed=NRD_SDK");
    println!("cargo:rerun-if-env-changed=NRD_LIB_DIR");
    println!("cargo:rerun-if-env-changed=BARD_NATIVE_LIBS");
    println!("cargo:rerun-if-changed=src/bridge.h");
    println!("cargo:rerun-if-changed=src/bridge.cpp");
    let sdk = env::var_os("NRD_SDK").map(PathBuf::from);
    let target = env::var("TARGET").expect("Cargo target");
    let lib = env::var_os("NRD_LIB_DIR").map(PathBuf::from).or_else(|| sdk.as_ref().map(|sdk| sdk.join("_Bin")))
        .unwrap_or_else(|| PathBuf::from(env::var_os("BARD_NATIVE_LIBS").expect("Set BARD_NATIVE_LIBS to the application native artifact directory, or NRD_SDK to a source-built SDK; see docs/nrd.md")).join(&target).join("nrd"));
    assert!(lib.is_absolute(), "NRD library directory must be absolute");
    let windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    for name in ["NRD", "ShaderMakeBlob"] {
        require_artifact(&lib.join(if windows {
            format!("{name}.lib")
        } else {
            format!("lib{name}.a")
        }));
    }
    if let Some(sdk) = sdk {
        assert!(sdk.is_absolute(), "NRD_SDK must be absolute");
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .debug(false)
            .file("src/bridge.cpp")
            .include(sdk.join("Include"))
            .compile("solarik_nrd_bridge");
    } else {
        require_artifact(&lib.join(if windows {
            "solarik_nrd_bridge.lib"
        } else {
            "libsolarik_nrd_bridge.a"
        }));
        println!("cargo:rustc-link-lib=static=solarik_nrd_bridge");
    }
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=NRD");
    println!("cargo:rustc-link-lib=static=ShaderMakeBlob");
    if !windows {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
}
