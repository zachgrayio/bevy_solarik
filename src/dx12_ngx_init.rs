//! NVIDIA NGX D3D12 initialization for the consuming application.
//!
//! The Vulkan DLSS path is initialized by `bevy_anti_alias`'s `DlssInitPlugin`
//! (which registers Vulkan create-device callbacks on `RawVulkanInitSettings`).
//! On a **D3D12 backend** that path is not applicable: NGX D3D12 must be
//! initialized once with the raw `ID3D12Device*`. Fork `bevy_render` now
//! dispatches `RawDx12InitSettings` create-device callbacks from
//! `initialize_renderer` with exactly that pointer. This module registers the
//! NGX D3D12 init callback and holds the process-global NGX SDK handle.
//!
//! Compiled only with feature `dx12_ngx_init`. The callback is capability-safe:
//! on non-NVIDIA / no-NGX systems `init_with_d3d12_device` returns an error which
//! is logged and ignored, so the game keeps running with its other backends —
//! no capability is fabricated and no GPU work is done at app-construction time.

use bevy_app::App;
use bevy_render::renderer::RawDx12InitSettings;
use std::sync::OnceLock;
use uuid::Uuid;

/// Process-global NGX D3D12 SDK handle. Stored once at device creation and kept
/// alive for the application's lifetime (NGX is an application-global SDK; there
/// is no per-frame recreation). The inner `D3D12Sdk` owns the capability
/// parameters and its `Drop` shuts NGX down — keeping it here means shutdown
/// happens at process teardown, not mid-frame.
static NGX_D3D12_SDK: OnceLock<D3D12SdkHolder> = OnceLock::new();

/// `Sync` wrapper: the raw NGX handles inside `D3D12Sdk` live for the whole
/// device lifetime, are set exactly once before rendering starts, and are only
/// read by NGX feature creation on the render thread. No mutation after init.
/// The field is intentionally never read: it is kept alive for its `Drop`.
#[expect(
    dead_code,
    reason = "D3D12Sdk is retained only so NGX is not shut down; only its Drop matters"
)]
struct D3D12SdkHolder(dlss_wgpu::dx12::D3D12Sdk);
#[expect(
    unsafe_code,
    reason = "NGX D3D12 SDK is a process-global, init-once handle"
)]
// SAFETY: the handle is write-once (set before rendering) and read-only
// thereafter; NGX state is process-global, no cross-thread mutation occurs.
unsafe impl Send for D3D12SdkHolder {}
#[expect(
    unsafe_code,
    reason = "NGX D3D12 SDK is a process-global, init-once handle"
)]
// SAFETY: see the Send impl — the raw handles are only read after init.
unsafe impl Sync for D3D12SdkHolder {}

/// Install the NGX D3D12 init callback for the application. Call before
/// `DefaultPlugins` / `RenderPlugin` builds the device (BARD calls this from
/// `configure`). `project_id` is BARD's NGX project UUID bytes passed by the
/// application — the fork has no application-specific constants.
pub fn install(app: &mut App, project_id: [u8; 16]) {
    let id = Uuid::from_bytes(project_id);
    let mut settings = app
        .world_mut()
        .get_resource_or_init::<RawDx12InitSettings>();

    // SAFETY: registering a callback whose body only forwards a copy of the
    // live `ID3D12Device*` to NGX init and logs on failure — it never removes
    // or fabricates device capabilities, queue infos or create-info state.
    #[expect(
        unsafe_code,
        reason = "registering an init-only, capability-preserving NGX D3D12 callback"
    )]
    unsafe {
        // SAFETY: the registered closure upholds the add_create_device_callback
        // contract as described above; it is add-only and capability-preserving.
        settings.add_create_device_callback(move |args| {
            match dlss_wgpu::dx12::init_with_d3d12_device(id, args.device) {
                Ok(sdk) => {
                    let _ = NGX_D3D12_SDK.set(D3D12SdkHolder(sdk));
                    tracing::info!("NGX D3D12 initialized for DLSS / DLSS RR");
                }
                Err(error) => {
                    tracing::info!(%error, "NGX D3D12 unavailable; DLSS on DX12 disabled");
                }
            }
        });
    }
}

/// Whether NGX D3D12 initialized successfully (queried by the capability probe).
pub fn ngx_d3d12_ready() -> bool {
    NGX_D3D12_SDK.get().is_some()
}

/// A Bevy plugin form, for applications that prefer `add_plugins`.
#[derive(Clone, Copy)]
pub struct Dx12NgxInitPlugin {
    pub project_id: [u8; 16],
}

impl bevy_app::Plugin for Dx12NgxInitPlugin {
    fn build(&self, app: &mut App) {
        install(app, self.project_id);
    }
}
