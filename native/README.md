# Pinned FidelityFX native artifacts (optional)

These modules are **runtime-only inputs**. `solarik-fsr-wgpu` loads the AMD
FidelityFX provider with `LoadLibrary`/`dlopen` at runtime; nothing is linked at
build time. A plain `cargo check`/`cargo test`/`cargo build` therefore needs no
DLL, no SDK, no environment variable and no account. A shipping application can
pin and deploy the exact provider beside its executable by populating this
directory.

`artifacts.json` pins the SDK **v2.4.0** signed Windows binaries with byte size
and SHA-256. `native/` holds **only this manifest and the MIT license text — no
binaries**. GitHub forbids uploading new Git LFS objects to public forks, and an
upstreamable OSS PR must be pure source, so the signed DLLs are pinned in the
**consuming application's private LFS tree** (BARD `native/<target>/ffx/`).
`build_support/native.rs` reads `FSR_NATIVE_LIBS` / `FSR_RUNTIME_DIR` (set by the
consuming app's `.cargo/config.toml`) and deploys them beside the built binary.
A local copy under `native/<target>/ffx/` is ignored by git and used only for
offline provider probes. AMD FidelityFX is MIT; retaining its license in
distributions is still required.

## Which provider

| Effect family | Windows (`x86_64-pc-windows-msvc/ffx/`) | Linux |
| --- | --- | --- |
| Vulkan loader + upscaler | `amd_fidelityfx_loader_vk.dll`, `amd_fidelityfx_upscaler_vk.dll` | `_vk.so` pair built from MIT source |
| Vulkan frame generation | `amd_fidelityfx_framegeneration_vk.dll` | built from MIT source |
| DX12 loader + upscaler | `amd_fidelityfx_loader_dx12.dll`, `amd_fidelityfx_upscaler_dx12.dll` | n/a |
| DX12 denoiser (FSR Ray Regeneration) | `amd_fidelityfx_denoiser_dx12.dll` | n/a |
| DX12 radiance cache | `amd_fidelityfx_radiancecache_dx12.dll` | n/a |
| DX12 frame generation | `amd_fidelityfx_framegeneration_dx12.dll` | n/a |

AMD publishes **no prebuilt Linux `.so`** and **no Vulkan build of Ray
Regeneration / radiance cache**. Linux provider libraries are built from the
pinned MIT source with `-DFFX_API_BACKEND=VK_X64` and recorded in
`artifacts.json` once produced (plan decision D6). The loader resolves effect
DLLs beside itself and the exe folder. `FSR_DLL` (exact path),
`FSR_RUNTIME_DIR`, `FSR_NATIVE_LIBS/<TARGET>/ffx` and the platform loader path
are searched. `FSR_BACKEND_VK_TYPE` pins the Vulkan backend descriptor
discriminator (SDK 2.4.0 uses `0x01000003`; 1.1.x used `0x00000003`).

## Provenance and access

- Source and signed Windows binaries: <https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK>
- Pinned tag: **v2.4.0**, directory `Kits/FidelityFX/signedbin`.
- SHA-256 for every module is in `artifacts.json` and verified by
  `build_support/native.rs` before deployment. Unresolved Git LFS pointers are
  rejected; a missing module is a warning (the provider is optional at runtime).
- License: `licenses/AMD-FidelityFX-SDK-LICENSE.txt` (MIT). Retain it in
  distributions. The fork's Rust code is MIT/Apache-2.0 and does not relicense
  the SDK.
- NVIDIA DLSS/NGX binaries are deliberately **not** here: their license
  forbids distribution in an open-source repository. They stay in the consuming
  application's private `native/`.
