//! Source-owned NGX D3D12 initialization and scoped access.
//!
//! Register before DefaultPlugins. The outer [`crate::run_app`] owner and SDK feature leases
//! keep the initialized runtime alive; final teardown is explicit, never a static leak or ECS
//! resource-drop ordering assumption.
use bevy_app::App;
use bevy_render::renderer::RawDx12InitSettings;
use std::{
    ops::Deref,
    sync::{Mutex, MutexGuard, OnceLock},
};
use uuid::Uuid;

fn sdk_slot() -> &'static Mutex<Option<D3D12SdkHolder>> {
    static SDK: OnceLock<Mutex<Option<D3D12SdkHolder>>> = OnceLock::new();
    SDK.get_or_init(|| Mutex::new(None))
}
struct D3D12SdkHolder {
    sdk: dlss_wgpu::dx12::D3D12Sdk,
    support: dlss_wgpu::dx12::Dx12FeatureSupport,
}
#[expect(
    unsafe_code,
    reason = "The owned NGX bootstrap is accessed through its mutex and retained session lifetime"
)]
// SAFETY: ownership/access is serialized by sdk_slot; no raw handle escapes scoped access.
unsafe impl Send for D3D12SdkHolder {}

/// Install NGX D3D12 device initialization before renderer construction. Native source failures
/// remain real capability failures; this hook never fabricates support or creates a second device.
pub fn install(app: &mut App, project_id: [u8; 16]) {
    if app.is_plugin_added::<crate::graphics_session::SessionPlugin>() {
        return;
    }
    app.add_plugins(crate::graphics_session::SessionPlugin);
    let id = Uuid::from_bytes(project_id);
    let mut settings = app
        .world_mut()
        .get_resource_or_init::<RawDx12InitSettings>();
    #[expect(
        unsafe_code,
        reason = "The renderer provides its live native D3D12 device to this initialization-only callback"
    )]
    // SAFETY: callback registration is capability-preserving; args.device is live and the SDK
    // retains its own COM reference before this callback returns.
    unsafe {
        settings.add_create_device_callback(move |args| {
            let mut slot = sdk_slot().lock().unwrap();
            if let Some(existing) = slot.as_ref() {
                assert_eq!(
                    existing.sdk.device, args.device,
                    "Only one native NGX D3D12 device may be active in a graphics session"
                );
                return;
            }
            match dlss_wgpu::dx12::init_with_d3d12_device(id, args.device) {
                Ok(sdk) => {
                    let sr = sdk
                        .feature_supported(dlss_wgpu::dx12::Dx12Feature::SuperResolution)
                        .unwrap_or(false);
                    let rr = sdk
                        .feature_supported(dlss_wgpu::dx12::Dx12Feature::RayReconstruction)
                        .unwrap_or(false);
                    let support = dlss_wgpu::dx12::Dx12FeatureSupport {
                        super_resolution: sr,
                        ray_reconstruction: rr,
                    };
                    *slot = Some(D3D12SdkHolder { sdk, support });
                    tracing::info!(
                        dlss_sr = sr,
                        dlss_rr = rr,
                        "NGX D3D12 initialized for DLSS / DLSS RR"
                    );
                }
                Err(error) => {
                    tracing::info!(%error, "NGX D3D12 unavailable; DLSS on DX12 disabled")
                }
            }
        });
    }
}

/// Whether this session has a successfully initialized D3D12 NGX bootstrap.
pub fn ngx_d3d12_ready() -> bool {
    sdk_slot().lock().unwrap().is_some()
}

/// Cached source-probed support; querying it does not call the native SDK during rendering.
pub fn ngx_d3d12_support() -> dlss_wgpu::dx12::Dx12FeatureSupport {
    sdk_slot()
        .lock()
        .unwrap()
        .as_ref()
        .map(|holder| holder.support)
        .unwrap_or(dlss_wgpu::dx12::Dx12FeatureSupport {
            super_resolution: false,
            ray_reconstruction: false,
        })
}

/// Scoped bootstrap access. Unlike a static reference, this cannot survive native shutdown.
/// Callers must still obey NGX's API synchronization requirements for direct native operations.
pub struct D3D12SdkAccess {
    holder: MutexGuard<'static, Option<D3D12SdkHolder>>,
}
impl Deref for D3D12SdkAccess {
    type Target = dlss_wgpu::dx12::D3D12Sdk;
    fn deref(&self) -> &Self::Target {
        &self.holder.as_ref().expect("Scoped live SDK").sdk
    }
}
/// Access the initialized SDK while retaining its scoped guard.
pub fn ngx_d3d12_sdk() -> Option<D3D12SdkAccess> {
    let holder = sdk_slot().lock().unwrap();
    holder.is_some().then_some(D3D12SdkAccess { holder })
}

pub(crate) struct NgxShutdown(Option<D3D12SdkHolder>);
impl Drop for NgxShutdown {
    fn drop(&mut self) {
        if self.0.is_some() {
            tracing::info!("Native graphics session: NGX SDK shutdown");
        }
        drop(self.0.take());
    }
}

pub(crate) fn prepare_shutdown() -> NgxShutdown {
    let mut holder = sdk_slot().lock().unwrap().take();
    if let Some(holder) = &mut holder {
        tracing::info!("Native graphics session: bootstrap parameter destruction");
        #[expect(
            unsafe_code,
            reason = "Final session ownership proves all SDK feature and parameter clients are gone"
        )]
        // SAFETY: all App/SDK leases have ended. The returned owner retains the SDK/device
        // until after provider shutdown; native calls occur outside the global slot mutex.
        unsafe {
            if let Err(error) = holder.sdk.release_parameters() {
                tracing::warn!(?error, "NGX bootstrap parameter destruction failed");
            }
        }
    }
    NgxShutdown(holder)
}

/// Plugin form of [`install`], registered before the renderer.
#[derive(Clone, Copy)]
pub struct Dx12NgxInitPlugin {
    pub project_id: [u8; 16],
}
impl bevy_app::Plugin for Dx12NgxInitPlugin {
    fn build(&self, app: &mut App) {
        install(app, self.project_id);
    }
}
