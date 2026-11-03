use std::{
    env,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

fn require_artifact(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
    let mut header = [0; 8];
    File::open(path)
        .unwrap_or_else(|error| {
            panic!(
                "Missing DLSS artifact {}: {error}. Run git lfs pull in the application checkout.",
                path.display()
            )
        })
        .read_exact(&mut header)
        .expect("Native library header");
    assert!(
        &header != b"version ",
        "{} is a Git LFS pointer. Run git lfs pull in the application checkout.",
        path.display()
    );
}

fn bundled(out: &Path, target: &str, os: &str) {
    let root = PathBuf::from(env::var_os("BARD_NATIVE_LIBS").expect("Set BARD_NATIVE_LIBS to the application native artifact directory, or DLSS_SDK to a complete SDK"));
    assert!(
        root.is_absolute(),
        "Native artifact directory must be absolute"
    );
    assert!(
        matches!(
            target,
            "x86_64-pc-windows-msvc" | "x86_64-unknown-linux-gnu"
        ),
        "No pinned DLSS artifacts for {target}"
    );
    let lib = root.join(target).join("dlss");
    let windows = os == "windows";
    let static_crt = env::var("CARGO_CFG_TARGET_FEATURE")
        .unwrap_or_default()
        .split(',')
        .any(|f| f == "crt-static");
    let ngx = if windows {
        if static_crt {
            "nvsdk_ngx_s"
        } else {
            "nvsdk_ngx_d"
        }
    } else {
        "nvsdk_ngx"
    };
    for name in ["wrap_static_fns", ngx] {
        require_artifact(&lib.join(if windows {
            format!("{name}.lib")
        } else {
            format!("lib{name}.a")
        }));
    }
    let bindings = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/bindings")
        .join(format!("{target}.rs"));
    println!("cargo:rerun-if-changed={}", bindings.display());
    fs::copy(bindings, out.join("bindings.rs"))
        .expect("Copy target-specific generated DLSS bindings");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=wrap_static_fns");
    println!("cargo:rustc-link-lib=static={ngx}");
    if !windows {
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=dylib=dl");
    }
    println!("cargo:rustc-env=DLSS_RUNTIME_DIR={}", lib.display());
}

fn main() {
    for name in [
        "BARD_NATIVE_LIBS",
        "DLSS_SDK",
        "VULKAN_SDK",
        "LIBCLANG_PATH",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    if cfg!(feature = "mock") {
        return;
    }

    // Get SDK paths
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Determine platform info
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let platform = if target_os == "windows" {
        format!("Windows_{target_arch}")
    } else {
        format!("Linux_{target_arch}")
    };
    println!("cargo:rustc-env=DLSS_SDK_PLATFORM={platform}");
    let Ok(dlss_sdk) = env::var("DLSS_SDK") else {
        bundled(
            &out_dir,
            &env::var("TARGET").expect("Cargo target"),
            &target_os,
        );
        return;
    };
    let vulkan_sdk = env::var("VULKAN_SDK")
        .expect("VULKAN_SDK is required only when overriding DLSS_SDK to regenerate bindings");
    println!("cargo:rustc-env=DLSS_RUNTIME_DIR={dlss_sdk}/lib/{platform}/rel");

    // Link to needed libraries
    let mut lib_dir = format!("{dlss_sdk}/lib/{platform}");
    if target_os == "windows" && target_arch == "x86_64" {
        lib_dir.push_str("/x64");
    }
    if !Path::new(&lib_dir).is_dir() {
        panic!("DLSS SDK at {dlss_sdk} has no libraries for {platform}");
    }
    println!("cargo:rustc-link-search=native={lib_dir}");
    if target_os == "windows" {
        #[cfg(not(target_feature = "crt-static"))]
        println!("cargo:rustc-link-lib=static=nvsdk_ngx_d");
        #[cfg(target_feature = "crt-static")]
        println!("cargo:rustc-link-lib=static=nvsdk_ngx_s");
    } else {
        println!("cargo:rustc-link-lib=static=nvsdk_ngx");
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=dylib=dl");
    }

    let bindings = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/bindings")
        .join(format!("{}.rs", env::var("TARGET").expect("Cargo target")));
    println!("cargo:rerun-if-changed={}", bindings.display());
    fs::copy(bindings, out_dir.join("bindings.rs")).expect("Copy target-specific DLSS 310.9.1 ABI");
    println!("cargo:rerun-if-changed=src/wrapper.h");
    println!("cargo:rerun-if-changed=src/wrap_static_fns.c");
    cc::Build::new()
        .file("src/wrap_static_fns.c")
        .include("src")
        .includes([
            format!("{dlss_sdk}/include"),
            format!("{vulkan_sdk}/include"),
        ])
        .compile("wrap_static_fns");
}
