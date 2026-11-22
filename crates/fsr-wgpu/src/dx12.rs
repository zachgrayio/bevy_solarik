//! wgpu 30 → raw D3D12 handle conversion for the FidelityFX DX12 provider.
//!
//! Mirrors [`crate::vulkan`]: extracts raw `ID3D12Device*`, `ID3D12Resource*`
//! and `ID3D12GraphicsCommandList*` pointers via wgpu-hal's `as_hal::<Dx12>()`
//! without linking against any Windows SDK crate.

#![cfg(windows)]

use crate::{native, quality};
use std::ffi::c_void;
use wgpu::hal::api::Dx12;

/// Raw `ID3D12Device*` for `ffxCreateBackendDX12Desc::device`.
pub(crate) fn device_ptr(device: &wgpu::Device) -> Option<*mut c_void> {
    // SAFETY: wgpu exposes the raw D3D12 device handle for native interop; the
    // handle stays alive for the lifetime of `wgpu::Device`.
    let hal = unsafe { device.as_hal::<Dx12>() }?;
    Some(hal.raw_device_ptr())
}

/// A `FfxApiResource` wrapping the `ID3D12Resource*` behind a wgpu texture view.
pub(crate) fn resource(view: &wgpu::TextureView, state: u32) -> Option<native::Resource> {
    let texture = view.texture();
    // SAFETY: `as_hal` borrows the live wgpu texture for the duration of this
    // call; the raw `ID3D12Resource*` is passed directly into the dispatch desc.
    let ptr = unsafe { texture.as_hal::<Dx12>().map(|hal| hal.raw_resource_ptr()) }?;
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
        resource: ptr,
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

/// Run `f` with the open `ID3D12GraphicsCommandList*` of a wgpu encoder.
pub(crate) fn with_command_list<R>(
    encoder: &mut wgpu::CommandEncoder,
    f: impl FnOnce(*mut c_void) -> R,
) -> Option<R> {
    // SAFETY: `as_hal_mut` opens the underlying D3D12 command list for raw
    // recording; `f` records FFX commands strictly within this scope.
    unsafe {
        encoder
            .as_hal_mut::<Dx12, _, _>(|hal| hal.and_then(|hal| hal.raw_command_list_ptr()).map(f))
    }
}
