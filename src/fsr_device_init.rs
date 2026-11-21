//! FFX Vulkan device-extension registration for the consuming application.
//!
//! The `FidelityFX` Vulkan provider needs certain `VkDevice` extensions
//! enabled at device-creation time; without them `ffxCreateContext` fails
//! inside the provider (measured with the signed FFX DLL, see `docs/fsr.md`).
//! wgpu exposes a create-device callback through `RawVulkanInitSettings`, the
//! same seam the `DLSS` plugin uses. This module contributes an FSR callback
//! to that resource.
//!
//! It is compiled only with feature `fsr_device_init` (which also enables
//! `bevy_render/raw_vulkan_init`). Call [`install_fsr_device_extensions`] from
//! the application before `DefaultPlugins`/`RenderPlugin` build, so both the
//! `DLSS` callbacks and this one are present when the `VkDevice` is created.
//!
//! The callback never adds an unsupported extension: it enumerates the physical
//! device's real capabilities and appends only the FFX-required subset, so
//! device creation stays safe on every GPU (including GPUs with no FSR support).

use bevy_app::App;
use bevy_render::renderer::raw_vulkan_init::RawVulkanInitSettings;

/// Register the FFX Vulkan device-extension callback on the app's
/// [`RawVulkanInitSettings`]. Safe: the callback only ever *adds* extensions
/// the device reports as supported, never removes features or enables
/// unsupported ones. Idempotent — the resource is shared with DLSS's own
/// callbacks via `get_resource_or_init`.
#[expect(
    unsafe_code,
    reason = "registering an add-only, capability-filtered Vulkan device-extension callback"
)]
pub fn install_fsr_device_extensions(app: &mut App) {
    let mut settings = app
        .world_mut()
        .get_resource_or_init::<RawVulkanInitSettings>();

    // SAFETY: the callback only appends `&'static CStr` device extensions that
    // the physical device actually exposes (enumerated via ash inside
    // `register_device_extensions_for_adapter`). It never removes features,
    // never changes queue-create infos or the create-info pNext chain, and
    // never enables anything the device does not support. This upholds every
    // safety requirement of `RawVulkanInitSettings::add_create_device_callback`.
    unsafe {
        settings.add_create_device_callback(move |args, adapter, _additional_features| {
            let added = solarik_fsr_wgpu::register_device_extensions_for_adapter(args, adapter);
            if added > 0 {
                tracing::info!(
                    "FFX: registered {added} supported Vulkan device extension(s) for FSR"
                );
            }
        });
    }
}

/// A Bevy [`Plugin`](bevy_app::Plugin) that installs the FFX device-extension
/// callback, for applications that prefer `add_plugins` over a direct call.
#[derive(Default)]
pub struct FsrDeviceInitPlugin;

impl bevy_app::Plugin for FsrDeviceInitPlugin {
    fn build(&self, app: &mut App) {
        install_fsr_device_extensions(app);
    }
}
