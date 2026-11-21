//! wgpu 30 → raw Vulkan handle conversion, the second (and last) unsafe boundary.
//!
//! The FfxApi Vulkan provider takes `VkImage`/`VkCommandBuffer` handles as
//! `void*`, not wgpu objects. These helpers follow the exact pattern already used
//! by the fork's `vendor/dlss_wgpu` (`as_hal::<Vulkan>()` + `raw_handle()`).

use crate::{native, quality};
use ash::vk::Handle as _;
use std::{ffi::c_void, sync::OnceLock};
use wgpu::hal::api::Vulkan;

static VULKAN_LOADER: OnceLock<Option<libloading::Library>> = OnceLock::new();

/// `PFN_vkGetDeviceProcAddr`, loaded from the Vulkan loader itself. The FSR
/// provider uses it to resolve device-level entry points. We keep the loader
/// module alive for the process, exactly like a dynamic-linked Vulkan wrapper.
fn vk_get_device_proc_addr() -> *mut c_void {
    let name = if cfg!(windows) {
        "vulkan-1.dll"
    } else {
        "libvulkan.so.1"
    };
    let loader = VULKAN_LOADER.get_or_init(|| unsafe { libloading::Library::new(name).ok() });
    let Some(loader) = loader else {
        return std::ptr::null_mut();
    };
    type GetDeviceProcAddr =
        unsafe extern "C" fn(*mut c_void, *const u8) -> Option<unsafe extern "C" fn()>;
    let Ok(symbol) = (unsafe { loader.get::<GetDeviceProcAddr>(b"vkGetDeviceProcAddr\0") }) else {
        return std::ptr::null_mut();
    };
    let function: GetDeviceProcAddr = *symbol;
    function as usize as *mut c_void
}

/// The `VkDeviceContext` chained onto an FSR create descriptor.
pub(crate) fn device_context(device: &wgpu::Device) -> Option<native::DeviceContextVk> {
    // SAFETY: wgpu exposes raw Vulkan handles only for backend interop. The
    // handles are immutable for the device's lifetime and are used solely to
    // initialize the FSR provider; no owned wgpu resource is retained here.
    let hal = unsafe { device.as_hal::<Vulkan>() }?;
    Some(native::DeviceContextVk {
        device: hal.raw_device().handle().as_raw() as usize as *mut c_void,
        physical_device: hal.raw_physical_device().as_raw() as usize as *mut c_void,
        get_device_proc_addr: vk_get_device_proc_addr(),
    })
}

/// A `FfxApiResource` wrapping the `VkImage` behind a wgpu texture view.
///
/// FSR creates its own `VkImageView`s internally; the `resource` field is the
/// image handle itself (see `ffxApiGetResourceVK` in `ffx_api_vk.h`).
pub(crate) fn resource(view: &wgpu::TextureView, state: u32) -> Option<native::Resource> {
    let texture = view.texture();
    // SAFETY: `as_hal`/`raw_handle` borrow the live wgpu texture; the returned
    // VkImage is passed straight into a descriptor owned by this dispatch.
    let image = unsafe { texture.as_hal::<Vulkan>().map(|hal| hal.raw_handle()) }?;
    Some(native::Resource {
        resource: image.as_raw() as usize as *mut c_void,
        description: native::ResourceDescription {
            resource_type: quality::resource_types::TEXTURE_2D,
            format: quality::surface_format(texture.format()),
            width: texture.width(),
            height: texture.height(),
            depth: 1,
            mip_count: texture.mip_level_count(),
            flags: 0,
            usage: quality::usage::READ_ONLY,
        },
        state,
    })
}

/// Run `f` with the raw `VkCommandBuffer` of a wgpu encoder while it is open for
/// HAL interop. FSR records its passes into that command buffer, so the call must
/// happen inside the `as_hal_mut` scope (matching `dlss_wgpu`).
pub(crate) fn with_command_buffer<R>(
    encoder: &mut wgpu::CommandEncoder,
    f: impl FnOnce(*mut c_void) -> R,
) -> Option<R> {
    // SAFETY: the raw handle is used only for the duration of this call, while
    // wgpu guarantees the encoder is alive and no other encoder is active.
    unsafe {
        encoder.as_hal_mut::<Vulkan, _, _>(|hal| {
            hal.map(|hal| f(hal.raw_handle().as_raw() as usize as *mut c_void))
        })
    }
}

