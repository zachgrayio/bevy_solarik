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
