# Shared D3D12 graphics-session lifetime

## Purpose

Keep native NGX/Streamline state and its code alive until all feature clients and graphics
objects have finished. Main-world resource drop order is not a renderer shutdown barrier:
`RenderAppChannels` can still own the render world when main-world resources are cleared.

## Files

- [Session owner](../src/graphics_session.rs): outer runner, shared leases and closing barrier.
- [NGX bootstrap](../src/dx12_ngx_init.rs): scoped source-owned SDK/device access and shutdown.
- [Feature SDK](../vendor/dlss_wgpu/src/sdk.rs): opaque runtime retention through parameter Drop.
- [Bevy hook](../vendor/bevy_anti_alias/src/dlss/mod.rs): `retain_dlss_runtime` registration.
- [Streamline runtime](../crates/streamline-wgpu/src/runtime.rs): native shutdown and retained DLLs.

## Application owner

Use `bevy_solarik::run_app(|| build_app()) -> AppExit` for shared D3D12 NGX applications.
Pass a builder, not an already-running application: the owner exists before initialization,
remains outside the complete App lifetime, and preserves its actual engine exit status.
Do not replace this with a main-world guard, `process::exit`, leaked graphics resources or a
forced successful exit. Device/queue work must not escape the App shutdown barrier unleased.

The NGX install seam still belongs before renderer construction. Successful setup retains
strong queue/device ownership; bootstrap COM ownership also protects initialization/error paths
before a wgpu device is available. `ngx_d3d12_sdk()` now returns scoped `D3D12SdkAccess`, not a
static borrow that can outlive native shutdown.

SR/RR features retain their SDK through `ReleaseFeature`; the SDK retains runtime leases through
its parameter destructor. Registration supports either plugin-finish order. Neither the runtime
nor its weak registry owns a strong SDK reference, so the graph has no SDK/session reference cycle.

## Final teardown

After the complete App and all retained SDK clients are gone:

1. Wait for submitted GPU work; all feature releases and SDK parameter destruction precede this.
2. Destroy bootstrap capability parameters while native providers remain initialized.
3. Shut Streamline down, retaining its runtime/interposer and already-loaded plugin modules.
4. Shut the owned NGX SDK down and release its bootstrap COM reference.
5. Drop the retained queue/device while native code remains loaded.
6. Release the provider/module code references, without permanently pinning DLLs.

Native DX12 feature release is serialized with SDK clients, but GPU polling is outside that lock
to avoid reentrant deferred-drop deadlocks. NGX/Streamline API synchronization requirements still
apply to direct callers ([NGX API contract](https://raw.githubusercontent.com/NVIDIA/DLSS/main/include/nvsdk_ngx.h),
[Streamline core API](https://raw.githubusercontent.com/NVIDIAGameWorks/Streamline/v2.14.16/include/sl_core_api.h)).
The final native cleanup runs outside registry and SDK mutexes.

The registry serializes acquisition with final release: Closing is established before the last
inner strong reference disappears. Concurrent acquisition waits until destruction completes;
weak-upgrade failure is never permission to initialize a second overlapping native session.

## Validation and scope

Ordinary CPU tests cover main/render/feature owner permutations, provider presence, barrier-pinned
final destruction, concurrent acquisition and exact COM retain/release. Run
`cargo test --lib graphics_session::tests` and the `dlss_wgpu` owned-device-reference test with its DX12 feature;
[native artifacts](../vendor/dlss_wgpu/README.md#bard-native-artifacts) owns link requirements.
Actual default graphics capture must also finish with a successful process exit: written PNGs
alone do not validate teardown. Legacy Vulkan fallback behavior is unchanged; this lifetime
integration is not a claim that every backend, driver or renderer configuration is certified.
