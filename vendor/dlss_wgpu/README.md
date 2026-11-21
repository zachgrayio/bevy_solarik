# dlss_wgpu - Deep Learning Super Sampling for wgpu

A wrapper for using [DLSS](https://www.nvidia.com/en-us/geforce/technologies/dlss) with [wgpu](https://github.com/gfx-rs/wgpu) when targeting Vulkan.

## Version Chart

| dlss_wgpu |   dlss   | wgpu  |
| :-------: | :------: | :---: |
|  v6.0.0   | v310.9.1 |  v30  |
|  v5.0.0   | v310.7.0 |  v30  |
|  v4.0.0   | v310.5.3 |  v29  |
|  v3.0.0   | v310.5.0 |  v28  |
|  v2.0.0   | v310.4.0 |  v27  |
|  v1.0.1   | v310.4.0 |  v26  |
|  v1.0.0   | v310.3.0 |  v26  |

## BARD native artifacts

This vendored patch uses the application's LFS-tracked Windows x64/MSVC and Linux
x64/GNU link/runtime libraries by default. Cargo's workspace-relative `BARD_NATIVE_LIBS`
points to their parent directory. Target-specific declarations are generated from
DLSS SDK 310.9.1; no bindgen, libclang or Vulkan SDK is required for normal builds.
The build script detects unresolved LFS pointers and reports `git lfs pull`.

`DLSS_SDK` remains an explicit override for a matching complete SDK. It compiles the
checked-in C wrappers with `VULKAN_SDK` headers and the ordinary C compiler, retaining
the pinned 310.9.1 Rust declarations. Regenerate both declarations and wrapper archives
when changing SDK versions. The SDK is separately licensed; application artifacts
retain NVIDIA's license. This crate contains no NVIDIA native SDK binaries.

## Regenerating the frame-generation bindings

`src/wrapper.h` includes `nvsdk_ngx_defs_dlssg.h` (the C-safe DLSS-G definitions
header: parameter keys, `NVSDK_NGX_DLSSG_*` enums, `PFN_NVSDK_NGX_DLSSG_*`
callbacks), so a full SDK regeneration naturally includes FG data. The generated
Rust declarations are checked in at `src/bindings/frame_generation.rs` and
`include!`d by `src/frame_generation.rs::dlssg_keys` (feature `frame_generation`).
They were produced with `rust-bindgen 0.73.2` from the public DLSS SDK v310.9.16
headers, not transcribed by hand:

```
bindgen wrapper_dlssg.h \
  --allowlist-item '.*DLSSG.*' \
  --allowlist-item '.*FrameGeneration.*' --allowlist-item '.*FrameInterpolation.*' \
  --allowlist-recursively=false \
  --raw-line 'use crate::nvsdk_ngx::{NVSDK_NGX_Handle, NVSDK_NGX_Parameter, NVSDK_NGX_Result};' \
  -I<ngx-include> -I<vulkan-include> -I<clang-resource> -o src/bindings/frame_generation.rs \
  -- -x c
```

`wrapper_dlssg.h` is `#include <nvsdk_ngx_defs_dlssg.h>`; `LIBCLANG_PATH` must
point at a libclang.dll. Regenerate on SDK upgrades. The **callable** FG helpers
(`NGX_VK_CREATE_DLSSG` / `NGX_VK_EVALUATE_DLSSG` and their `*_Eval/_Create`
params) live in the C++-only `nvsdk_ngx_helpers_dlssg_vk.h` /
`nvsdk_ngx_params_dlssg.h` and are NOT exposed here (default member initializers
cannot be included by the C-compiled `wrap_static_fns.c`); they need `__extern`
C++ shims and NVIDIA Streamline present interception, tracked in
`frame_generation.rs::BINDINGS_REGENERATION_REQUIRED_FOR`.

## Distributing Your App

Once your app is compiled, you do not need to distribute the entire DLSS SDK, or set the `DLSS_SDK` environment variable. You only need to distribute the DLSS DLL(s) and license text as follows:

1. Copy the DLL:
    * Windows: Copy `$DLSS_SDK/lib/Windows_x86_64/rel/nvngx_dlss.dll` to the same directory as your app
    * Linux: Copy `$DLSS_SDK/lib/Linux_x86_64/rel/libnvidia-ngx-dlss.so.310.9.1` to the same directory as your app
2. Include the full copyright and license blurb texts from section `9.5` of `$DLSS_SDK/doc/DLSS_Programming_Guide_Release.pdf` with your app
3. Additionally, for DLSS ray reconstruction:
    * Windows: Copy `$DLSS_SDK/lib/Windows_x86_64/rel/nvngx_dlssd.dll` to the same directory as your app
    * Linux: Copy `$DLSS_SDK/lib/Linux_x86_64/rel/libnvidia-ngx-dlssd.so.310.9.1` to the same directory as your app

## Debug Overlay

When `dlss_wgpu` is compiled with the `debug_overlay` cargo feature, and the `DLSS_SDK` environment variable is set, the development version of the DLSS DLLs will be linked.

The development version of the DLSS SDK comes with an in-app overlay to help debug usage of DLSS. See section `8.2` of `$DLSS_SDK/doc/DLSS_Programming_Guide_Release.pdf` for details.

## Validation Errors

Due to a bug in DLSS, you should [expect to see Vulkan validation errors](https://forums.developer.nvidia.com/t/validation-errors-using-dlss-vulkan-sdk-due-to-vkcmdclearcolorimage/326493).

These errors are safe to ignore.

## Contributing

You can run cargo commands without polluting your local environment by running:

```sh
docker run -v `pwd`:/src $(docker build -q .) cargo clippy
```

You must comply with the DLSS license to run this command.
