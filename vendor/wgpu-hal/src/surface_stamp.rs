//! Native surface configuration identity used by presentation integrations.
use core::sync::atomic::{AtomicU64, Ordering};

static NEXT_SURFACE_ID: AtomicU64 = AtomicU64::new(1);

/// Immutable identity of one successfully configured native surface.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SurfaceStamp {
    /// Process-unique surface identity, independent of native pointer reuse.
    pub surface_id: u64,
    /// Successful configuration number on this surface.
    pub generation: u64,
    /// Application request committed by this configuration, or zero.
    pub request_epoch: u64,
    /// Configured physical width.
    pub width: u32,
    /// Configured physical height.
    pub height: u32,
}

/// Per-surface request/configuration counters. Failed configurations do not commit.
#[derive(Debug)]
pub struct SurfaceStampSource {
    surface_id: u64,
    requested_epoch: AtomicU64,
    generation: AtomicU64,
}

impl Default for SurfaceStampSource {
    fn default() -> Self {
        Self {
            surface_id: NEXT_SURFACE_ID.fetch_add(1, Ordering::Relaxed),
            requested_epoch: AtomicU64::new(0),
            generation: AtomicU64::new(0),
        }
    }
}

impl SurfaceStampSource {
    /// Stable identity for the lifetime of this surface.
    pub fn surface_id(&self) -> u64 {
        self.surface_id
    }

    /// Associate the next successful configuration with an application request.
    pub fn request(&self, epoch: u64) {
        self.requested_epoch.store(epoch, Ordering::Release);
    }

    /// Prepare an immutable candidate without acknowledging configuration success.
    pub fn candidate(&self, width: u32, height: u32) -> SurfaceStamp {
        SurfaceStamp {
            surface_id: self.surface_id,
            generation: self.generation.load(Ordering::Acquire) + 1,
            request_epoch: self.requested_epoch.load(Ordering::Acquire),
            width,
            height,
        }
    }

    /// Commit only after successful native configuration. Native callers serialize configure.
    pub fn commit(&self, stamp: SurfaceStamp) {
        debug_assert_eq!(stamp.surface_id, self.surface_id);
        self.generation.store(stamp.generation, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_successful_configuration_advances_generation() {
        let source = SurfaceStampSource::default();
        source.request(7);
        let failed = source.candidate(1280, 720);
        assert_eq!(failed, source.candidate(1280, 720));
        source.commit(failed);
        let same_size = source.candidate(1280, 720);
        assert_eq!(same_size.surface_id, failed.surface_id);
        assert_eq!(same_size.generation, failed.generation + 1);
        assert_eq!(same_size.request_epoch, 7);
    }
    #[test]
    fn surface_identity_is_not_a_native_pointer() {
        let first = SurfaceStampSource::default();
        let second = SurfaceStampSource::default();
        assert_ne!(first.surface_id(), second.surface_id());
    }
}