/// Vulkan device requirements for the FFX SDK 2.4.0 Vulkan provider.
///
/// `Kits/FidelityFX/api/include/vk/ffx_api_vk.h` does **not** enumerate the
/// required device extensions/features: it only declares `VkDeviceContext`,
/// `VkQueueInfoFFXAPI` and the surface-format conversions. The lists below are
/// a conservative, documented set drawn from the FFX SDK's Vulkan backend
/// requirements (shader `FP16`/int8, 8/16-bit storage, `FP16` atomics,
/// subgroup ops, bindless descriptor indexing, buffer device address,
/// `VK_KHR_swapchain` for the frame-generation swapchain, and maintenance
/// extensions).
///
/// **These names need runtime confirmation against the exact provider DLL you
/// ship**: SDK generation, FSR version and the enabled effect mix change them.
/// This is a compile-time data/ABI helper only — it opens no `VkDevice`, queries
/// no `VkPhysicalDevice` and links nothing. Enable the corresponding
/// `VkPhysicalDevice*Features2` fields in the application's device-creation
/// callback before turning FSR on.
#[allow(dead_code)]
pub mod device_extensions {
    /// Device extension names FFX is known to require at `VkDevice` creation.
    pub const DEVICE_EXTENSIONS: &[&str] = &[
        "VK_KHR_16bit_storage",
        "VK_KHR_8bit_storage",
        "VK_EXT_descriptor_indexing",
        "VK_EXT_scalar_block_layout",
        "VK_EXT_shader_atomic_float",
        "VK_EXT_subgroup_size_control",
        "VK_KHR_buffer_device_address",
        "VK_KHR_maintenance1",
        "VK_KHR_maintenance4",
        "VK_KHR_shader_float16_int8",
        "VK_KHR_spirv_1_4",
        "VK_KHR_storage_buffer_storage_class",
        "VK_KHR_swapchain",
    ];

    /// Feature / structure field names that must be enabled alongside the
    /// extensions above (the `VkPhysicalDevice*Features` / `*Features2` chain).
    pub const DEVICE_FEATURES: &[&str] = &[
        "shaderFloat16",
        "shaderInt8",
        "shaderInt64",
        "bufferDeviceAddress",
        "shaderBufferFloat16Atomics",
        "shaderSharedFloat16Atomics",
        "descriptorIndexing",
        "shaderSampledImageArrayNonUniformIndexing",
        "shaderStorageBufferArrayNonUniformIndexing",
        "storageBuffer8BitAccess",
        "storageBuffer16BitAccess",
        "subgroupSizeControl",
        "scalarBlockLayout",
    ];

    /// The `DEVICE_EXTENSIONS` slice, for callers that prefer a function.
    pub fn required_device_extensions() -> &'static [&'static str] {
        DEVICE_EXTENSIONS
    }

    /// The `DEVICE_FEATURES` slice, for callers that prefer a function.
    pub fn required_device_features() -> &'static [&'static str] {
        DEVICE_FEATURES
    }
}

#[cfg(test)]
mod tests {
    use super::device_extensions::*;
    use std::collections::BTreeSet;

    #[test]
    fn device_requirements_are_non_empty_and_unique() {
        assert!(!DEVICE_EXTENSIONS.is_empty());
        assert!(!DEVICE_FEATURES.is_empty());
        for list in [DEVICE_EXTENSIONS, DEVICE_FEATURES] {
            let mut seen = BTreeSet::new();
            for name in list {
                assert!(seen.insert(*name), "duplicate device requirement: {name}");
            }
        }
        assert!(!required_device_extensions().is_empty());
        assert!(!required_device_features().is_empty());
    }
}
