//! Per-window requests and receipts for successful native surface configuration.
use bevy_ecs::prelude::*;
use std::sync::{Arc, Mutex};

/// Portable configuration identity, independent of pointer or native handle reuse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowSurfaceStamp {
    pub surface_id: u64,
    pub generation: u64,
    pub request_epoch: u64,
    pub width: u32,
    pub height: u32,
}

/// Shared main/render output channel attached to the corresponding window entity.
/// This reports configuration, not completion of a native Present.
#[derive(Component, Clone, Debug, Default)]
pub struct WindowSurfaceReceipt(Arc<Mutex<ReceiptState>>);

#[derive(Debug, Default)]
struct ReceiptState {
    configured: Option<WindowSurfaceStamp>,
    supported: Option<bool>,
}

impl WindowSurfaceReceipt {
    /// Last successfully configured native surface for this window.
    /// A changed `surface_id` explicitly reports surface replacement.
    pub fn configured(&self) -> Option<WindowSurfaceStamp> {
        self.0.lock().unwrap().configured
    }

    /// Whether the selected backend can provide native configuration stamps.
    /// `None` means the render world has not inspected this window yet.
    pub fn supported(&self) -> Option<bool> {
        self.0.lock().unwrap().supported
    }

    pub(super) fn set_supported(&self, supported: bool) {
        let mut state = self.0.lock().unwrap();
        state.supported = Some(supported);
        if !supported {
            state.configured = None;
        }
    }

    pub(super) fn publish(&self, stamp: WindowSurfaceStamp) {
        let mut state = self.0.lock().unwrap();
        state.configured = Some(stamp);
        state.supported = Some(true);
    }
}

/// Configure this window once for `epoch`, including at unchanged dimensions.
/// Insert only after the native window has acknowledged its requested state.
/// On supported backends, zero or mismatching dimensions keep it pending.
/// Unsupported backends ignore the epoch without blocking ordinary rendering;
/// their receipt reports `supported() == Some(false)` and no native stamp.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowReconfigureEpoch {
    pub epoch: u64,
    pub width: u32,
    pub height: u32,
}

/// Unclamped dimensions: a minimized zero-size window must not acknowledge 1x1.
#[doc(hidden)]
#[derive(Component, Clone, Copy)]
pub struct ExtractedSurfaceExtent(pub u32, pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequestDecision {
    Unrequested,
    WaitingForExtent,
    Configure,
    Applied,
}

pub(super) fn request_decision_for_backend(
    supported: bool,
    request: Option<&WindowReconfigureEpoch>,
    applied_epoch: Option<u64>,
    extent: (u32, u32),
) -> RequestDecision {
    if supported {
        request_decision(request, applied_epoch, extent)
    } else {
        // An unsupported receipt is not a presentation/configuration barrier.
        RequestDecision::Unrequested
    }
}

fn request_decision(
    request: Option<&WindowReconfigureEpoch>,
    applied_epoch: Option<u64>,
    extent: (u32, u32),
) -> RequestDecision {
    let Some(request) = request else {
        return RequestDecision::Unrequested;
    };
    if applied_epoch == Some(request.epoch) {
        return RequestDecision::Applied;
    }
    if request.width == 0 || request.height == 0 || extent != (request.width, request.height) {
        return RequestDecision::WaitingForExtent;
    }
    RequestDecision::Configure
}

/// Validate the actual post-configure native result before committing an epoch.
pub(super) fn configuration_succeeded(
    surface_id: u64,
    previous: Option<WindowSurfaceStamp>,
    actual: WindowSurfaceStamp,
    extent: (u32, u32),
    request: Option<&WindowReconfigureEpoch>,
) -> bool {
    actual.surface_id == surface_id
        && actual.generation > previous.map_or(0, |stamp| stamp.generation)
        && (actual.width, actual.height) == extent
        && request.is_none_or(|request| {
            actual.request_epoch == request.epoch
                && (actual.width, actual.height) == (request.width, request.height)
        })
}

#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "android",
    feature = "vulkan-portability"
))]
impl From<wgpu::hal::surface_stamp::SurfaceStamp> for WindowSurfaceStamp {
    fn from(stamp: wgpu::hal::surface_stamp::SurfaceStamp) -> Self {
        Self {
            surface_id: stamp.surface_id,
            generation: stamp.generation,
            request_epoch: stamp.request_epoch,
            width: stamp.width,
            height: stamp.height,
        }
    }
}

/// Borrow only for an atomic epoch request; retain no raw HAL handle.
pub(super) fn request_native_epoch(surface: &wgpu::Surface<'_>, epoch: u64) -> Option<u64> {
    #[cfg(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "android",
        feature = "vulkan-portability"
    ))]
    // SAFETY: the guard owns the borrow. Only the HAL's epoch atomic is updated;
    // no configuration, destruction or raw GPU operation occurs through it.
    unsafe {
        if let Some(native) = surface.as_hal::<wgpu::hal::api::Vulkan>() {
            native.request_reconfigure_epoch(epoch);
            return Some(native.presentation_surface_id());
        }
    }
    #[cfg(target_os = "windows")]
    // SAFETY: same restricted, guarded epoch operation as the Vulkan branch.
    unsafe {
        if let Some(native) = surface.as_hal::<wgpu::hal::api::Dx12>() {
            native.request_reconfigure_epoch(epoch);
            return Some(native.presentation_surface_id());
        }
    }
    let _ = (surface, epoch);
    None
}

