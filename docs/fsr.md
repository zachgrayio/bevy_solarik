# AMD FidelityFX Super Resolution (FSR) on Vulkan

The optional `fsr` feature attaches AMD's DLSS-equivalent temporal upscaler to
the Solarik realtime compositor. It is the AMD counterpart of the fork's DLSS
Ray Reconstruction / Super Resolution path and follows the same integration
shape as the [`nrd` feature](nrd.md): an optional feature, a dedicated `-wgpu`
bindings crate, a per-view native controller and a feature-gated node hook.

**FSR is dynamically linked.** `crates/fsr-wgpu` (`solarik-fsr-wgpu`) binds the
FidelityFX SDK's five-function *FfxApi* C ABI and resolves it at runtime with
`libloading`. No AMD binary is a link input, so `cargo check`, `cargo test` and
`cargo build` need no SDK, no DLL, no environment variable and no account.
When no provider is present the renderer reports FSR as unavailable and keeps
native resolution — the same graceful fallback as an unsupported DLSS mode in a
consuming application. AMD's own `ffx_api_loader.h` recommends exactly this
`LoadLibrary` + `GetProcAddress` approach.

## Files

- `crates/fsr-wgpu/` — `solarik-fsr-wgpu`: `#[repr(C)]` FfxApi bindings,
  runtime loader, wgpu→Vulkan handle conversion, DLSS-style quality API.
- `crates/fsr-wgpu/src/abi.h` — the pinned C declarations (provenance only).
- `src/realtime/fsr.rs` / `fsr.wesl` — per-view `ViewFsr`, compose pipeline and
  the `upscale()` hook called after NRD in `src/realtime/node.rs`.
- `src/realtime/mod.rs` — `FsrQualityMode` component enum and
  `SolarikLighting::{fsr, fsr_quality, fsr_sharpness}`.
- `build_support/native.rs`, `native/`, `.cargo/config.toml`, `.gitattributes` —
  optional native-artifact deployment.
- `tests/fsr.rs`, `crates/fsr-wgpu/tests/abi.rs` — CPU-only checks, including
  FFI round-trip tests against an in-process stub provider.
- `crates/fsr-wgpu/examples/fsr_probe.rs` — runnable provider-availability probe;
  opens no window or GPU device.

## Provider and access (no sign-in)

Source and signed Windows binaries are public and MIT-licensed at
<https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK>, pinned at tag
**v2.4.0** in [native/artifacts.json](../native/artifacts.json). There is **no
developer account, EULA gate or login** (unlike NVIDIA DLSS). The 2.4.0 loader
(`amd_fidelityfx_loader_vk.dll` / `_dx12.dll`) resolves per-effect DLLs beside
itself: upscaler, frame generation (Vulkan), denoiser / radiance cache /
frame generation (DX12). AMD publishes **no prebuilt Linux `.so`** and **no
Vulkan build of Ray Regeneration or radiance cache**; Linux provider libraries
are built from the MIT source with `-DFFX_API_BACKEND=VK_X64` (decision D6).
[native/README.md](../native/README.md) owns the artifact record.

Search order, first match wins: `FSR_DLL` (exact path), `FSR_RUNTIME_DIR`,
`FSR_NATIVE_LIBS/<TARGET>/ffx` and finally the platform loader path
(executable directory / system path). `FSR_BACKEND_VK_TYPE` pins the version-
specific backend descriptor discriminator; SDK 2.4.0 uses `0x01000003` and
1.1.x used `0x00000003` (both retried at context creation). `0x02000003` is the
Xbox backend id, not a Vulkan type.

## SDK 2.4.0 effect bindings

`crates/fsr-wgpu/src/effects.rs` mirrors the additional FfxApi descriptors with
`#[repr(C)]` types, compile-time size assertions and in-process stub-FFI tests:
upscaler version (`0x0001000b`), DX12 backend (`0x00000002`), denoiser /
FSR Ray Regeneration (`0x00050000`), radiance cache (`0x00060000`) and frame
generation (`0x00020000`), plus provider-version enumeration/override. Layouts
were cross-checked against the MSVC-compiled C declarations recorded in
`src/abi.h`. These are the ABI layer for Phases 2/5/6; GPU dispatch wiring is
hardware-gated (D12) and reviewed on RX 9000+ / RTX 50-series.

## Engine attach

Enable `fsr` on the `bevy_solarik` dependency. `SolarikLightingPlugin` then
registers `fsr::init_fsr` at `RenderStartup`, which loads the optional provider
into the `FsrState` resource and queues the compose pipeline. Each view gets a
`ViewFsr` (upscaled output texture + persistent native context) through
`prepare.rs`, and `node.rs` calls `fsr::upscale()` after the NRD/resolve stage.

The camera-facing API mirrors `bevy_anti_alias::dlss::Dlss`:

```rust
use bevy_solarik::realtime::{FsrQualityMode, SolarikLighting};

// Opt in per camera; defaults to NativeAA (1:1 temporal AA).
lighting.fsr = true;
lighting.fsr_quality = FsrQualityMode::Performance;
lighting.fsr_sharpness = 0.0;

// Drive the same camera components DLSS uses:
let render = lighting.fsr_quality.render_resolution([display_w, display_h]);
let jitter = lighting.fsr_quality.suggested_jitter(frame_index, display_w);
let mip_bias = lighting.fsr_quality.suggested_mip_bias(display_w);
// MainPassResolutionOverride = render; TemporalJitter { offset: jitter.into() };
// MipBias(mip_bias); reset FSR history after a camera cut or resolution change.
```

