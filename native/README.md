# Pinned FSR native artifacts (optional)

These modules are **runtime-only inputs**. `solarik-fsr-wgpu` loads the AMD
FidelityFX provider with `LoadLibrary`/`dlopen` at runtime; nothing is linked at
build time. A plain `cargo check`/`cargo test`/`cargo build` therefore needs no
DLL, no SDK, no environment variable and no account. A shipping application can
pin and deploy the exact provider beside its executable by populating this
directory.

`artifacts.json` lists the expected filenames per target. `native/` is read by
`build_support/native.rs` (deployment) and by `crates/fsr-wgpu/build.rs`
(`FSR_NATIVE_DIR`). Cargo's `.cargo/config.toml` sets the workspace-relative
`FSR_NATIVE_LIBS`; a consuming application supplies its own value.

## Which provider

| SDK | Windows | Linux |
| --- | --- | --- |
| FidelityFX-SDK **v1.1.4** (last prebuilt Vulkan release) | `amd_fidelityfx_vk.dll` | `libamd_fidelityfx_vk.so` (**build from MIT source**, no prebuilt `.so` is published) |
| FidelityFX-SDK **v2.4.0+** | `amd_fidelityfx_loader_vk.dll` + `amd_fidelityfx_upscaler_vk.dll` | build the matching `_vk.so` pair from MIT source |

SDK 2.0–2.3 is DX12-only and has no Vulkan provider. The loader tries every
candidate name in order; `FSR_DLL` (exact path), `FSR_RUNTIME_DIR`,
`FSR_NATIVE_LIBS/<TARGET>/fsr` and the platform loader path are searched.
`FSR_BACKEND_VK_TYPE` pins the backend descriptor discriminator when a specific
SDK generation needs it (otherwise all three known values are retried).

## Provenance and access

- Source and signed Windows binaries: <https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK>
  (MIT; `PrebuiltSignedDLL/` at tag `v1.1.4`, `Kits/FidelityFX/signedbin/` at
  `v2.4.0`). GitHub release assets are public.
- **No sign-in, no developer account, no EULA gate.** This is unlike NVIDIA DLSS.
- The bindings are pinned to `ffx_api.h`, `ffx_api_types.h`, `ffx_upscale.h` and
  `vk/ffx_api_vk.h`; see `crates/fsr-wgpu/src/abi.h` and the layout assertions in
  `crates/fsr-wgpu/src/native.rs`. Regenerate bindings and DLLs together when the
  SDK changes.
- The AMD FidelityFX SDK is MIT-licensed; retain
  `licenses/AMD-FidelityFX-SDK-LICENSE.txt` in distributions.

## Rebuilding / pinning

1. Download `FidelityFX-SDK-v1.1.4.zip` (or the v2.4.0 archive) from the public
   GitHub release, or build `ffx-api` with `-DFFX_API_BACKEND=VK_X64` for Linux.
2. Copy the Vulkan provider into `native/<target>/fsr/` using the exact names in
   `artifacts.json`.
3. Record byte size and SHA-256 in `artifacts.json`; keep the LICENSE alongside.
4. `git lfs pull` hydrates the pinned modules. Unresolved LFS pointers are
   detected and reported by `build_support/native.rs` (a warning, not a failure,
   because the provider is optional).

DLSS and NRD are **not** redistributed here; the consuming application owns
those artifacts. FSR's MIT license does not relicense the NVIDIA SDK portions.
