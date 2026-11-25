//! Dynamically loaded NVIDIA Streamline 2.14.16 integration (DLSS-G / MFG + Reflex)
//! for wgpu 30 on both DX12 and Vulkan.
//!
//! Zero link-time dependency on any NVIDIA library: `sl.interposer.dll` is loaded
//! dynamically with `libloading` at runtime. When absent or on non-NVIDIA GPUs,
//! initialization returns `Err` cleanly and the engine falls back gracefully.

pub mod abi;

#[allow(
    unsafe_code,
    reason = "Dynamic FFI boundary to sl.interposer.dll and wgpu-hal swapchain hooks"
)]
mod runtime;

pub use runtime::{
    Error, FrameGenCapabilities, FrameGenState, RetiredRuntime, Streamline, attach_device,
    current_capabilities, dlss_optimal_render_size, evaluate_dlss_rr, evaluate_dlss_sr,
    install_swapchain_interception, is_ready, is_vulkan_interposed, preinit, record_frame,
    shutdown, shutdown_retaining_runtime,
};
