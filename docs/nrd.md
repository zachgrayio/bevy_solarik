# NRD RELAX on Vulkan

The optional `nrd` feature denoises opaque diffuse and specular lighting with NVIDIA
NRD 4.18.0 RELAX. It runs the SDK's source-built SPIR-V through wgpu 30 on Windows
and Linux; it does not require an NVIDIA GPU, DLSS, NRI, or a runtime NRD DLL.
An application may enable `dlss` and `nrd` together. Views with DLSS Ray
Reconstruction use RR; other `SolarikLighting` views default to NRD. Set
`SolarikLighting::nrd = false` for a raw comparison. ReSTIR stays active in both.

## Source build

Use a C++17 compiler, CMake, Ninja, and network access for NRD's pinned ShaderMake,
MathLib and DXC dependencies. On Windows run CMake from an x64 Visual Studio
developer environment with the Windows SDK installed. On Linux install a C++
toolchain, CMake, Ninja and Vulkan development/runtime packages.

```text
git clone --branch v4.18.0 https://github.com/NVIDIA-RTX/NRD
git -C NRD checkout 8f12a22fc90e6a3c7f5da4c9c3103633c842963a
cmake -S NRD -B nrd-build -G Ninja -DCMAKE_BUILD_TYPE=Release -DNRD_STATIC_LIBRARY=ON -DNRD_NRI=OFF -DNRD_EMBEDS_DXIL_SHADERS=OFF -DNRD_EMBEDS_DXBC_SHADERS=OFF -DNRD_EMBEDS_SPIRV_SHADERS=ON -DNRD_SUPPORTS_QUAD_INTRINSICS=OFF -DNRD_NORMAL_ENCODING=2
cmake --build nrd-build --parallel
```

Set absolute `NRD_SDK` to the NRD source checkout. Put both `NRD.lib` and
`ShaderMakeBlob.lib` (Windows), or `libNRD.a` and `libShaderMakeBlob.a` (Linux), in
`NRD/_Bin`, or set `NRD_LIB_DIR` to a directory containing both libraries.
ShaderMakeBlob is produced in `nrd-build/_deps/shadermake-build/`. The Rust build
compiles a small C ABI bridge and statically links both libraries. Header version
and library normal/roughness encodings are checked; mismatched SDKs fail early.
Use normal encoding **2**, linear roughness, and the same flags on both platforms.

Enable `nrd` on the dependency and apply all four root patches from the
[development setup](../README.md#usage). Request
`SolarikPlugins::required_wgpu_features()` when creating the Vulkan device;
NRD adds passthrough shaders, filterable float32 and 16-bit normalized formats.
Bevy's functionality priority enables supported features, but passthrough shaders
must be explicitly requested. Unsupported hardware must retain a raster fallback.

## Pipeline and history

ReSTIR DI/GI and glossy tracing publish separate exposed diffuse/specular radiance
and unnormalized hit distances. Preparation demodulates material color and writes
motion (current minus previous UV), world normals, linear roughness and positive
view-Z. The bridge reverses the motion convention for NRD. Resolve remodulates
filtered lighting and adds emission. Depth-zero and non-deferred pixels are excluded.
Forward opaque/masked actors and transparency render after the opaque resolve.
Primary glass/foliage retain their existing compositors; they are not independently
NRD-denoised. NRD is a denoiser, not an upscaler or an anti-aliasing replacement.

Controllers, texture pools and compute pipelines persist per camera. Camera/view
history resets, resolution or RR changes, and disabling/re-enabling NRD invalidate
history. Camera matrices are non-jittered, column-major. Trusted SPIR-V loading and
the C ABI are isolated in `crates/nrd-wgpu/src/native.rs`; renderer code stays safe
Rust. The adapter rewrites SDK descriptor decorations to dense bindings because
wgpu 30 compacts Vulkan layouts. Both dependency and SDK versions are pinned.

## Validation and distribution

`cargo test -p solarik-nrd-wgpu -- --nocapture` executes the actual source-built
RELAX shaders on Vulkan, reads back output and checks noise reduction, brightness
and history restart. `cargo test -p bevy_solarik --features nrd --test nrd_shaders`
compiles the production WESL producers/guides with and without RR. Windows RTX 4090
and Linux WSL2 Vulkan software-driver execution are validated; Linux hardware ray
tracing and the complete BARD renderer still require native Linux hardware review.
See BARD's matched headless capture record for integration image review.

NRD itself is governed by its **NVIDIA RTX SDKs license**, separately from this
adapter's MIT/Apache license. SDK source, generated shaders and libraries are not
vendored here. Distributions must include the SDK license and required notices,
including “This software contains source code provided by NVIDIA Corporation.”
