//! Deploy the optional AMD `FidelityFX` FSR runtime modules next to the built
//! binaries, following the native-artifact convention used by consuming
//! applications (pinned per-target modules, Git LFS and idempotent deployment).
//!
//! FSR differs from DLSS/NRD in one important way: `solarik-fsr-wgpu` loads the
//! provider with dlopen/LoadLibrary at runtime, so these DLLs are **not** build
//! or link inputs. A missing or unresolved artifact is a warning, not a build
//! failure — the renderer then reports FSR as unavailable and keeps native
//! resolution. Hydrated artifacts are still deployed with the same byte-exact,
//! idempotent copy and Git LFS pointer validation as the NVIDIA modules.

use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
};

/// Pinned `FidelityFX` SDK 2.4.0 provider modules per target. The SDK ships a
/// small loader plus per-effect DLLs; the loader resolves effect DLLs next to
/// itself (and beside the exe), so all eight Windows modules are deployed.
/// Vulkan and DX12 sets are both listed because the backend is chosen at
/// runtime. Linux provider libraries are built from the pinned MIT source (D6).
fn runtime_modules(target: &str) -> Option<&'static [&'static str]> {
    match target {
        "x86_64-pc-windows-msvc" => Some(&[
            "amd_fidelityfx_loader_vk.dll",
            "amd_fidelityfx_upscaler_vk.dll",
            "amd_fidelityfx_framegeneration_vk.dll",
            "amd_fidelityfx_loader_dx12.dll",
            "amd_fidelityfx_upscaler_dx12.dll",
            "amd_fidelityfx_denoiser_dx12.dll",
            "amd_fidelityfx_radiancecache_dx12.dll",
            "amd_fidelityfx_framegeneration_dx12.dll",
        ]),
        "x86_64-unknown-linux-gnu" => Some(&[
            "libamd_fidelityfx_loader_vk.so",
            "libamd_fidelityfx_upscaler_vk.so",
            "libamd_fidelityfx_framegeneration_vk.so",
        ]),
        // Metal targets compile the fork without Vulkan-only features and have
        // no pinned modules; a system-provided provider is still loadable.
        _ => None,
    }
}

/// Validate a native artifact is real bytes, not an unresolved Git LFS pointer.
fn artifact(path: &Path) -> bool {
    println!("cargo:rerun-if-changed={}", path.display());
    let mut header = [0; 8];
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    if &header == b"version " {
        println!(
            "cargo:warning={} is an unresolved Git LFS pointer; run git lfs pull to enable FSR",
            path.display()
        );
        return false;
    }
    true
}

/// Copy only when the bytes differ, so a running game can keep its DLL open.
fn deploy(source: &Path, destination: &Path) -> bool {
    if !artifact(source) {
        return false;
    }
    if destination.is_file()
        && let (Ok(mut a), Ok(mut b)) = (fs::File::open(source), fs::File::open(destination))
    {
        let mut left = blake3::Hasher::new();
        let mut right = blake3::Hasher::new();
        if left.update_reader(&mut a).is_ok()
            && right.update_reader(&mut b).is_ok()
            && left.finalize() == right.finalize()
        {
            return true;
        }
    }
    match fs::copy(source, destination) {
        Ok(_) => true,
        Err(error) => {
            println!(
                "cargo:warning=Could not deploy {}: {error}",
                source.display()
            );
            false
        }
    }
}

/// Resolve the application's pinned native artifact directory and license dir.
fn native_paths() -> Option<(PathBuf, PathBuf)> {
    if let Some(dir) = env::var_os("FSR_RUNTIME_DIR") {
        let runtime = PathBuf::from(dir);
        if runtime.is_absolute() {
            let licenses = runtime.parent().unwrap_or(&runtime).join("licenses");
            return Some((runtime, licenses));
        }
        println!(
            "cargo:warning=FSR_RUNTIME_DIR must be absolute; ignoring FSR artifact deployment"
        );
    }
    let target = env::var("TARGET").unwrap_or_default();
    if let Some(root) = env::var_os("FSR_NATIVE_LIBS") {
        let root = PathBuf::from(root);
        if root.is_absolute() {
            return Some((root.join(&target).join("ffx"), root.join("licenses")));
        }
        println!(
            "cargo:warning=FSR_NATIVE_LIBS must be absolute; ignoring FSR artifact deployment"
        );
    }
    None
}

pub(crate) fn configure() {
    for key in ["FSR_NATIVE_LIBS", "FSR_RUNTIME_DIR", "FSR_DLL"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    if env::var_os("CARGO_FEATURE_FSR").is_none() {
        return;
    }
    let target = env::var("TARGET").unwrap_or_default();
    let Some(names) = runtime_modules(&target) else {
        return;
    };
    let Some((root, licenses)) = native_paths() else {
        println!(
            "cargo:warning=FSR runtime modules are optional (dynamic loading); set FSR_NATIVE_LIBS or FSR_RUNTIME_DIR to deploy them"
        );
        return;
    };
    let Ok(out) = env::var("OUT_DIR") else {
        return;
    };
    let Some(destination) = Path::new(&out).ancestors().nth(3) else {
        return;
    };
    if let Err(error) = fs::create_dir_all(destination) {
        println!("cargo:warning=FSR deployment directory: {error}");
        return;
    }
    let mut deployed = 0;
    for name in names {
        if deploy(&root.join(name), &destination.join(name)) {
            deployed += 1;
        }
    }
    if licenses.is_dir() {
        println!("cargo:rerun-if-changed={}", licenses.display());
        if let Ok(entries) = fs::read_dir(&licenses) {
            for entry in entries.flatten() {
                let source = entry.path();
                if source.is_file() {
                    let _ = deploy(&source, &destination.join(entry.file_name()));
                }
            }
        }
    }
    if deployed == 0 {
        println!(
            "cargo:warning=No pinned FSR runtime module found in {}; the provider will be loaded from FSR_DLL/FSR_RUNTIME_DIR or the system path at runtime",
            root.display()
        );
    }
}