pub(super) fn native_stamp(surface: &wgpu::Surface<'_>) -> Option<WindowSurfaceStamp> {
    #[cfg(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "android",
        feature = "vulkan-portability"
    ))]
    // SAFETY: immutable copied metadata only; the guard keeps the surface alive.
    unsafe {
        if let Some(native) = surface.as_hal::<wgpu::hal::api::Vulkan>() {
            return native.presentation_stamp().map(Into::into);
        }
    }
    #[cfg(target_os = "windows")]
    // SAFETY: immutable copied metadata only; no raw handle escapes the guard.
    unsafe {
        if let Some(native) = surface.as_hal::<wgpu::hal::api::Dx12>() {
            return native.presentation_stamp().map(Into::into);
        }
    }
    let _ = surface;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> WindowReconfigureEpoch {
        WindowReconfigureEpoch {
            epoch: 9,
            width: 1280,
            height: 720,
        }
    }

    #[test]
    fn same_size_request_forces_configure_once() {
        let request = request();
        assert_eq!(
            request_decision(Some(&request), None, (1280, 720)),
            RequestDecision::Configure
        );
        assert_eq!(
            request_decision(Some(&request), Some(8), (1280, 720)),
            RequestDecision::Configure
        );
        assert_eq!(
            request_decision(Some(&request), Some(9), (1280, 720)),
            RequestDecision::Applied
        );
        assert_eq!(
            request_decision(None, Some(9), (1280, 720)),
            RequestDecision::Unrequested
        );
    }

    #[test]
    fn wrong_and_zero_extents_remain_pending() {
        let request = request();
        for extent in [(0, 0), (1, 1), (1280, 719), (1920, 1080)] {
            assert_eq!(
                request_decision(Some(&request), Some(8), extent),
                RequestDecision::WaitingForExtent
            );
        }
    }

    #[test]
    fn failed_or_mismatched_configuration_cannot_commit() {
        let request = request();
        let before = WindowSurfaceStamp {
            surface_id: 3,
            generation: 4,
            request_epoch: 8,
            width: 1280,
            height: 720,
        };
        let success = WindowSurfaceStamp {
            generation: 5,
            request_epoch: 9,
            ..before
        };
        assert!(configuration_succeeded(
            3,
            Some(before),
            success,
            (1280, 720),
            Some(&request)
        ));
        for actual in [
            before,
            WindowSurfaceStamp {
                surface_id: 4,
                ..success
            },
            WindowSurfaceStamp {
                request_epoch: 8,
                ..success
            },
            WindowSurfaceStamp {
                width: 1279,
                ..success
            },
        ] {
            assert!(!configuration_succeeded(
                3,
                Some(before),
                actual,
                (1280, 720),
                Some(&request)
            ));
        }
    }

    #[test]
    fn unsupported_backend_requests_do_not_block_ordinary_rendering() {
        let request = request();
        assert_eq!(
            request_decision_for_backend(false, Some(&request), None, (0, 0)),
            RequestDecision::Unrequested,
        );
        assert_eq!(
            request_decision_for_backend(false, Some(&request), None, (1280, 720)),
            RequestDecision::Unrequested,
        );
        let receipt = WindowSurfaceReceipt::default();
        assert_eq!(receipt.supported(), None);
        receipt.set_supported(false);
        assert_eq!(receipt.supported(), Some(false));
        assert_eq!(receipt.configured(), None);
        // A replaced backend must not retain a stale previous native identity.
        receipt.publish(WindowSurfaceStamp {
            surface_id: 9,
            generation: 1,
            ..Default::default()
        });
        receipt.set_supported(false);
        assert_eq!(receipt.configured(), None);
    }

    #[test]
    fn window_receipts_do_not_share_channels() {
        let first = WindowSurfaceReceipt::default();
        let second = WindowSurfaceReceipt::default();
        let first_render = first.clone();
        let stamp = WindowSurfaceStamp {
            surface_id: 3,
            generation: 1,
            ..Default::default()
        };
        first_render.publish(stamp);
        assert_eq!(first.configured(), Some(stamp));
        assert_eq!(second.configured(), None);
        first_render.publish(WindowSurfaceStamp {
            surface_id: 8,
            ..stamp
        });
        assert_eq!(first.configured().unwrap().surface_id, 8);
    }

    #[test]
    fn already_applied_epoch_never_stalls_a_later_os_resize() {
        let request = request();
        assert_eq!(
            request_decision(Some(&request), Some(request.epoch), (3840, 2071)),
            RequestDecision::Applied,
        );
    }
}
