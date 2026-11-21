# Windows x64 FSR provider

Place the pinned **Vulkan** provider here (Git LFS, see `../../README.md` and
`../../artifacts.json`):

- FidelityFX-SDK v1.1.4: `amd_fidelityfx_vk.dll`
- FidelityFX-SDK v2.4.0+: `amd_fidelityfx_loader_vk.dll` and
  `amd_fidelityfx_upscaler_vk.dll`

No binaries are committed: `solarik-fsr-wgpu` dynamically loads the provider,
so builds, tests and the headless workspace suite need none. `build_support/native.rs`
deploys any pinned modules beside the built binaries; an empty directory only
produces a warning and leaves FSR as an unavailable capability.

The AMD FidelityFX SDK is MIT-licensed and publicly available at
<https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK> with **no sign-in**.