The default hook runs FSR **NativeAA** at the view's current resolution (FSR
reads the resolved HDR target, writes `ViewFsr::output`, and the compose pass
copies it back). Scaled presets require the application to lower the camera
render resolution and size the output texture to display resolution, exactly as
it does for DLSS Super Resolution; `FsrQualityMode` supplies the numbers. The
`fsr.wesl` compose pass already scales between the two sizes.

`solarik_fsr_wgpu::Context` also exposes `ffxQuery` helpers for ratio, render
resolution and jitter. These are issued on a live context: SDK 1.1.x providers
dereference the context handle for upscaler query descriptors, so null-context
(global) queries are deliberately not offered. `QualityMode` provides the same
numbers from the documented ratios without needing a GPU device.

## Native artifact pattern

A common convention for pinned native modules is `native/<target>/<vendor>/`,
tracked with Git LFS, with a workspace-relative root in `.cargo/config.toml`,
LFS-pointer validation and byte-identical deployment beside the built
executable. This repository mirrors that convention for FSR through
`build_support/native.rs`, with one deliberate difference: because the provider
is dynamically loaded, a missing or unresolved artifact is a **warning**, not a
build failure. `native/artifacts.json` is a manifest template — no AMD binary is
committed.

## Application integration

A consuming application that wants FSR at runtime needs only to:

1. Enable the `fsr` feature on its `bevy_solarik` dependency.
2. Provide the provider: set `FSR_DLL` / `FSR_RUNTIME_DIR`, or populate
   `FSR_NATIVE_LIBS/<TARGET>/ffx` (in the consuming app's private Git LFS tree)
   and let the build script deploy it. Record sizes and SHA-256 in
   `native/artifacts.json` and retain the SDK license. The public fork branch
   stays pure source — GitHub forbids new LFS objects on public forks.
3. Set `SolarikLighting::fsr` (plus `fsr_quality` / `fsr_sharpness`) on its
   camera, apply `TemporalJitter`, `MipBias` and the render-resolution override
   from `FsrQualityMode`, and fall back when `FsrState::library` is `None`.

No application-specific code lives in this feature; it is entirely generic and
can be upstreamed on its own.

## Validation and limits

- `cargo check -p bevy_solarik --features fsr` and
  `cargo test -p solarik-fsr-wgpu` are CPU-only and open no renderer.
- `crates/fsr-wgpu/src/native.rs` asserts the pinned 64-bit struct sizes;
  regenerate `src/abi.h` and the bindings together when SDK headers change.
- `cargo run -p solarik-fsr-wgpu --example fsr_probe` verifies provider loading
  at runtime. With the public SDK 1.1.4 Vulkan DLL (`amd_fidelityfx_vk.dll`,
  9,332,432 bytes) it reports `FSR provider loaded`; without any DLL it reports
  unavailability and exits successfully. The DLL is public, MIT and needs no
  sign-in.
- The whole FfxApi call chain (create/dispatch/query/destroy, descriptor
  chaining and `pNext` retargeting) is exercised in-process against a stub
  provider with the exact C ABI signatures — no DLL or GPU required.

### Vulkan device requirements (remaining GPU work)

`ffxCreateContext` needs the application's `VkDevice` to be created with the
FFX Vulkan backend's required device extensions/features, exactly as
`dlss_wgpu::register_device_extensions` does for NGX. This is an application
device-creation concern, not something a renderer library can register after
the fact. Measured with the real signed 1.1.4 DLL on an RTX 4090 (wgpu 30
Vulkan device, all non-experimental features enabled):

- `Library::load()` resolves all five exports successfully.
- The v1.1.4 backend descriptor (`0x00000003`) is recognized, but
  `ffxCreateContext` access-violates while initializing the Vulkan backend on
  a device that has not enabled the FFX extensions.
- The other two discriminator values (`0x01000003`, `0x02000003`) return clean
  `FFX_API_RETURN_ERROR` (1), confirming the descriptor layout and chain are
  correct and that the crash is device setup, not a bindings bug.

So before a visual playtest the consuming application must register the FFX
Vulkan device extensions in its device-creation callback and confirm
`ffxCreateContext`/`ffxDispatch` on real `VkImage`s. NativeAA compositing and
scaled-resolution render-target sizing then need a hardware image-review pass
on Windows and Linux, as NRD's validation did. Because FSR is opt-in
(`SolarikLighting::fsr = false` by default), an application that has not done
this setup is unaffected.

- FSR is an upscaler, not a ray-reconstruction denoiser: NRD/DLSS RR remain the
  lighting denoisers, and ReSTIR stays active. FSR runs after their resolve.

## Licensing

The Rust bindings are MIT OR Apache-2.0, as is the rest of this fork. The AMD
FidelityFX SDK is MIT; distributions retain
`native/licenses/AMD-FidelityFX-SDK-LICENSE.txt` and the SDK's required notices.
This license does not relicense NVIDIA's DLSS/NRD SDK portions, which the
consuming application continues to own.
