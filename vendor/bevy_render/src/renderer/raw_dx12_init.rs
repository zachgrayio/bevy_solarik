//! # D3D12 NGX device-initialization hook
//!
//! This module is the D3D12 counterpart to
//! `crate::renderer::raw_vulkan_init::RawVulkanInitSettings`. An application
//! inserts [`RawDx12InitSettings`] as a [`Resource`] *before* it adds
//! `DefaultPlugins` / creates the render device, so that NVIDIA NGX
//! (DLSS / DLSS Ray Reconstruction) can be set up for wgpu's D3D12 backend.
//!
//! ## Why a raw device pointer?
//!
//! NGX's D3D12 entry points — `NVSDK_NGX_D3D12_Init` and
//! `NVSDK_NGX_D3D12_Init_with_ProjectID` — must be handed the process's raw
//! `ID3D12Device*` (a COM interface pointer). wgpu keeps that pointer private
//! inside its D3D12 backend, so it has to be captured around device creation
//! and forwarded to NGX. That pointer is exactly what a create-device callback
//! receives through [`CreateDx12DeviceCallbackArgs::device`].
//!
//! ## Why this differs from the Vulkan hook
//!
//! On Vulkan, NGX needs a set of instance/device extensions enabled *while the
//! instance and device are being created*, which is why the Vulkan hook plugs
//! into wgpu-hal's `init_with_callback` / `open_with_callback`.
//!
//! wgpu-hal's D3D12 backend has **no** `init_with_callback` /
//! `open_with_callback` extension hooks: D3D12 has no Vulkan-style extensions
//! for NGX to inject. NGX D3D12 only needs the raw `ID3D12Device*` *after* the
//! device already exists. Consequently this resource does not patch device
//! creation; it just collects callbacks and exposes
//! [`RawDx12InitSettings::run_create_device_callbacks`], which the integration
//! point calls once with the raw device pointer (for example obtained from
//! wgpu's D3D12 HAL `Device::raw_device()`).
//!
//! This module is only compiled on Windows (gated by `#[cfg(windows)]` in
//! `renderer::mod`), so Linux and Apple builds are unaffected.

use alloc::sync::Arc;
use bevy_ecs::resource::Resource;
use core::ffi::c_void;

/// Opaque raw `ID3D12Device*`. NGX takes the COM pointer as `void*`; keeping it
/// as `*mut c_void` avoids pulling a windows-sys crate into this module.
pub type D3D12DevicePtr = *mut c_void;

/// Arguments handed to a D3D12 "create instance" callback.
///
/// Empty on purpose: NGX D3D12 has no instance-level requirements. It exists
/// for API symmetry with the Vulkan hook.
pub struct CreateDx12InstanceCallbackArgs;

/// Arguments handed to a D3D12 "create device" callback.
pub struct CreateDx12DeviceCallbackArgs {
    /// The raw `ID3D12Device*` needed by `NVSDK_NGX_D3D12_Init`.
    pub device: D3D12DevicePtr,
}

/// Settings the application inserts as a [`Resource`] *before* it adds
/// `DefaultPlugins` / creates the render device, in order to register NGX's
/// required D3D12 device setup.
///
/// Mirrors `crate::renderer::raw_vulkan_init::RawVulkanInitSettings`.
#[derive(Resource, Default, Clone)]
pub struct RawDx12InitSettings {
    // SAFETY: kept private so callbacks can only be registered through the
    // `unsafe` methods below.
    create_instance_callbacks: Vec<Arc<dyn Fn(&mut CreateDx12InstanceCallbackArgs) + Send + Sync>>,
    // SAFETY: kept private so callbacks can only be registered through the
    // `unsafe` methods below.
    create_device_callbacks: Vec<Arc<dyn Fn(&mut CreateDx12DeviceCallbackArgs) + Send + Sync>>,
}

impl RawDx12InitSettings {
    /// Register a callback invoked at D3D12 instance creation.
    ///
    /// # Safety
    /// - The callback must not assume instance features NGX does not provide.
    /// - NGX D3D12 has no instance-level setup, so most applications register
    ///   none; the method exists for symmetry with the Vulkan hook.
    #[allow(unsafe_code)]
    pub unsafe fn add_create_instance_callback(
        &mut self,
        callback: impl Fn(&mut CreateDx12InstanceCallbackArgs) + Send + Sync + 'static,
    ) {
        self.create_instance_callbacks.push(Arc::new(callback));
    }

    /// Register a callback invoked once the D3D12 device (and its raw
    /// `ID3D12Device*`) exists. This is where NGX D3D12 should be initialized.
    ///
    /// # Safety
    /// - The callback receives a raw COM pointer; it must keep it valid and
    ///   must not use it after the owning device has been destroyed.
    /// - The callback must not remove or fabricate device capabilities.
    #[allow(unsafe_code)]
    pub unsafe fn add_create_device_callback(
        &mut self,
        callback: impl Fn(&mut CreateDx12DeviceCallbackArgs) + Send + Sync + 'static,
    ) {
        self.create_device_callbacks.push(Arc::new(callback));
    }

    /// Run every registered create-instance callback.
    pub fn run_create_instance_callbacks(&self) {
        let mut args = CreateDx12InstanceCallbackArgs;
        for callback in &self.create_instance_callbacks {
            callback(&mut args);
        }
    }

    /// Run every registered create-device callback with the raw
    /// `ID3D12Device*`. Call this once the wgpu D3D12 device exists.
    pub fn run_create_device_callbacks(&self, device: D3D12DevicePtr) {
        let mut args = CreateDx12DeviceCallbackArgs { device };
        for callback in &self.create_device_callbacks {
            callback(&mut args);
        }
    }
}
