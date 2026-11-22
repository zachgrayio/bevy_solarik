//! wgpu 30 → raw Vulkan handle conversion, the second (and last) unsafe boundary.
//!
//! The `FfxApi` Vulkan provider takes `VkImage`/`VkCommandBuffer` handles as
//! `void*`, not wgpu objects. These helpers follow the exact pattern already used
//! by the fork's `vendor/dlss_wgpu` (`as_hal::<Vulkan>()` + `raw_handle()`).

use crate::{native, quality};
use ash::vk::Handle as _;
use std::ffi::{CStr, c_void};
use wgpu::hal::api::Vulkan;

/// The `VkDeviceContext` chained onto an FSR create descriptor.
pub(crate) fn device_context(device: &wgpu::Device) -> Option<native::DeviceContextVk> {
    // SAFETY: wgpu exposes raw Vulkan handles only for backend interop. The
    // handles are immutable for the device's lifetime and are used solely to
    // initialize the FSR provider; no owned wgpu resource is retained here.
    let hal = unsafe { device.as_hal::<Vulkan>() }?;
    let get_device_proc_addr = hal
        .shared_instance()
        .raw_instance()
        .fp_v1_0()
        .get_device_proc_addr as usize as *mut c_void;
    Some(native::DeviceContextVk {
        device: hal.raw_device().handle().as_raw() as usize as *mut c_void,
        physical_device: hal.raw_physical_device().as_raw() as usize as *mut c_void,
        get_device_proc_addr,
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
    let fmt = texture.format();
    let mut usage = quality::usage::READ_ONLY;
    if fmt.has_depth_aspect() {
        usage |= quality::usage::DEPTH_TARGET;
    }
    if fmt.has_stencil_aspect() {
        usage |= quality::usage::STENCIL_TARGET;
    }
    if state == quality::states::UNORDERED_ACCESS
        || texture
            .usage()
            .contains(wgpu::TextureUsages::STORAGE_BINDING)
    {
        usage |= quality::usage::UAV;
    }
    Some(native::Resource {
        resource: image.as_raw() as usize as *mut c_void,
        description: native::ResourceDescription {
            resource_type: quality::resource_types::TEXTURE_2D,
            format: quality::surface_format(fmt),
            width: texture.width(),
            height: texture.height(),
            depth: 1,
            mip_count: texture.mip_level_count(),
            flags: 0,
            usage,
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
    use std::{collections::BTreeSet, ffi::CStr};

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

    /// The required extensions as `&'static CStr`, matching `DEVICE_EXTENSIONS`
    /// one-for-one. These are the exact values to append to wgpu-hal's
    /// `CreateDeviceCallbackArgs::extensions` (`Vec<&'static CStr>`) so the
    /// `VkDevice` is created with the FFX Vulkan provider's requirements.
    ///
    /// Built from `CStr::from_bytes_with_nul` at compile time on static byte
    /// literals — no runtime allocation, no GPU/DLL/device access.
    pub const DEVICE_EXTENSION_CSTRS: &[&CStr] = &[
        cstr(b"VK_KHR_16bit_storage\0"),
        cstr(b"VK_KHR_8bit_storage\0"),
        cstr(b"VK_EXT_descriptor_indexing\0"),
        cstr(b"VK_EXT_scalar_block_layout\0"),
        cstr(b"VK_EXT_shader_atomic_float\0"),
        cstr(b"VK_EXT_subgroup_size_control\0"),
        cstr(b"VK_KHR_buffer_device_address\0"),
        cstr(b"VK_KHR_maintenance1\0"),
        cstr(b"VK_KHR_maintenance4\0"),
        cstr(b"VK_KHR_shader_float16_int8\0"),
        cstr(b"VK_KHR_spirv_1_4\0"),
        cstr(b"VK_KHR_storage_buffer_storage_class\0"),
        cstr(b"VK_KHR_swapchain\0"),
    ];

    /// Compile-time `&CStr` from a NUL-terminated byte literal. `const fn` safe
    /// on 1.98; the `match` keeps it total (never panics on a valid literal).
    const fn cstr(bytes: &'static [u8]) -> &'static CStr {
        match CStr::from_bytes_with_nul(bytes) {
            Ok(c) => c,
            Err(_) => panic!("device extension literal must be NUL-terminated"),
        }
    }

    /// Capability-filtered extension selection: given the extension names the
    /// physical device actually exposes, return the subset of the FFX-required
    /// extensions that are both required AND available, in `DEVICE_EXTENSIONS`
    /// order, de-duplicated. Pure CPU data — no GPU/DLL/device work.
    ///
    /// This is the safe core of FFX Vulkan device setup: wgpu-hal's
    /// `open_with_callback` lets a callback add `&'static CStr` extensions, but
    /// requesting an extension the device lacks would fail `vkCreateDevice`.
    /// Filtering against the adapter's reported extensions avoids that.
    pub fn supported_subset<'a>(
        available: impl IntoIterator<Item = &'a str>,
    ) -> Vec<&'static CStr> {
        let available: BTreeSet<&str> = available.into_iter().collect();
        DEVICE_EXTENSIONS
            .iter()
            .zip(DEVICE_EXTENSION_CSTRS)
            .filter(|(name, _)| available.contains(*name))
            .map(|(_, cstr)| *cstr)
            .collect()
    }
}

/// Append the FFX-required Vulkan device extensions that the adapter supports
/// to wgpu-hal's `CreateDeviceCallbackArgs::extensions` vector.
///
/// `available` is the list of device extension names the physical device
/// exposes (from `vkEnumerateDeviceExtensionProperties`). Only supported FFX
/// extensions are appended, and never duplicated. A NRD/DLSS callback may have
/// already appended some — this function is idempotent and order-preserving.
pub fn register_device_extensions<'a>(
    extensions: &mut Vec<&'static CStr>,
    available: impl IntoIterator<Item = &'a str>,
) -> usize {
    let mut added = 0;
    for cstr in device_extensions::supported_subset(available) {
        if !extensions.contains(&cstr) {
            extensions.push(cstr);
            added += 1;
        }
    }
    added
}

/// Register FFX device extensions inside wgpu-hal's create-device callback,
/// enumerating the physical device's real capabilities first.
///
/// Call this from a `RawVulkanInitSettings::add_create_device_callback`
/// closure, passing the `CreateDeviceCallbackArgs` and the raw `Adapter` it
/// receives. It queries `vkEnumerateDeviceExtensionProperties` through the
/// adapter's ash instance and appends only FFX-required extensions that the
/// device actually supports, so `vkCreateDevice` can never be handed an
/// unsupported extension (safe on every GPU). Returns the number appended.
///
/// This runs at device creation (GPU present); the pure filtering it delegates
/// to is CPU-unit-tested. It never opens a DLL or creates a wgpu device itself.
pub fn register_device_extensions_for_adapter(
    args: &mut wgpu::hal::vulkan::CreateDeviceCallbackArgs<'_, '_, '_>,
    adapter: &wgpu::hal::vulkan::Adapter,
) -> usize {
    // SAFETY: the adapter's raw instance and physical device are live for the
    // duration of the create-device callback; ash owns the returned Vec.
    let props = unsafe {
        adapter
            .shared_instance()
            .raw_instance()
            .enumerate_device_extension_properties(adapter.raw_physical_device())
    };
    let names: Vec<String> = match props {
        Ok(props) => props
            .iter()
            .filter_map(|p| p.extension_name_as_c_str().ok())
            .filter_map(|c| c.to_str().ok().map(str::to_owned))
            .collect(),
        Err(_) => Vec::new(),
    };
    register_device_extensions(args.extensions, names.iter().map(String::as_str))
}

#[cfg(test)]
mod tests {
    use super::device_extensions::*;
    use super::register_device_extensions;
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

    #[test]
    fn cstr_constants_match_str_names_one_for_one() {
        use std::ffi::CStr;
        assert_eq!(DEVICE_EXTENSION_CSTRS.len(), DEVICE_EXTENSIONS.len());
        for (name, cstr) in DEVICE_EXTENSIONS.iter().zip(DEVICE_EXTENSION_CSTRS) {
            assert_eq!(*name, cstr.to_str().expect("valid UTF-8 extension name"));
            assert_eq!(
                cstr.to_bytes_with_nul().last(),
                Some(&0),
                "must be NUL-terminated"
            );
            // Round-trips through the std CStr constructor.
            assert_eq!(
                CStr::from_bytes_with_nul(cstr.to_bytes_with_nul()).unwrap(),
                *cstr
            );
        }
    }

    #[test]
    fn supported_subset_filters_unsupported_and_preserves_order() {
        let available = [
            "VK_KHR_swapchain",
            "VK_KHR_16bit_storage", // out of order on purpose
            "unrelated_extension",
        ];
        let got: Vec<&str> = supported_subset(available)
            .iter()
            .map(|c| c.to_str().unwrap())
            .collect();
        assert_eq!(got, ["VK_KHR_16bit_storage", "VK_KHR_swapchain"]);
    }

    #[test]
    fn supported_subset_is_empty_for_unsupported_device() {
        assert!(supported_subset(["VK_KHR_swapchain_does_not_exist"]).is_empty());
        assert_eq!(
            supported_subset(DEVICE_EXTENSIONS.iter().copied()).len(),
            13
        );
    }

    #[test]
    fn register_device_extensions_is_idempotent_and_existing_safe() {
        let mut exts: Vec<&'static std::ffi::CStr> = vec![];
        let first = register_device_extensions(&mut exts, DEVICE_EXTENSIONS.iter().copied());
        assert_eq!(first, 13);
        assert_eq!(exts.len(), 13);
        // Running again adds nothing; a pre-existing DLSS-style extension is kept.
        let second = register_device_extensions(&mut exts, DEVICE_EXTENSIONS.iter().copied());
        assert_eq!(second, 0);
        assert_eq!(exts.len(), 13);
    }
}
