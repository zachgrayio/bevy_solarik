# Linux x64 FSR provider

AMD publishes **no prebuilt Linux `.so`**. Build `ffx-api` from the MIT source
with `-DFFX_API_BACKEND=VK_X64` (v1.1.4 produces `libamd_fidelityfx_vk.so`), or
build the v2.4.0+ loader/upscaler pair, and place the result here:

- `libamd_fidelityfx_vk.so` (v1.1.4)
- `libamd_fidelityfx_loader_vk.so` + `libamd_fidelityfx_upscaler_vk.so` (v2.4.0+)

No binaries are committed: `solarik-fsr-wgpu` dynamically loads the provider,
so builds, tests and the headless workspace suite need none. An empty directory
only produces a warning and leaves FSR as an unavailable capability.

Source: <https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK> (MIT, no
sign-in). See `../../README.md` and `../../artifacts.json`.
