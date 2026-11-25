//! Dynamic loader and runtime controller for NVIDIA Streamline 2.14.16.

use crate::abi::*;
use ash::vk::Handle as _;
use std::{
    collections::HashMap,
    ffi::{CString, c_char, c_void},
    path::PathBuf,
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

/// Errors returned while loading or driving Streamline.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `sl.interposer.dll` was not found or could not be opened.
    #[error("Streamline interposer not found: {0}")]
    NotFound(String),
    /// A required C export was missing from `sl.interposer.dll`.
    #[error("Streamline interposer missing symbol {0}: {1}")]
    Symbol(&'static str, String),
    /// Streamline API returned a non-zero `sl::Result`.
    #[error("Streamline API {0} returned error code {1}")]
    Api(&'static str, SlResult),
    /// Streamline has not been initialized for this process.
    #[error("Streamline is not initialized")]
    NotInitialized,
    /// Active wgpu backend is neither Vulkan nor Windows DX12.
    #[error("Streamline requires DX12 or Vulkan")]
    UnsupportedBackend,
}

/// Real runtime capabilities queried from Streamline (`sl.dlss.dll`,
/// `sl.dlss_d.dll`, `sl.dlss_g.dll`, `sl.reflex.dll`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameGenCapabilities {
    /// Streamline `kFeatureDLSS` (Super Resolution) is supported on this GPU.
    pub dlss_sr_supported: bool,
    /// Streamline `kFeatureDLSS_RR` (Ray Reconstruction) is supported on this GPU.
    pub dlss_rr_supported: bool,
    /// Streamline initialized and `kFeatureDLSS_G` is supported on this GPU/OS.
    pub dlss_g_supported: bool,
    /// Maximum interpolated frames per real frame reported by `slDLSSGGetState`
    /// (`1` = 2x on RTX 40-series; `>= 2` = 3x/4x/6x MFG on RTX 50-series).
    pub max_generated_frames: u32,
    /// True when hardware supports Multi-Frame Generation (`max_generated_frames >= 2`).
    pub mfg_supported: bool,
    /// True when NVIDIA Reflex low-latency pacing is available.
    pub reflex_supported: bool,
}

/// Live per-frame telemetry from `slDLSSGGetState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameGenState {
    /// Raw `sl::DLSSGStatus` bitfield (`0` = ok).
    pub status: u32,
    /// Frames presented since the previous `slDLSSGGetState` call (real + generated).
    pub num_frames_actually_presented: u32,
    /// Hardware max interpolated frames per real frame.
    pub max_generated_frames: u32,
}

type FnSlInit = unsafe extern "C" fn(*const Preferences, u64) -> SlResult;
type FnSlShutdown = unsafe extern "C" fn() -> SlResult;
type FnSlIsFeatureSupported = unsafe extern "C" fn(Feature, *const AdapterInfo) -> SlResult;
type FnSlGetFeatureFunction =
    unsafe extern "C" fn(Feature, *const c_char, *mut *mut c_void) -> SlResult;
type FnSlGetNewFrameToken = unsafe extern "C" fn(*mut *mut BaseStructure, *const u32) -> SlResult;
type FnSlSetConstants =
    unsafe extern "C" fn(*const Constants, *const BaseStructure, *const ViewportHandle) -> SlResult;
type FnSlSetTagForFrame = unsafe extern "C" fn(
    *const BaseStructure,
    *const ViewportHandle,
    *const ResourceTag,
    u32,
    *mut c_void,
) -> SlResult;
type FnSlEvaluateFeature = unsafe extern "C" fn(
    Feature,
    *const BaseStructure,
    *const *const BaseStructure,
    u32,
    *mut c_void,
) -> SlResult;
type FnSlSetD3DDevice = unsafe extern "C" fn(*mut c_void) -> SlResult;
type FnSlSetVulkanInfo = unsafe extern "C" fn(*const VulkanInfo) -> SlResult;
type FnSlUpgradeInterface = unsafe extern "C" fn(*mut *mut c_void) -> SlResult;

type FnDlssSetOptions = unsafe extern "C" fn(*const ViewportHandle, *const DlssOptions) -> SlResult;
type FnDlssGetOptimalSettings =
    unsafe extern "C" fn(*const DlssOptions, *mut DlssOptimalSettings) -> SlResult;
type FnDlssDSetOptions =
    unsafe extern "C" fn(*const ViewportHandle, *const DlssDOptions) -> SlResult;
type FnDlssDGetOptimalSettings =
    unsafe extern "C" fn(*const DlssDOptions, *mut DlssDOptimalSettings) -> SlResult;
type FnDlssGSetOptions =
    unsafe extern "C" fn(*const ViewportHandle, *const DlssGOptions) -> SlResult;
type FnDlssGGetState =
    unsafe extern "C" fn(*const ViewportHandle, *mut DlssGState, *const DlssGOptions) -> SlResult;
type FnReflexSetOptions = unsafe extern "C" fn(*const ReflexOptions) -> SlResult;
type FnReflexSleep = unsafe extern "C" fn(*const BaseStructure) -> SlResult;
type FnPclSetMarker = unsafe extern "C" fn(u32, *const BaseStructure) -> SlResult;

struct CoreExports {
    _library: libloading::Library,
    sl_init: FnSlInit,
    sl_shutdown: FnSlShutdown,
    sl_is_feature_supported: FnSlIsFeatureSupported,
    sl_get_feature_function: FnSlGetFeatureFunction,
    sl_get_new_frame_token: FnSlGetNewFrameToken,
    sl_set_constants: FnSlSetConstants,
    sl_set_tag_for_frame: FnSlSetTagForFrame,
    sl_evaluate_feature: FnSlEvaluateFeature,
    #[cfg(windows)]
    sl_set_d3d_device: FnSlSetD3DDevice,
    sl_set_vulkan_info: FnSlSetVulkanInfo,
    #[cfg(windows)]
    sl_upgrade_interface: FnSlUpgradeInterface,
}

struct FeatureExports {
    dlss_set_options: Option<FnDlssSetOptions>,
    dlss_get_optimal_settings: Option<FnDlssGetOptimalSettings>,
    dlss_d_set_options: Option<FnDlssDSetOptions>,
    dlss_d_get_optimal_settings: Option<FnDlssDGetOptimalSettings>,
    dlss_g_set_options: Option<FnDlssGSetOptions>,
    dlss_g_get_state: Option<FnDlssGGetState>,
    reflex_set_options: Option<FnReflexSetOptions>,
    reflex_sleep: Option<FnReflexSleep>,
    pcl_set_marker: Option<FnPclSetMarker>,
}

/// Owns immutable extension pointees for shallow resource copies retained by SL.
/// Keys are view ranges, not image handles: normal per-frame images reuse entries.
#[derive(Default)]
struct ResourceMetadata {
    ranges: HashMap<[u32; 5], Box<SubresourceRange>>,
}

impl ResourceMetadata {
    fn range_pointer(&mut self, range: ash::vk::ImageSubresourceRange) -> *mut BaseStructure {
        let key = [
            range.aspect_mask.as_raw(),
            range.base_mip_level,
            range.level_count,
            range.base_array_layer,
            range.layer_count,
        ];
        let entry = self.ranges.entry(key).or_insert_with(|| {
            Box::new(SubresourceRange {
                base: BaseStructure::new(GUID_SUBRESOURCE_RANGE, K_STRUCT_VERSION_1),
                aspect_mask: key[0],
                base_mip_level: key[1],
                level_count: key[2],
                base_array_layer: key[3],
                layer_count: key[4],
            })
        });
        // The ABI requires a mutable pointer, but published metadata is never mutated.
        // Box allocations survive map growth and moves of the runtime owner.
        std::ptr::from_ref(&entry.base).cast_mut()
    }
}

/// Bounded observations. A setter result is never treated as SDK mode adoption.
#[derive(Default)]
struct PresentDiagnostics {
    generation: u64,
    ordinal: u64,
    since_change: u64,
    requested_mode: u32,
    previous: Option<(SlResult, u32, u32, usize, u64)>,
}
impl PresentDiagnostics {
    fn reconfigured(&mut self) {
        self.generation += 1;
        self.since_change = 0;
        self.previous = None;
    }
    fn observe(&mut self, state: (SlResult, u32, u32, usize, u64)) -> bool {
        self.ordinal += 1;
        self.since_change += 1;
        // Fence values normally change every frame; do not use them as a spam trigger.
        let changed = self
            .previous
            .is_none_or(|p| (p.0, p.1, p.2, p.3) != (state.0, state.1, state.2, state.3));
        self.previous = Some(state);
        changed || matches!(self.since_change, 1 | 30 | 60 | 120)
    }
}

// v2.14.16 sl_core_types.h: APIError is a four-byte HRESULT/VkResult union
// on Windows, and PFunOnAPIErrorCallback takes const APIError&.
unsafe extern "C" fn on_dlssg_api_error(error: *const i32) {
    if let Some(error) = unsafe { error.as_ref() } {
        tracing::warn!(target: "solarik_fg_diagnostics", error, thread = ?std::thread::current().id(), "DLSS-G underlying presentation API error callback");
    }
}

fn log_native_failure(operation: &str, result: SlResult) {
    if result != SL_RESULT_OK {
        tracing::warn!(target: "solarik_fg_diagnostics", operation, result, thread = ?std::thread::current().id(), "Streamline native call failed");
    }
}

/// Portable native configuration identity, copied from the actual HAL swapchain.
pub use wgpu::hal::surface_stamp::SurfaceStamp;

/// A nonblocking pause bound to one native surface. Mark issuance before changing
/// the window, then arm only after the native window acknowledges application.
/// Completion requires a successful configuration and Present for the exact epoch.
#[derive(Debug)]
pub struct PresentationPause(Arc<Mutex<PauseProgress>>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExpectedSurface {
    epoch: u64,
    width: u32,
    height: u32,
}

#[derive(Debug)]
struct PauseProgress {
    baseline: SurfaceStamp,
    off_after: Option<u64>,
    ready: bool,
    issued_epoch: Option<u64>,
    expected: Option<ExpectedSurface>,
    configured: Option<SurfaceStamp>,
    complete: bool,
    failed: bool,
    bypass_sdk: bool,
    leases: usize,
}

impl PauseProgress {
    fn new(baseline: SurfaceStamp) -> Self {
        Self {
            baseline,
            off_after: None,
            ready: false,
            issued_epoch: None,
            expected: None,
            configured: None,
            complete: false,
            failed: false,
            bypass_sdk: false,
            leases: 1,
        }
    }

    fn immediate() -> Self {
        Self {
            ready: true,
            complete: true,
            bypass_sdk: true,
            ..Self::new(SurfaceStamp::default())
        }
    }

    fn matches_expected(&self, stamp: SurfaceStamp) -> bool {
        self.expected.is_some_and(|expected| {
            stamp.surface_id == self.baseline.surface_id
                && stamp.generation > self.baseline.generation
                && stamp.request_epoch == expected.epoch
                && stamp.width == expected.width
                && stamp.height == expected.height
        })
    }

    fn configured(&mut self, stamp: SurfaceStamp) {
        if !self.failed
            && !self.complete
            && self.matches_expected(stamp)
            && self
                .configured
                .is_none_or(|previous| stamp.generation >= previous.generation)
        {
            self.configured = Some(stamp);
        }
    }

    fn observe(&mut self, stamp: SurfaceStamp, successful: bool, query_ok: bool, ordinal: u64) {
        if self.failed
            || self.complete
            || !successful
            || !query_ok
            || stamp.surface_id != self.baseline.surface_id
            || stamp.generation < self.baseline.generation
        {
            return;
        }
        if self.off_after.is_some_and(|after| ordinal > after) {
            self.ready = true;
        }
        if self.ready
            && self.matches_expected(stamp)
            && self.configured.is_some_and(|configured| {
                configured.surface_id == stamp.surface_id
                    && configured.request_epoch == stamp.request_epoch
                    && configured.width == stamp.width
                    && configured.height == stamp.height
                    && stamp.generation >= configured.generation
            })
        {
            self.complete = true;
        }
    }

    fn mark_issued(&mut self, epoch: u64) -> Result<(), &'static str> {
        if self.bypass_sdk {
            return Ok(());
        }
        if self.failed {
            return Err("Streamline surface pause failed");
        }
        if !self.ready {
            return Err("Surface pause has no ordered OFF Present receipt");
        }
        if self.complete {
            return Err("Surface pause already completed");
        }
        if epoch == 0
            || epoch <= self.baseline.request_epoch
            || self.issued_epoch.is_some_and(|current| epoch < current)
        {
            return Err("Surface mutation epoch is stale");
        }
        if self.issued_epoch != Some(epoch) {
            self.issued_epoch = Some(epoch);
            self.expected = None;
            self.configured = None;
        }
        Ok(())
    }

    fn arm(&mut self, epoch: u64, width: u32, height: u32) -> Result<(), &'static str> {
        if self.bypass_sdk {
            return Ok(());
        }
        if width == 0 || height == 0 {
            return Err("Surface extent is zero; native application remains pending");
        }
        self.mark_issued(epoch)?;
        let expected = ExpectedSurface {
            epoch,
            width,
            height,
        };
        if self.expected.is_some_and(|current| current != expected) {
            return Err("Surface extent changed within one mutation epoch");
        }
        self.expected = Some(expected);
        Ok(())
    }
}

impl Clone for PresentationPause {
    fn clone(&self) -> Self {
        self.0.lock().unwrap().leases += 1;
        Self(Arc::clone(&self.0))
    }
}

impl Drop for PresentationPause {
    fn drop(&mut self) {
        let mut state = self.0.lock().unwrap();
        state.leases -= 1;
        // Once a native mutation is issued, runtime ownership must outlive every
        // caller lease until a matching new-surface receipt makes resumption safe.
        if state.leases == 0 && state.issued_epoch.is_none() {
            state.complete = true;
        }
    }
}

impl PresentationPause {
    /// Poll without blocking the window pump. Errors never report readiness.
    pub fn is_ready(&self) -> Result<bool, &'static str> {
        let state = self.0.lock().unwrap();
        if state.failed {
            Err("Streamline surface pause failed")
        } else {
            Ok(state.ready)
        }
    }

    /// Call immediately before issuing the native window mutation. Dropping all
    /// leases after this call does not release the runtime's finishing pause.
    pub fn mark_native_mutation_issued(&self, request_epoch: u64) -> Result<(), &'static str> {
        self.0.lock().unwrap().mark_issued(request_epoch)
    }

    /// Arm after native application acknowledgment and before requesting renderer
    /// configuration for this epoch. Earlier configure events are not receipts.
    pub fn arm_after_native_applied(
        &self,
        request_epoch: u64,
        expected_width: u32,
        expected_height: u32,
    ) -> Result<(), &'static str> {
        self.0
            .lock()
            .unwrap()
            .arm(request_epoch, expected_width, expected_height)
    }

    /// The old ordinal-only finish operation is deliberately invalid for an
    /// active SDK pause. It cannot distinguish an old in-flight swapchain.
    pub fn finish(&self) {
        let mut state = self.0.lock().unwrap();
        if !state.bypass_sdk {
            state.failed = true;
        }
    }

    /// True only after a matching configured surface successfully presented, or
    /// when this token has no SDK work to pause. No ordinal fallback is used.
    pub fn is_complete(&self) -> bool {
        self.0.lock().unwrap().complete
    }
}

fn failed_pause() -> PresentationPause {
    PresentationPause(Arc::new(Mutex::new(PauseProgress {
        failed: true,
        ..PauseProgress::new(SurfaceStamp::default())
    })))
}

/// Whether SDK presentation needs a target-bound pause. A false result permits
/// native-window-only handling without manufacturing a native surface identity.
/// An initialized DG presenter always requires this protocol, even before its
/// first ON request: it can otherwise become enabled during an identity-free change.
pub fn presentation_pause_required() -> bool {
    let guard = global_slot().lock().unwrap();
    guard
        .as_ref()
        .is_some_and(|sl| sl.capabilities.dlss_g_supported)
}

/// Pause the target surface without changing the application's FG preference.
/// The viewport-0 controller permits one active target, never another window's
/// receipts. Repeated requests for that target share one bounded transaction.
pub fn request_presentation_pause_for_surface(baseline: SurfaceStamp) -> PresentationPause {
    let mut guard = global_slot().lock().unwrap();
    let Some(sl) = guard.as_mut().filter(|sl| sl.capabilities.dlss_g_supported) else {
        return PresentationPause(Arc::new(Mutex::new(PauseProgress::immediate())));
    };
    if baseline.surface_id == 0
        || baseline.generation == 0
        || baseline.width == 0
        || baseline.height == 0
        || sl
            .last_present_stamp
            .is_some_and(|last| last.surface_id != baseline.surface_id)
    {
        return failed_pause();
    }
    if let Some(existing) = &sl.surface_pause {
        let mut state = existing.lock().unwrap();
        if !state.complete {
            if state.baseline.surface_id != baseline.surface_id {
                return failed_pause();
            }
            state.leases += 1;
            return PresentationPause(Arc::clone(existing));
        }
    }
    let mut progress = PauseProgress::new(baseline);
    // The runtime owns this pause before another ON can be recorded. Never-ON
    // does not bypass the token or its post-mutation receipts.
    progress.ready = !sl.has_enabled_dlssg
        || (sl.last_mode == DLSSG_MODE_OFF
            && sl.last_ordered_off_stamp.is_some_and(|last| {
                last.surface_id == baseline.surface_id && last.generation >= baseline.generation
            }));
    let state = Arc::new(Mutex::new(progress));
    sl.surface_pause = Some(Arc::clone(&state));
    tracing::info!(target: "solarik_fg_diagnostics", ?baseline, ordinal = sl.diagnostics.ordinal, "Requested target-surface presentation pause");
    PresentationPause(state)
}

/// Compatibility entry point. Active FG requires an explicit surface identity.
pub fn request_presentation_pause() -> PresentationPause {
    let guard = global_slot().lock().unwrap();
    if guard
        .as_ref()
        .is_none_or(|sl| !sl.capabilities.dlss_g_supported)
    {
        PresentationPause(Arc::new(Mutex::new(PauseProgress::immediate())))
    } else {
        failed_pause()
    }
}

#[cfg(test)]
mod presentation_pause_tests {
    use super::*;

    fn stamp(generation: u64, epoch: u64) -> SurfaceStamp {
        SurfaceStamp {
            surface_id: 7,
            generation,
            request_epoch: epoch,
            width: 1280,
            height: 720,
        }
    }
    fn ready() -> PauseProgress {
        let mut state = PauseProgress::new(stamp(4, 8));
        state.off_after = Some(10);
        state.observe(stamp(4, 8), true, true, 11);
        assert!(state.ready);
        state
    }

    #[test]
    fn off_receipt_requires_target_surface_order_and_success() {
        let mut state = PauseProgress::new(stamp(4, 8));
        state.off_after = Some(10);
        let wrong = SurfaceStamp {
            surface_id: 9,
            ..stamp(4, 8)
        };
        for (surface, present, query, ordinal) in [
            (stamp(4, 8), true, true, 10),
            (wrong, true, true, 11),
            (stamp(4, 8), false, true, 12),
            (stamp(4, 8), true, false, 13),
        ] {
            state.observe(surface, present, query, ordinal);
            assert!(!state.ready);
        }
        state.observe(stamp(4, 8), true, true, 14);
        assert!(state.ready);
        assert!(!state.complete);
    }

    #[test]
    fn old_inflight_present_cannot_finish_an_armed_change() {
        let mut state = ready();
        state.mark_issued(9).unwrap();
        state.arm(9, 1280, 720).unwrap();
        state.observe(stamp(4, 8), true, true, 100);
        assert!(!state.complete);
        // A candidate/present alone does not prove configure succeeded.
        state.observe(stamp(5, 9), true, true, 101);
        assert!(!state.complete);
        state.configured(stamp(5, 9));
        state.observe(stamp(5, 9), true, true, 102);
        assert!(state.complete);
    }

    #[test]
    fn configuration_and_present_must_match_epoch_extent_and_surface() {
        let mut state = ready();
        state.arm(9, 1280, 720).unwrap();
        for candidate in [
            SurfaceStamp {
                surface_id: 99,
                ..stamp(5, 9)
            },
            SurfaceStamp {
                width: 1920,
                ..stamp(5, 9)
            },
            stamp(5, 8),
            stamp(4, 9),
        ] {
            state.configured(candidate);
            state.observe(candidate, true, true, 20);
            assert!(!state.complete);
        }
        state.configured(stamp(5, 9));
        state.observe(stamp(5, 9), false, true, 21);
        state.observe(stamp(5, 9), true, false, 22);
        assert!(!state.complete);
        state.observe(stamp(6, 9), true, true, 23);
        assert!(state.complete);
    }

    #[test]
    fn configure_before_native_application_arm_is_not_a_receipt() {
        let mut state = ready();
        state.mark_issued(9).unwrap();
        state.configured(stamp(5, 9));
        state.arm(9, 1280, 720).unwrap();
        state.observe(stamp(5, 9), true, true, 20);
        assert!(!state.complete);
        state.configured(stamp(6, 9));
        state.observe(stamp(6, 9), true, true, 21);
        assert!(state.complete);
    }

    #[test]
    fn replacement_is_bounded_and_rejects_previous_epoch_receipts() {
        let mut state = ready();
        state.arm(9, 1280, 720).unwrap();
        state.configured(stamp(5, 9));
        state.mark_issued(10).unwrap();
        state.arm(10, 1280, 720).unwrap();
        assert!(state.arm(9, 1280, 720).is_err());
        state.configured(stamp(5, 9));
        state.observe(stamp(5, 9), true, true, 30);
        assert!(!state.complete);
        state.configured(stamp(6, 10));
        state.observe(stamp(6, 10), true, true, 31);
        assert!(state.complete);
    }

    #[test]
    fn drop_cancels_before_issue_but_keeps_finishing_state_after_issue() {
        for phase in 0..=2 {
            let state = Arc::new(Mutex::new(ready()));
            let token = PresentationPause(Arc::clone(&state));
            let clone = token.clone();
            let issued = phase != 0;
            if issued {
                token.mark_native_mutation_issued(9).unwrap();
            }
            if phase == 2 {
                token.arm_after_native_applied(9, 1280, 720).unwrap();
            }
            drop(token);
            assert!(!state.lock().unwrap().complete);
            drop(clone);
            let mut progress = state.lock().unwrap();
            assert_eq!(progress.complete, !issued);
            assert_eq!(progress.leases, 0);
            if issued {
                progress.arm(9, 1280, 720).unwrap();
                progress.configured(stamp(5, 9));
                progress.observe(stamp(5, 9), true, true, 40);
                assert!(progress.complete);
            }
        }
    }

    #[test]
    fn minimized_and_failed_requests_never_fake_completion() {
        let mut state = ready();
        state.mark_issued(9).unwrap();
        assert!(state.arm(9, 0, 720).is_err());
        state.observe(stamp(5, 9), true, true, 1000);
        assert!(!state.complete);
        state.failed = true;
        state.configured(stamp(5, 9));
        state.observe(stamp(5, 9), true, true, 1001);
        assert!(!state.complete);
    }
}

/// Process-global Streamline controller.
pub struct Streamline {
    core: CoreExports,
    features: FeatureExports,
    capabilities: FrameGenCapabilities,
    active_frame_token: *mut BaseStructure,
    last_constants_frame: Option<u32>,
    last_mode: u32,
    last_present_stamp: Option<SurfaceStamp>,
    last_ordered_off_stamp: Option<SurfaceStamp>,
    has_enabled_dlssg: bool,
    surface_pause: Option<Arc<Mutex<PauseProgress>>>,
    last_num_frames: u32,
    last_dlssg_opts: DlssGOptions,
    resize_cooldown_frames: u32,
    last_recorded_frame: Option<u32>,
    last_recorded_state: FrameGenState,
    is_dx12: bool,
    installed_swapchain_hooks: bool,
    native_shutdown: bool,
    // SL frame tagging shallow-copies Resource.next. Keep every published range
    // alive through deferred present and slShutdown, including runtime retirement.
    resource_metadata: ResourceMetadata,
    diagnostics: PresentDiagnostics,
    // Keep wide-string paths / project id alive for slInit lifetime.
    _plugin_dir_wide: Vec<u16>,
    _project_id_c: CString,
}

// SAFETY: Access to the global Streamline controller is serialized behind a
// `Mutex`; all function pointers are immutable process-wide DLL exports.
unsafe impl Send for Streamline {}
unsafe impl Sync for Streamline {}

static GLOBAL_STREAMLINE: OnceLock<Mutex<Option<Streamline>>> = OnceLock::new();

fn global_slot() -> &'static Mutex<Option<Streamline>> {
    GLOBAL_STREAMLINE.get_or_init(|| Mutex::new(None))
}

fn candidate_interposer_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        out.push(parent.join("sl.interposer.dll"));
    }
    if let Some(root) = std::env::var_os("BARD_NATIVE_LIBS") {
        out.push(
            PathBuf::from(root)
                .join("x86_64-pc-windows-msvc")
                .join("streamline")
                .join("sl.interposer.dll"),
        );
    }
    out.push(PathBuf::from("sl.interposer.dll"));
    out
}

fn plugin_and_model_paths(
    plugin_dir: &std::path::Path,
    executable_dir: Option<&std::path::Path>,
    native_root: Option<&std::path::Path>,
) -> Vec<PathBuf> {
    let mut paths = vec![plugin_dir.to_path_buf()];
    let mut add = |path: PathBuf| {
        if !paths.contains(&path) {
            paths.push(path);
        }
    };
    if let Some(parent) = plugin_dir.parent() {
        add(parent.join("dlss"));
    }
    if let Some(root) = native_root {
        add(root.join("x86_64-pc-windows-msvc").join("dlss"));
    }
    if let Some(directory) = executable_dir {
        add(directory.to_path_buf());
    }
    paths
}

fn to_wide_nul(path: &std::path::Path) -> Vec<u16> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }
}

macro_rules! load_sym {
    ($lib:expr, $name:literal, $ty:ty) => {{
        let sym: libloading::Symbol<$ty> = unsafe { $lib.get($name.as_bytes()) }
            .map_err(|e| Error::Symbol($name, e.to_string()))?;
        *sym
    }};
}

fn load_core() -> Result<(CoreExports, PathBuf), Error> {
    if !cfg!(windows) {
        return Err(Error::NotFound("Streamline is Windows-only".into()));
    }
    let mut attempted = Vec::new();
    for path in candidate_interposer_paths() {
        let library = match unsafe { libloading::Library::new(&path) } {
            Ok(lib) => lib,
            Err(err) => {
                attempted.push(format!("{} ({err})", path.display()));
                continue;
            }
        };
        let plugin_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|e| e.parent().map(PathBuf::from))
            })
            .unwrap_or_else(|| PathBuf::from("."));

        let core = CoreExports {
            sl_init: load_sym!(library, "slInit\0", FnSlInit),
            sl_shutdown: load_sym!(library, "slShutdown\0", FnSlShutdown),
            sl_is_feature_supported: load_sym!(
                library,
                "slIsFeatureSupported\0",
                FnSlIsFeatureSupported
            ),
            sl_get_feature_function: load_sym!(
                library,
                "slGetFeatureFunction\0",
                FnSlGetFeatureFunction
            ),
            sl_get_new_frame_token: load_sym!(
                library,
                "slGetNewFrameToken\0",
                FnSlGetNewFrameToken
            ),
            sl_set_constants: load_sym!(library, "slSetConstants\0", FnSlSetConstants),
            sl_set_tag_for_frame: load_sym!(library, "slSetTagForFrame\0", FnSlSetTagForFrame),
            sl_evaluate_feature: load_sym!(library, "slEvaluateFeature\0", FnSlEvaluateFeature),
            #[cfg(windows)]
            sl_set_d3d_device: load_sym!(library, "slSetD3DDevice\0", FnSlSetD3DDevice),
            sl_set_vulkan_info: load_sym!(library, "slSetVulkanInfo\0", FnSlSetVulkanInfo),
            #[cfg(windows)]
            sl_upgrade_interface: load_sym!(library, "slUpgradeInterface\0", FnSlUpgradeInterface),
            _library: library,
        };
        tracing::info!(
            path = %path.display(),
            plugin_dir = %plugin_dir.display(),
            "NVIDIA Streamline interposer loaded"
        );
        return Ok((core, plugin_dir));
    }
    Err(Error::NotFound(attempted.join("; ")))
}

unsafe fn query_feature_fn<T: Copy>(
    core: &CoreExports,
    feature: Feature,
    name: &'static [u8],
) -> Option<T> {
    let mut ptr_out: *mut c_void = ptr::null_mut();
    let res =
        unsafe { (core.sl_get_feature_function)(feature, name.as_ptr().cast(), &mut ptr_out) };
    if res == SL_RESULT_OK && !ptr_out.is_null() {
        Some(unsafe { std::mem::transmute_copy(&ptr_out) })
    } else {
        None
    }
}

/// A `slInit` performed before the wgpu instance existed, kept alive so
/// [`attach_device`] can attach the device it eventually produced.
struct PreInit {
    core: CoreExports,
    plugin_dir_wide: Vec<u16>,
    project_id_c: CString,
    is_dx12: bool,
    dlss_g_enabled: bool,
}

static PRE_INIT: OnceLock<Mutex<Option<PreInit>>> = OnceLock::new();

/// Whether [`preinit`] routed Vulkan entry points through the interposer, so its
/// swapchain proxies can own present for DLSS-G.
static VULKAN_INTERPOSED: AtomicBool = AtomicBool::new(false);

/// Startup policy: whether Streamline is permitted to load `kFeatureDLSS_G` and
/// install swapchain proxies/hooks. Defaults to `true` for backwards compatibility.
static DLSS_G_ENABLED: AtomicBool = AtomicBool::new(true);

/// Configure whether Streamline may load `kFeatureDLSS_G` and install swapchain
/// interception when initializing. Must be called before [`preinit`] / [`attach_device`].
pub fn set_dlss_g_enabled(enabled: bool) {
    DLSS_G_ENABLED.store(enabled, Ordering::Release);
}

/// Monotonic count of frames actually presented to the swapchain as reported by
/// `slDLSSGGetState` on the present thread (`on_present_end`).
static PRESENTED_FRAMES: AtomicU64 = AtomicU64::new(0);
fn reported_presented_frames(result: SlResult, native_count: u32) -> u64 {
    if result == SL_RESULT_OK {
        u64::from(native_count)
    } else {
        0
    }
}

/// Total frames actually presented to the swapchain (real + generated) since
/// startup, measured synchronously on the present thread via `slDLSSGGetState`.
pub fn total_presented_frames() -> u64 {
    PRESENTED_FRAMES.load(Ordering::Relaxed)
}

fn pre_init_slot() -> &'static Mutex<Option<PreInit>> {
    PRE_INIT.get_or_init(|| Mutex::new(None))
}

unsafe extern "C" fn sl_log_callback(log_type: u32, msg: *const c_char) {
    if msg.is_null() {
        return;
    }
    let text = unsafe { std::ffi::CStr::from_ptr(msg) }.to_string_lossy();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    match log_type {
        2 => tracing::error!("SL: {trimmed}"),
        1 => tracing::warn!("SL: {trimmed}"),
        _ => tracing::debug!("SL: {trimmed}"),
    }
}

fn preference_flags(is_dx12: bool, use_interposer_proxies: bool) -> u64 {
    let mut flags = PREF_DISABLE_CL_STATE_TRACKING | PREF_USE_FRAME_BASED_RESOURCE_TAGGING;
    if use_interposer_proxies {
        if is_dx12 {
            // sl_core_types.h: eUseDXGIFactoryProxy (avoid base factory vtable patching).
            flags |= 1 << 5;
        }
    } else {
        flags |= PREF_USE_MANUAL_HOOKING;
    }
    flags
}

const FEATURES_WITH_DLSS_G: [Feature; 5] = [
    K_FEATURE_DLSS,
    K_FEATURE_DLSS_RR,
    K_FEATURE_PCL,
    K_FEATURE_REFLEX,
    K_FEATURE_DLSS_G,
];

const FEATURES_WITHOUT_DLSS_G: [Feature; 4] = [
    K_FEATURE_DLSS,
    K_FEATURE_DLSS_RR,
    K_FEATURE_PCL,
    K_FEATURE_REFLEX,
];

fn features_to_load(enable_dlss_g: bool) -> &'static [Feature] {
    if enable_dlss_g {
        &FEATURES_WITH_DLSS_G
    } else {
        &FEATURES_WITHOUT_DLSS_G
    }
}

/// Issue `slInit` with the preferences this crate always uses.
fn init_streamline(
    core: &CoreExports,
    project_id_c: &CString,
    plugin_dir: &std::path::Path,
    is_dx12: bool,
    use_interposer_proxies: bool,
    enable_dlss_g: bool,
) -> Result<(), Error> {
    let executable = std::env::current_exe().ok();
    let native_root = std::env::var_os("BARD_NATIVE_LIBS").map(PathBuf::from);
    let paths = plugin_and_model_paths(
        plugin_dir,
        executable.as_deref().and_then(std::path::Path::parent),
        native_root.as_deref(),
    )
    .into_iter()
    .filter(|path| path.is_dir())
    .collect::<Vec<_>>();
    tracing::info!(?paths, "Streamline plugin and NGX model search paths");
    // Both levels of pointer storage remain alive throughout slInit, which
    // copies the search paths into its loader configuration.
    let wide_paths = paths
        .iter()
        .map(|path| to_wide_nul(path))
        .collect::<Vec<_>>();
    let plugin_paths = wide_paths
        .iter()
        .map(|path| path.as_ptr())
        .collect::<Vec<_>>();
    let features = features_to_load(enable_dlss_g);
    let engine_ver = c"1.0.0";
    let flags = preference_flags(is_dx12, use_interposer_proxies);
    let prefs = Preferences {
        base: BaseStructure::new(GUID_PREFERENCES, K_STRUCT_VERSION_1),
        show_console: false,
        log_level: if std::env::var("SOLARIK_SL_VERBOSE").as_deref() == Ok("1") {
            2
        } else {
            LOG_LEVEL_DEFAULT
        },
        paths_to_plugins: plugin_paths.as_ptr(),
        num_paths_to_plugins: plugin_paths.len() as u32,
        path_to_logs_and_data: ptr::null(),
        allocate_callback: ptr::null(),
        release_callback: ptr::null(),
        log_message_callback: sl_log_callback as *const c_void,
        flags,
        features_to_load: features.as_ptr(),
        num_features_to_load: features.len() as u32,
        application_id: 0,
        engine: ENGINE_TYPE_CUSTOM,
        engine_version: engine_ver.as_ptr(),
        project_id: project_id_c.as_ptr(),
        render_api: if is_dx12 {
            RENDER_API_D3D12
        } else {
            RENDER_API_VULKAN
        },
    };
    let init_res = unsafe { (core.sl_init)(&prefs, K_SDK_VERSION) };
    if init_res != SL_RESULT_OK {
        return Err(Error::Api("slInit", init_res));
    }
    Ok(())
}

/// Initialize Streamline **before** the wgpu instance is created.
///
/// DLSS-G on Vulkan has no "bring your own swapchain" entry point: Streamline's
/// `vkCreateSwapchainKHR`/`vkQueuePresentKHR` proxies must own present, and they
/// only accept a `VkSurfaceKHR` whose `VkInstance` the interposer itself created.
/// Routing `wgpu-hal`'s entry loading through `sl.interposer.dll` is what makes
/// that true, so this must run before `RenderPlugin` builds the device.
///
/// Returns `Err` (leaving the graph on the plain Vulkan loader) when the
/// interposer is absent, so this is safe on every machine.
pub fn preinit(project_id_uuid: &str, is_dx12: bool) -> Result<(), Error> {
    preinit_with_options(project_id_uuid, is_dx12, true)
}

/// Initialize Streamline before the wgpu instance is created, with explicit
/// control over whether `kFeatureDLSS_G` and swapchain interposer proxies are
/// enabled. When `enable_dlss_g` is `false`, `kFeatureDLSS_G` is excluded from
/// `features_to_load`, `use_interposer_proxies` is `false`, and Vulkan entry
/// loading stays on the plain system loader so another presenter (such as
/// FidelityFX FG) can own swapchain presentation exclusively.
pub fn preinit_with_options(
    project_id_uuid: &str,
    is_dx12: bool,
    enable_dlss_g: bool,
) -> Result<(), Error> {
    if is_ready() || pre_init_slot().lock().unwrap().is_some() {
        return Ok(());
    }
    DLSS_G_ENABLED.store(enable_dlss_g, Ordering::Release);
    let (core, plugin_dir) = load_core()?;
    let plugin_dir_wide = to_wide_nul(&plugin_dir);
    let project_id_c = CString::new(project_id_uuid).unwrap_or_else(|_| CString::new("").unwrap());
    let use_interposer_proxies = enable_dlss_g;
    init_streamline(
        &core,
        &project_id_c,
        &plugin_dir,
        is_dx12,
        use_interposer_proxies,
        enable_dlss_g,
    )?;
    if !is_dx12 && enable_dlss_g {
        let entry_library = plugin_dir.join("sl.interposer.dll");
        let entry_library = entry_library.to_string_lossy().into_owned();
        wgpu::hal::vulkan::set_vulkan_entry_library(Some(entry_library.as_str()));
        VULKAN_INTERPOSED.store(true, Ordering::Release);
        tracing::info!(entry_library, "Streamline owns Vulkan entry points");
    }
    *pre_init_slot().lock().unwrap() = Some(PreInit {
        core,
        plugin_dir_wide,
        project_id_c,
        is_dx12,
        dlss_g_enabled: enable_dlss_g,
    });
    Ok(())
}

/// Initialize Streamline early (before swapchain creation) and attach to the
/// live wgpu device (DX12 or Vulkan).
///
/// Safe on all hardware: on non-NVIDIA GPUs or when `sl.interposer.dll` is
/// absent, logs and returns `Err` without installing any swapchain hook.
pub fn attach_device(
    project_id_uuid: &str,
    device: &wgpu::Device,
) -> Result<FrameGenCapabilities, Error> {
    if let Ok(guard) = global_slot().lock()
        && let Some(sl) = guard.as_ref()
    {
        return Ok(sl.capabilities);
    }
    let is_dx12 = {
        #[cfg(windows)]
        {
            unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    };
    let is_vulkan = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }.is_some();
    if !is_dx12 && !is_vulkan {
        return Err(Error::UnsupportedBackend);
    }

    let pre = pre_init_slot().lock().unwrap().take();
    let (core, plugin_dir_wide, project_id_c, dlss_g_enabled) = match pre {
        Some(pre) if pre.is_dx12 == is_dx12 => (
            pre.core,
            pre.plugin_dir_wide,
            pre.project_id_c,
            pre.dlss_g_enabled,
        ),
        Some(pre) => {
            // Preserve the initialized loader for outer-owner abort cleanup. Never unload
            // a live preinit or initialize a second SDK for a mismatched backend.
            *pre_init_slot().lock().unwrap() = Some(pre);
            return Err(Error::UnsupportedBackend);
        }
        None => {
            let dlss_g_enabled = DLSS_G_ENABLED.load(Ordering::Acquire);
            let (core, plugin_dir) = load_core()?;
            let plugin_dir_wide = to_wide_nul(&plugin_dir);
            let project_id_c =
                CString::new(project_id_uuid).unwrap_or_else(|_| CString::new("").unwrap());
            init_streamline(
                &core,
                &project_id_c,
                &plugin_dir,
                is_dx12,
                is_dx12 && dlss_g_enabled,
                dlss_g_enabled,
            )?;
            (core, plugin_dir_wide, project_id_c, dlss_g_enabled)
        }
    };

    let mut adapter_info = AdapterInfo::default();
    if is_dx12 {
        #[cfg(windows)]
        {
            let hal = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }
                .ok_or(Error::UnsupportedBackend)?;
            let set_res = unsafe { (core.sl_set_d3d_device)(hal.raw_device_ptr()) };
            if set_res != SL_RESULT_OK {
                *pre_init_slot().lock().unwrap() = Some(PreInit {
                    core,
                    plugin_dir_wide,
                    project_id_c,
                    is_dx12,
                    dlss_g_enabled,
                });
                return Err(Error::Api("slSetD3DDevice", set_res));
            }
        }
    } else {
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or(Error::UnsupportedBackend)?;
        let phys = hal.raw_physical_device().as_raw() as usize as *mut c_void;
        adapter_info.vk_physical_device = phys;
        // Streamline `sl_helpers_vk.h`: "IMPORTANT: Only call slSetVulkanInfo if
        // NOT using vkCreateDevice and vkCreateInstance proxies provided by SL."
        // Calling it when interposed via `sl.interposer.dll` returns
        // `eErrorInvalidIntegration` (19).
        if !VULKAN_INTERPOSED.load(Ordering::Acquire) {
            let vk_info = VulkanInfo {
                device: hal.raw_device().handle().as_raw() as usize as *mut c_void,
                instance: hal.shared_instance().raw_instance().handle().as_raw() as usize
                    as *mut c_void,
                physical_device: phys,
                graphics_queue_family: hal.queue_family_index(),
                compute_queue_family: hal.queue_family_index(),
                ..VulkanInfo::default()
            };
            let set_res = unsafe { (core.sl_set_vulkan_info)(&vk_info) };
            if set_res != SL_RESULT_OK {
                *pre_init_slot().lock().unwrap() = Some(PreInit {
                    core,
                    plugin_dir_wide,
                    project_id_c,
                    is_dx12,
                    dlss_g_enabled,
                });
                return Err(Error::Api("slSetVulkanInfo", set_res));
            }
        }
    }

    let dlss_sr_res = unsafe { (core.sl_is_feature_supported)(K_FEATURE_DLSS, &adapter_info) };
    let dlss_rr_res = unsafe { (core.sl_is_feature_supported)(K_FEATURE_DLSS_RR, &adapter_info) };
    let dlss_g_res = if dlss_g_enabled {
        unsafe { (core.sl_is_feature_supported)(K_FEATURE_DLSS_G, &adapter_info) }
    } else {
        SL_RESULT_OK.wrapping_add(1)
    };
    let reflex_res = unsafe { (core.sl_is_feature_supported)(K_FEATURE_REFLEX, &adapter_info) };
    let features = FeatureExports {
        dlss_set_options: unsafe { query_feature_fn(&core, K_FEATURE_DLSS, b"slDLSSSetOptions\0") },
        dlss_get_optimal_settings: unsafe {
            query_feature_fn(&core, K_FEATURE_DLSS, b"slDLSSGetOptimalSettings\0")
        },
        dlss_d_set_options: unsafe {
            query_feature_fn(&core, K_FEATURE_DLSS_RR, b"slDLSSDSetOptions\0")
        },
        dlss_d_get_optimal_settings: unsafe {
            query_feature_fn(&core, K_FEATURE_DLSS_RR, b"slDLSSDGetOptimalSettings\0")
        },
        dlss_g_set_options: if dlss_g_enabled {
            unsafe { query_feature_fn(&core, K_FEATURE_DLSS_G, b"slDLSSGSetOptions\0") }
        } else {
            None
        },
        dlss_g_get_state: if dlss_g_enabled {
            unsafe { query_feature_fn(&core, K_FEATURE_DLSS_G, b"slDLSSGGetState\0") }
        } else {
            None
        },
        reflex_set_options: unsafe {
            query_feature_fn(&core, K_FEATURE_REFLEX, b"slReflexSetOptions\0")
        },
        reflex_sleep: unsafe { query_feature_fn(&core, K_FEATURE_REFLEX, b"slReflexSleep\0") },
        pcl_set_marker: unsafe { query_feature_fn(&core, K_FEATURE_PCL, b"slPCLSetMarker\0") },
    };

    let mut max_generated_frames = if dlss_g_enabled && dlss_g_res == SL_RESULT_OK {
        1
    } else {
        0
    };
    if dlss_g_enabled
        && dlss_g_res == SL_RESULT_OK
        && let Some(get_state) = features.dlss_g_get_state
    {
        let viewport = ViewportHandle::new(0);
        let mut state = DlssGState::default();
        if unsafe { get_state(&viewport, &mut state, ptr::null()) } == SL_RESULT_OK
            && state.num_frames_to_generate_max > 0
        {
            max_generated_frames = state.num_frames_to_generate_max;
        }
    }

    // With `preinit` the interposer created the VkInstance/VkDevice, so its
    // swapchain proxies accept the surface and can own present. Without it the
    // interposer never saw the surface and `vkCreateSwapchainKHR` fails, so
    // Vulkan stays present-markers-only.
    let interposed = is_dx12 || VULKAN_INTERPOSED.load(Ordering::Acquire);
    let dlss_g_supported = dlss_g_enabled
        && interposed
        && dlss_g_res == SL_RESULT_OK
        && features.dlss_g_set_options.is_some();
    let effective_max_gen = if dlss_g_supported {
        max_generated_frames
    } else {
        0
    };
    let capabilities = FrameGenCapabilities {
        dlss_sr_supported: dlss_sr_res == SL_RESULT_OK && features.dlss_set_options.is_some(),
        dlss_rr_supported: dlss_rr_res == SL_RESULT_OK && features.dlss_d_set_options.is_some(),
        dlss_g_supported,
        max_generated_frames: effective_max_gen,
        mfg_supported: effective_max_gen >= 2,
        reflex_supported: reflex_res == SL_RESULT_OK && features.reflex_set_options.is_some(),
    };

    tracing::info!(
        is_dx12,
        dlss_sr = capabilities.dlss_sr_supported,
        dlss_rr = capabilities.dlss_rr_supported,
        dlss_g = capabilities.dlss_g_supported,
        max_generated_frames = capabilities.max_generated_frames,
        mfg = capabilities.mfg_supported,
        reflex = capabilities.reflex_supported,
        "NVIDIA Streamline initialized"
    );

    let sl = Streamline {
        core,
        features,
        capabilities,
        active_frame_token: ptr::null_mut(),
        last_constants_frame: None,
        last_mode: DLSSG_MODE_OFF,
        last_present_stamp: None,
        last_ordered_off_stamp: None,
        has_enabled_dlssg: false,
        surface_pause: None,
        last_num_frames: 0,
        last_dlssg_opts: DlssGOptions::default(),
        resize_cooldown_frames: 0,
        last_recorded_frame: None,
        last_recorded_state: FrameGenState::default(),
        is_dx12,
        installed_swapchain_hooks: false,
        native_shutdown: false,
        resource_metadata: ResourceMetadata::default(),
        diagnostics: PresentDiagnostics::default(),
        _plugin_dir_wide: plugin_dir_wide,
        _project_id_c: project_id_c,
    };
    *global_slot().lock().unwrap() = Some(sl);

    if capabilities.dlss_g_supported {
        install_swapchain_interception();
    }

    Ok(capabilities)
}

/// Clear `wgpu-hal` swapchain hooks and call `slShutdown()` cleanly before the
/// GPU device is destroyed at application exit.
pub fn shutdown() {
    drop(shutdown_retaining_runtime());
}

/// Native shutdown with only the core interposer DLL owner retained. Feature/model DLLs
/// remain SDK-owned and detach during shutdown while the GPU keepalive is alive. Drop the
/// returned value after all queue/device/swapchain objects have been destroyed.
pub fn shutdown_retaining_runtime() -> Option<RetiredRuntime> {
    let runtime = global_slot().lock().unwrap().take();
    let pre = pre_init_slot().lock().unwrap().take();
    if let Some(sl) = runtime.as_ref()
        && sl.installed_swapchain_hooks
    {
        if sl.is_dx12 {
            #[cfg(windows)]
            wgpu::hal::dx12::set_swapchain_hooks(wgpu::hal::dx12::Dx12SwapchainHooks::default());
        } else {
            wgpu::hal::vulkan::set_swapchain_interceptor(
                wgpu::hal::vulkan::VulkanSwapchainInterceptor::default(),
            );
        }
    }
    if runtime.is_none() && pre.is_none() {
        return None;
    }
    let runtime = runtime.map(|mut runtime| {
        runtime.shutdown_native();
        runtime
    });
    if let Some(pre) = &pre {
        tracing::info!("Streamline pre-init shutdown begin");
        let result = unsafe { (pre.core.sl_shutdown)() };
        tracing::info!(result, "Streamline pre-init shutdown end");
    }
    Some(RetiredRuntime { runtime, pre })
}

/// Retains the core interposer after SDK shutdown until final device/queue teardown.
/// No extra references delay SDK-owned feature/model DLL detach past GPU destruction.
pub struct RetiredRuntime {
    runtime: Option<Streamline>,
    pre: Option<PreInit>,
}
impl Drop for RetiredRuntime {
    fn drop(&mut self) {
        drop(self.runtime.take());
        drop(self.pre.take());
    }
}

/// Install the `wgpu-hal` swapchain/present interception hooks for the active
/// backend (DX12 `slUpgradeInterface` or Vulkan `sl.interposer.dll` proxy fns).
pub fn install_swapchain_interception() {
    let mut guard = global_slot().lock().unwrap();
    let Some(sl) = guard.as_mut() else {
        return;
    };
    if sl.is_dx12 {
        #[cfg(windows)]
        {
            wgpu::hal::dx12::merge_swapchain_hooks(wgpu::hal::dx12::Dx12SwapchainHooks {
                create_swapchain_for_hwnd: Some(dx12_create_swapchain_for_hwnd),
                on_swapchain_created: Some(dx12_upgrade_swapchain),
                on_before_swapchain_change: Some(on_before_swapchain_recreate),
                on_present_start: Some(on_present_start),
                on_surface_configured: Some(on_surface_configured),
                on_present_stamp: Some(on_present_stamp),
                ..Default::default()
            });
            sl.installed_swapchain_hooks = true;
            tracing::info!("Streamline DX12 swapchain/present interception hooks installed");
        }
    } else {
        // When `preinit` routed `ash::Entry` through `sl.interposer.dll`,
        // `ash::khr::swapchain::Device` already holds the interposer's own
        // `vkCreateSwapchainKHR`/`vkQueuePresentKHR` proxies — do not double-wrap.
        // Attach only the present markers (PCL/Reflex) and resize notification.
        wgpu::hal::vulkan::merge_swapchain_interceptor(
            wgpu::hal::vulkan::VulkanSwapchainInterceptor {
                on_present_start: Some(on_present_start),
                on_surface_configured: Some(on_surface_configured),
                on_present_stamp: Some(on_present_stamp),
                on_before_swapchain_recreate: Some(on_before_swapchain_recreate),
                ..Default::default()
            },
        );
        sl.installed_swapchain_hooks = true;
        tracing::info!("Streamline Vulkan present-marker hooks installed");
    }
}

#[cfg(windows)]
unsafe fn dx12_create_swapchain_for_hwnd(
    request: &wgpu::hal::dx12::Dx12SwapchainCreateRequest,
) -> Option<*mut c_void> {
    use windows::{
        Win32::{
            Foundation::HWND,
            Graphics::Dxgi::{DXGI_SWAP_CHAIN_DESC1, IDXGIFactory4, IDXGIOutput},
        },
        core::{IUnknown, Interface},
    };

    let guard = global_slot().lock().ok()?;
    let sl = guard.as_ref()?;
    // All request pointers borrow live wgpu-hal inputs for this callback only.
    // Clone the factory before upgrading a temporary slot, never wgpu's own
    // interface storage. The queue stays borrowed. In SDK 2.14.16 the factory
    // proxy constructor AddRefs the native factory; it does not consume this
    // clone. Keep that reference in RAII separately from the returned proxy.
    let factory = unsafe { IDXGIFactory4::from_raw_borrowed(&request.factory) }?;
    let queue = unsafe { IUnknown::from_raw_borrowed(&request.queue) }?;
    if request.hwnd.is_null() || request.desc.is_null() {
        return None;
    }
    let native_factory = factory.clone();
    let mut factory_slot = native_factory.as_raw();
    let result = unsafe { (sl.core.sl_upgrade_interface)(&mut factory_slot) };
    if factory_slot.is_null() {
        return None;
    }
    let factory = if factory_slot == native_factory.as_raw() {
        // Disabled interposer or already-proxied input: no new reference.
        native_factory.clone()
    } else {
        // New proxy owns its own native reference; adopt its initial COM ref.
        unsafe { IDXGIFactory4::from_raw(factory_slot) }
    };
    if result != SL_RESULT_OK {
        tracing::warn!(
            result,
            "Streamline factory upgrade failed; falling back to DXGI"
        );
        return None;
    }
    // Calling the proxy (not the native factory) runs SL's before/after
    // CreateSwapChainForHwnd hooks with the original queue, HWND and descriptor.
    // No new device/queue is opened and no encoder raw pointer escapes.
    let created = unsafe {
        factory.CreateSwapChainForHwnd(
            queue,
            HWND(request.hwnd),
            request.desc.cast::<DXGI_SWAP_CHAIN_DESC1>(),
            None,
            None::<&IDXGIOutput>,
        )
    };
    match created {
        Ok(swapchain) => {
            tracing::info!("Streamline DX12 swapchain created through factory proxy");
            Some(swapchain.into_raw())
        }
        Err(error) => {
            tracing::warn!(%error, "Streamline DX12 proxy swapchain creation failed; falling back to DXGI");
            None
        }
    }
}

#[cfg(windows)]
unsafe fn dx12_upgrade_swapchain(raw_swapchain: *mut c_void) -> Option<*mut c_void> {
    let guard = global_slot().lock().ok()?;
    let sl = guard.as_ref()?;
    let mut ptr_slot = raw_swapchain;
    let res = unsafe { (sl.core.sl_upgrade_interface)(&mut ptr_slot) };
    if res == SL_RESULT_OK && !ptr_slot.is_null() {
        tracing::info!("Streamline upgraded IDXGISwapChain3 for DLSS-G frame generation");
        Some(ptr_slot)
    } else {
        tracing::warn!(
            res,
            "slUpgradeInterface for IDXGISwapChain3 returned non-OK"
        );
        None
    }
}

fn on_before_swapchain_recreate() {
    if let Ok(mut guard) = global_slot().lock()
        && let Some(sl) = guard.as_mut()
    {
        sl.diagnostics.reconfigured();
        tracing::info!(target: "solarik_fg_diagnostics", generation = sl.diagnostics.generation, ordinal = sl.diagnostics.ordinal, requested_mode = sl.diagnostics.requested_mode, last_successful_setter_mode = sl.last_mode, frame = ?sl.last_recorded_frame, constants_frame = ?sl.last_constants_frame, token = ?sl.active_frame_token, options = ?sl.last_dlssg_opts, thread = ?std::thread::current().id(), "Swapchain change notification; SDK effective mode is not exposed");
        if sl.last_mode != DLSSG_MODE_ON {
            return;
        }
        let Some(set_opts) = sl.features.dlss_g_set_options else {
            return;
        };
        let viewport = ViewportHandle::new(0);
        let off = DlssGOptions {
            mode: DLSSG_MODE_OFF,
            ..sl.last_dlssg_opts
        };
        let result = unsafe { set_opts(&viewport, &off) };
        tracing::info!(target: "solarik_fg_diagnostics", generation = sl.diagnostics.generation, ordinal = sl.diagnostics.ordinal, result, options = ?off, thread = ?std::thread::current().id(), "slDLSSGSetOptions pause request; not adoption acknowledgment");
        if result == SL_RESULT_OK {
            sl.last_mode = DLSSG_MODE_OFF;
            sl.last_num_frames = 0;
            sl.last_dlssg_opts = off;
            sl.resize_cooldown_frames = 30;
            tracing::info!("Streamline DLSS-G paused before swapchain change");
        } else {
            tracing::warn!(
                result,
                "Streamline DLSS-G pause before swapchain change failed"
            );
        }
    }
}

fn on_present_start() {
    if let Ok(guard) = global_slot().try_lock()
        && let Some(sl) = guard.as_ref()
        && !sl.active_frame_token.is_null()
        && let Some(set_marker) = sl.features.pcl_set_marker
    {
        unsafe {
            log_native_failure(
                "slPCLSetMarker PresentStart",
                set_marker(PCL_MARKER_PRESENT_START, sl.active_frame_token),
            );
        }
    }
}

fn on_surface_configured(stamp: SurfaceStamp) {
    if let Ok(guard) = global_slot().lock()
        && let Some(sl) = guard.as_ref()
        && let Some(request) = &sl.surface_pause
    {
        request.lock().unwrap().configured(stamp);
        tracing::info!(target: "solarik_fg_diagnostics", ?stamp, "Observed successful native surface configuration");
    }
}

fn on_present_stamp(stamp: SurfaceStamp, successful_present: bool) {
    if let Ok(mut guard) = global_slot().lock()
        && let Some(sl) = guard.as_mut()
    {
        if !sl.active_frame_token.is_null()
            && let Some(set_marker) = sl.features.pcl_set_marker
        {
            unsafe {
                log_native_failure(
                    "slPCLSetMarker PresentEnd",
                    set_marker(PCL_MARKER_PRESENT_END, sl.active_frame_token),
                );
            }
        }
        // Observe each completed Present. Sparse queries undercounted on the
        // deployed provider (it returned 2, not 16, across eight Presents).
        // Never extrapolate that value or replace a reported zero with a guess.
        let mut presented = 0u64;
        if let Some(get_state) = sl.features.dlss_g_get_state {
            let viewport = ViewportHandle::new(0);
            let mut raw_state = DlssGState::default();
            let query_start = std::time::Instant::now();
            // Options are only needed for optional VRAM-estimation queries.
            let res = unsafe { get_state(&viewport, &mut raw_state, ptr::null()) };
            if successful_present && res == SL_RESULT_OK {
                sl.last_present_stamp = Some(stamp);
                if sl.last_mode == DLSSG_MODE_OFF {
                    sl.last_ordered_off_stamp = Some(stamp);
                }
            }
            if let Some(request) = &sl.surface_pause {
                let mut state = request.lock().unwrap();
                let was_ready = state.ready;
                let was_complete = state.complete;
                state.observe(
                    stamp,
                    successful_present,
                    res == SL_RESULT_OK,
                    sl.diagnostics.ordinal + 1,
                );
                if state.ready != was_ready || state.complete != was_complete {
                    tracing::info!(target: "solarik_fg_diagnostics", ?stamp, ordinal = sl.diagnostics.ordinal + 1, successful_present, query_result = res, ready = state.ready, complete = state.complete, "Presentation pause stamped boundary observed; not an effective-mode query");
                }
            }
            tracing::debug!(target: "solarik_present_timing", elapsed_ms = query_start.elapsed().as_secs_f64() * 1000.0, "Streamline presentation state query");
            // Preserve public counting policy. OFF queries are diagnostic only.
            if sl.last_mode == DLSSG_MODE_ON {
                presented = reported_presented_frames(res, raw_state.num_frames_actually_presented);
            }
            if sl.diagnostics.observe((
                res,
                raw_state.status,
                raw_state.num_frames_actually_presented,
                raw_state.inputs_processing_completion_fence as usize,
                raw_state.last_present_inputs_processing_completion_fence_value,
            )) {
                tracing::info!(target: "solarik_fg_diagnostics",
                    generation = sl.diagnostics.generation, ordinal = sl.diagnostics.ordinal, since_change = sl.diagnostics.since_change,
                    requested_mode = sl.diagnostics.requested_mode, last_successful_setter_mode = sl.last_mode,
                    res, status = raw_state.status, presented = raw_state.num_frames_actually_presented,
                    min_dimension = raw_state.min_width_or_height, max_gen = raw_state.num_frames_to_generate_max,
                    vsync_supported = raw_state.b_is_vsync_support_available,
                    completion_fence = ?raw_state.inputs_processing_completion_fence,
                    completion_target = raw_state.last_present_inputs_processing_completion_fence_value,
                    frame = ?sl.last_recorded_frame, constants_frame = ?sl.last_constants_frame,
                    token = ?sl.active_frame_token, options = ?sl.last_dlssg_opts,
                    thread = ?std::thread::current().id(), "Native state after Present; no effective-mode field");
            }
        }
        PRESENTED_FRAMES.fetch_add(presented, Ordering::Relaxed);
    }
}

/// Whether Streamline pre-init routed Vulkan entry points through `sl.interposer.dll`.
pub fn is_vulkan_interposed() -> bool {
    VULKAN_INTERPOSED.load(Ordering::Acquire)
}

/// Query optimal render resolution `[width, height]` from Streamline DLSS SR or RR.
pub fn dlss_optimal_render_size(
    output_size: [u32; 2],
    mode: u32,
    ray_reconstruction: bool,
) -> Option<[u32; 2]> {
    let guard = global_slot().lock().ok()?;
    let sl = guard.as_ref()?;
    if ray_reconstruction {
        let get_opt = sl.features.dlss_d_get_optimal_settings?;
        let opts = DlssDOptions {
            mode,
            output_width: output_size[0],
            output_height: output_size[1],
            ..DlssDOptions::default()
        };
        let mut settings = DlssDOptimalSettings::default();
        if unsafe { get_opt(&opts, &mut settings) } == SL_RESULT_OK
            && settings.optimal_render_width > 0
            && settings.optimal_render_height > 0
        {
            return Some([
                settings.optimal_render_width,
                settings.optimal_render_height,
            ]);
        }
    } else {
        let get_opt = sl.features.dlss_get_optimal_settings?;
        let opts = DlssOptions {
            mode,
            output_width: output_size[0],
            output_height: output_size[1],
            ..DlssOptions::default()
        };
        let mut settings = DlssOptimalSettings::default();
        if unsafe { get_opt(&opts, &mut settings) } == SL_RESULT_OK
            && settings.optimal_render_width > 0
            && settings.optimal_render_height > 0
        {
            return Some([
                settings.optimal_render_width,
                settings.optimal_render_height,
            ]);
        }
    }
    None
}

/// Evaluate Streamline DLSS Super Resolution (`kFeatureDLSS`) on the active
/// command encoder. `barrier_encoder` must be submitted before the fresh,
/// otherwise unused `cmd_buffer` encoder, followed by all output consumers.
///
/// Before native commands execute, callers must provide GPU-valid input contents
/// and establish wgpu initialization accounting for every native-read input and
/// native-written output subresource. These state barriers do not initialize
/// memory, and native writes do not update wgpu's initialization tracker. Record
/// any required tracked initialization before evaluation in `barrier_encoder`
/// (or an earlier submitted encoder), never in the fresh native encoder. A later
/// tracked read can otherwise clear an output that wgpu still considers unused.
/// Renderable outputs can use an empty Load/Store render pass; storage-only
/// outputs require a suitable caller-owned tracked initialization path instead.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_dlss_sr(
    frame_index: u32,
    mode: u32,
    output_size: [u32; 2],
    render_size: [u32; 2],
    constants: &Constants,
    color_in: &wgpu::TextureView,
    color_out: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    motion_vectors: &wgpu::TextureView,
    exposure: Option<&wgpu::TextureView>,
    barrier_encoder: &mut wgpu::CommandEncoder,
    cmd_buffer: &mut wgpu::CommandEncoder,
) -> Result<(), Error> {
    let mut guard = global_slot().lock().unwrap();
    let sl = guard.as_mut().ok_or(Error::NotInitialized)?;
    let set_opts = sl
        .features
        .dlss_set_options
        .ok_or(Error::Api("slDLSSSetOptions", 16))?;

    let mut token: *mut BaseStructure = ptr::null_mut();
    let tok_res = unsafe { (sl.core.sl_get_new_frame_token)(&mut token, &frame_index) };
    if tok_res != SL_RESULT_OK || token.is_null() {
        return Err(Error::Api("slGetNewFrameToken", tok_res));
    }
    sl.active_frame_token = token;

    let viewport = ViewportHandle::new(0);
    if sl.last_constants_frame != Some(frame_index) {
        let constants_res = unsafe { (sl.core.sl_set_constants)(constants, token, &viewport) };
        if constants_res != SL_RESULT_OK {
            return Err(Error::Api("slSetConstants", constants_res));
        }
        sl.last_constants_frame = Some(frame_index);
    }

    let opts = DlssOptions {
        mode,
        output_width: output_size[0],
        output_height: output_size[1],
        // Match the Bevy caller's Automatic exposure mode when it supplies no
        // exposure texture; explicit exposure buffers disable auto exposure.
        use_auto_exposure: if exposure.is_none() {
            BOOLEAN_TRUE
        } else {
            BOOLEAN_FALSE
        },
        ..DlssOptions::default()
    };
    let opt_res = unsafe { set_opts(&viewport, &opts) };
    if opt_res != SL_RESULT_OK {
        return Err(Error::Api("slDLSSSetOptions", opt_res));
    }

    let mut in_res = make_sl_resource(sl, color_in, false);
    let mut out_res = make_sl_resource(sl, color_out, true);
    let mut depth_res = make_sl_resource(sl, depth, false);
    let mut mv_res = make_sl_resource(sl, motion_vectors, false);
    let mut exp_res = exposure.and_then(|v| make_sl_resource(sl, v, false));

    let render_ext = Extent {
        top: 0,
        left: 0,
        width: render_size[0],
        height: render_size[1],
    };
    let out_ext = Extent {
        top: 0,
        left: 0,
        width: output_size[0],
        height: output_size[1],
    };
    let mut tags = Vec::with_capacity(5);
    let mut push_tag = |res: Option<&mut Resource>, kind: BufferType, ext: Extent| {
        if let Some(r) = res {
            let mut t = ResourceTag::new(r, kind, ext);
            t.lifecycle = RESOURCE_LIFECYCLE_VALID_UNTIL_EVALUATE;
            tags.push(t);
        }
    };
    push_tag(
        in_res.as_mut(),
        K_BUFFER_TYPE_SCALING_INPUT_COLOR,
        render_ext,
    );
    push_tag(
        out_res.as_mut(),
        K_BUFFER_TYPE_SCALING_OUTPUT_COLOR,
        out_ext,
    );
    push_tag(depth_res.as_mut(), K_BUFFER_TYPE_DEPTH, render_ext);
    push_tag(mv_res.as_mut(), K_BUFFER_TYPE_MOTION_VECTORS, render_ext);
    push_tag(
        exp_res.as_mut(),
        K_BUFFER_TYPE_EXPOSURE,
        Extent {
            top: 0,
            left: 0,
            width: 1,
            height: 1,
        },
    );

    transition_dlss_resources(
        barrier_encoder,
        [Some(color_in), Some(depth), Some(motion_vectors), exposure],
        color_out,
    );
    with_dlss_command_buffer(sl.is_dx12, cmd_buffer, |raw_cmd| {
        let tag_res = unsafe {
            (sl.core.sl_set_tag_for_frame)(
                token,
                &viewport,
                tags.as_ptr(),
                tags.len() as u32,
                raw_cmd,
            )
        };
        if tag_res != SL_RESULT_OK {
            return Err(Error::Api("slSetTagForFrame", tag_res));
        }
        let inputs: [*const BaseStructure; 1] = [(&viewport as *const ViewportHandle).cast()];
        let eval_res = unsafe {
            (sl.core.sl_evaluate_feature)(
                K_FEATURE_DLSS,
                token,
                inputs.as_ptr(),
                inputs.len() as u32,
                raw_cmd,
            )
        };
        if eval_res != SL_RESULT_OK {
            return Err(Error::Api("slEvaluateFeature(DLSS)", eval_res));
        }
        Ok(())
    })
}

/// Evaluate Streamline DLSS Ray Reconstruction (`kFeatureDLSS_RR`) on the active
/// command encoder. `barrier_encoder` must be submitted before the fresh,
/// otherwise unused `cmd_buffer` encoder, followed by all output consumers.
///
/// Before native commands execute, callers must provide GPU-valid input contents
/// and establish wgpu initialization accounting for every native-read input and
/// native-written output subresource. These state barriers do not initialize
/// memory, and native writes do not update wgpu's initialization tracker. Record
/// any required tracked initialization before evaluation in `barrier_encoder`
/// (or an earlier submitted encoder), never in the fresh native encoder. A later
/// tracked read can otherwise clear an output that wgpu still considers unused.
/// Renderable outputs can use an empty Load/Store render pass; storage-only
/// outputs require a suitable caller-owned tracked initialization path instead.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_dlss_rr(
    frame_index: u32,
    mode: u32,
    output_size: [u32; 2],
    render_size: [u32; 2],
    constants: &Constants,
    world_to_camera_view: [[f32; 4]; 4],
    camera_view_to_world: [[f32; 4]; 4],
    color_in: &wgpu::TextureView,
    color_out: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    motion_vectors: &wgpu::TextureView,
    diffuse_albedo: &wgpu::TextureView,
    specular_albedo: &wgpu::TextureView,
    normal_roughness: &wgpu::TextureView,
    specular_motion_vectors: Option<&wgpu::TextureView>,
    exposure: Option<&wgpu::TextureView>,
    barrier_encoder: &mut wgpu::CommandEncoder,
    cmd_buffer: &mut wgpu::CommandEncoder,
) -> Result<(), Error> {
    let mut guard = global_slot().lock().unwrap();
    let sl = guard.as_mut().ok_or(Error::NotInitialized)?;
    let set_opts = sl
        .features
        .dlss_d_set_options
        .ok_or(Error::Api("slDLSSDSetOptions", 16))?;

    let mut token: *mut BaseStructure = ptr::null_mut();
    let tok_res = unsafe { (sl.core.sl_get_new_frame_token)(&mut token, &frame_index) };
    if tok_res != SL_RESULT_OK || token.is_null() {
        return Err(Error::Api("slGetNewFrameToken", tok_res));
    }
    sl.active_frame_token = token;

    let viewport = ViewportHandle::new(0);
    if sl.last_constants_frame != Some(frame_index) {
        let constants_res = unsafe { (sl.core.sl_set_constants)(constants, token, &viewport) };
        if constants_res != SL_RESULT_OK {
            return Err(Error::Api("slSetConstants", constants_res));
        }
        sl.last_constants_frame = Some(frame_index);
    }

    let opts = DlssDOptions {
        mode,
        output_width: output_size[0],
        output_height: output_size[1],
        normal_roughness_mode: DLSSD_NORMAL_ROUGHNESS_PACKED,
        world_to_camera_view,
        camera_view_to_world,
        ..DlssDOptions::default()
    };
    let opt_res = unsafe { set_opts(&viewport, &opts) };
    if opt_res != SL_RESULT_OK {
        return Err(Error::Api("slDLSSDSetOptions", opt_res));
    }

    let mut in_res = make_sl_resource(sl, color_in, false);
    let mut out_res = make_sl_resource(sl, color_out, true);
    let mut depth_res = make_sl_resource(sl, depth, false);
    let mut mv_res = make_sl_resource(sl, motion_vectors, false);
    let mut diff_res = make_sl_resource(sl, diffuse_albedo, false);
    let mut spec_res = make_sl_resource(sl, specular_albedo, false);
    let mut norm_res = make_sl_resource(sl, normal_roughness, false);
    let mut spec_mv_res = specular_motion_vectors.and_then(|v| make_sl_resource(sl, v, false));
    let mut exp_res = exposure.and_then(|v| make_sl_resource(sl, v, false));

    let render_ext = Extent {
        top: 0,
        left: 0,
        width: render_size[0],
        height: render_size[1],
    };
    let out_ext = Extent {
        top: 0,
        left: 0,
        width: output_size[0],
        height: output_size[1],
    };
    let mut tags = Vec::with_capacity(9);
    let mut push_tag = |res: Option<&mut Resource>, kind: BufferType, ext: Extent| {
        if let Some(r) = res {
            let mut t = ResourceTag::new(r, kind, ext);
            t.lifecycle = RESOURCE_LIFECYCLE_VALID_UNTIL_EVALUATE;
            tags.push(t);
        }
    };
    push_tag(
        in_res.as_mut(),
        K_BUFFER_TYPE_SCALING_INPUT_COLOR,
        render_ext,
    );
    push_tag(
        out_res.as_mut(),
        K_BUFFER_TYPE_SCALING_OUTPUT_COLOR,
        out_ext,
    );
    push_tag(depth_res.as_mut(), K_BUFFER_TYPE_DEPTH, render_ext);
    push_tag(mv_res.as_mut(), K_BUFFER_TYPE_MOTION_VECTORS, render_ext);
    push_tag(diff_res.as_mut(), K_BUFFER_TYPE_ALBEDO, render_ext);
    push_tag(spec_res.as_mut(), K_BUFFER_TYPE_SPECULAR_ALBEDO, render_ext);
    push_tag(
        norm_res.as_mut(),
        K_BUFFER_TYPE_NORMAL_ROUGHNESS,
        render_ext,
    );
    push_tag(
        spec_mv_res.as_mut(),
        K_BUFFER_TYPE_SPECULAR_MOTION_VECTORS,
        render_ext,
    );
    push_tag(
        exp_res.as_mut(),
        K_BUFFER_TYPE_EXPOSURE,
        Extent {
            top: 0,
            left: 0,
            width: 1,
            height: 1,
        },
    );

    transition_dlss_resources(
        barrier_encoder,
        [
            Some(color_in),
            Some(depth),
            Some(motion_vectors),
            Some(diffuse_albedo),
            Some(specular_albedo),
            Some(normal_roughness),
            specular_motion_vectors,
            exposure,
        ],
        color_out,
    );
    with_dlss_command_buffer(sl.is_dx12, cmd_buffer, |raw_cmd| {
        let tag_res = unsafe {
            (sl.core.sl_set_tag_for_frame)(
                token,
                &viewport,
                tags.as_ptr(),
                tags.len() as u32,
                raw_cmd,
            )
        };
        if tag_res != SL_RESULT_OK {
            return Err(Error::Api("slSetTagForFrame", tag_res));
        }
        let inputs: [*const BaseStructure; 1] = [(&viewport as *const ViewportHandle).cast()];
        let eval_res = unsafe {
            (sl.core.sl_evaluate_feature)(
                K_FEATURE_DLSS_RR,
                token,
                inputs.as_ptr(),
                inputs.len() as u32,
                raw_cmd,
            )
        };
        if eval_res != SL_RESULT_OK {
            return Err(Error::Api("slEvaluateFeature(DLSS_RR)", eval_res));
        }
        Ok(())
    })
}

// Barriers belong on the tracked encoder; as_hal_mut requires a fresh encoder.
fn transition_dlss_resources<'a>(
    encoder: &mut wgpu::CommandEncoder,
    inputs: impl IntoIterator<Item = Option<&'a wgpu::TextureView>>,
    output: &wgpu::TextureView,
) {
    let inputs = inputs
        .into_iter()
        .flatten()
        .map(|view| wgpu::TextureTransition {
            texture: view.texture(),
            selector: None,
            state: wgpu::TextureUses::RESOURCE,
        });
    let output = wgpu::TextureTransition {
        texture: output.texture(),
        selector: None,
        state: wgpu::TextureUses::STORAGE_READ_WRITE,
    };
    encoder.transition_resources(std::iter::empty(), inputs.chain(std::iter::once(output)));
}

// Native recording must finish inside as_hal_mut; never return its raw pointer.
fn with_dlss_command_buffer(
    is_dx12: bool,
    encoder: &mut wgpu::CommandEncoder,
    evaluate: impl FnOnce(*mut c_void) -> Result<(), Error>,
) -> Result<(), Error> {
    if is_dx12 {
        #[cfg(windows)]
        unsafe {
            encoder.as_hal_mut::<wgpu::hal::api::Dx12, _, _>(|hal| {
                let raw = hal
                    .and_then(|h| h.raw_command_list_ptr())
                    .filter(|raw| !raw.is_null())
                    .ok_or(Error::UnsupportedBackend)?;
                evaluate(raw)
            })
        }
        #[cfg(not(windows))]
        {
            let _ = (encoder, evaluate);
            Err(Error::UnsupportedBackend)
        }
    } else {
        unsafe {
            encoder.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|hal| {
                let hal = hal.ok_or(Error::UnsupportedBackend)?;
                evaluate(hal.raw_handle().as_raw() as usize as *mut c_void)
            })
        }
    }
}

/// Whether Streamline is initialized and ready.
pub fn is_ready() -> bool {
    global_slot().lock().ok().is_some_and(|g| g.is_some())
}

/// Last probed Streamline capabilities (all-false if Streamline did not initialize).
pub fn current_capabilities() -> FrameGenCapabilities {
    global_slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.capabilities))
        .unwrap_or_default()
}

/// Per-frame Streamline update: Reflex pacing + constants + resource tags +
/// DLSS-G options/state.
///
/// `num_frames_to_generate`: `0` = Off, `1` = 2x (RTX 4090+), `2` = 3x, `3` = 4x (RTX 50-series).
/// When enabled, `cmd_buffer` must be a fresh encoder, submitted after
/// `barrier_encoder`. No native encoder is required when disabling DLSS-G.
#[allow(clippy::too_many_arguments)]
pub fn record_frame(
    frame_index: u32,
    num_frames_to_generate: u32,
    constants: &Constants,
    depth: &wgpu::TextureView,
    motion_vectors: &wgpu::TextureView,
    hudless_color: &wgpu::TextureView,
    render_size: [u32; 2],
    display_size: [u32; 2],
    barrier_encoder: &mut wgpu::CommandEncoder,
    cmd_buffer: Option<&mut wgpu::CommandEncoder>,
) -> Result<FrameGenState, Error> {
    let mut guard = global_slot().lock().unwrap();
    let sl = guard.as_mut().ok_or(Error::NotInitialized)?;
    sl.diagnostics.requested_mode = if num_frames_to_generate > 0 {
        DLSSG_MODE_ON
    } else {
        DLSSG_MODE_OFF
    };

    // The caller runs per extracted view, but this controller currently owns
    // one viewport (0). Avoid overwriting its tags/options for the same token.
    // Multi-viewport presentation requires explicit viewport identities.
    if sl.last_recorded_frame == Some(frame_index) {
        return Ok(sl.last_recorded_state);
    }
    sl.last_recorded_frame = Some(frame_index);

    if let Some(request) = &sl.surface_pause {
        let mut pause = request.lock().unwrap();
        if !pause.complete {
            if pause.off_after.is_none() && !pause.failed {
                if let Some(set_options) = sl.features.dlss_g_set_options {
                    let options = DlssGOptions {
                        mode: DLSSG_MODE_OFF,
                        ..sl.last_dlssg_opts
                    };
                    let result = unsafe { set_options(&ViewportHandle::new(0), &options) };
                    tracing::info!(target: "solarik_fg_diagnostics", frame_index, ordinal = sl.diagnostics.ordinal, result, thread = ?std::thread::current().id(), "Ordered render path requested OFF for application window transaction");
                    if result == SL_RESULT_OK {
                        sl.last_mode = DLSSG_MODE_OFF;
                        sl.last_num_frames = 0;
                        sl.last_dlssg_opts = options;
                        pause.off_after = Some(sl.diagnostics.ordinal);
                    } else {
                        pause.failed = true;
                    }
                } else {
                    pause.failed = true;
                }
            }
            let state = FrameGenState {
                max_generated_frames: sl.capabilities.max_generated_frames,
                ..Default::default()
            };
            sl.last_recorded_state = state;
            return Ok(state);
        }
    }

    if sl.resize_cooldown_frames > 0 {
        sl.resize_cooldown_frames -= 1;
        let state = FrameGenState {
            max_generated_frames: sl.capabilities.max_generated_frames,
            num_frames_actually_presented: 1,
            ..Default::default()
        };
        sl.last_recorded_state = state;
        return Ok(state);
    }

    let enabled = num_frames_to_generate > 0 && sl.capabilities.dlss_g_supported;
    if !enabled && sl.last_mode == DLSSG_MODE_OFF {
        let state = FrameGenState {
            max_generated_frames: sl.capabilities.max_generated_frames,
            ..Default::default()
        };
        sl.last_recorded_state = state;
        return Ok(state);
    }

    let mut token: *mut BaseStructure = ptr::null_mut();
    let tok_res = unsafe { (sl.core.sl_get_new_frame_token)(&mut token, &frame_index) };
    if tok_res != SL_RESULT_OK || token.is_null() {
        return Err(Error::Api("slGetNewFrameToken", tok_res));
    }
    sl.active_frame_token = token;
    let clamped_gen = if enabled {
        num_frames_to_generate.min(sl.capabilities.max_generated_frames.max(1))
    } else {
        0
    };

    // DLSS-G requires Reflex active when Frame Generation is enabled.
    if let Some(set_reflex) = sl.features.reflex_set_options {
        let reflex_opts = ReflexOptions {
            mode: if enabled {
                REFLEX_MODE_LOW_LATENCY
            } else {
                REFLEX_MODE_OFF
            },
            ..ReflexOptions::default()
        };
        unsafe {
            log_native_failure("slReflexSetOptions", set_reflex(&reflex_opts));
        }
    }
    // Frame pacing belongs at the simulation boundary, not inside a per-view
    // render node holding the runtime mutex. Do not sleep in this callback.
    let _ = (&hudless_color, &sl.features.reflex_sleep);
    if let Some(set_marker) = sl.features.pcl_set_marker {
        unsafe {
            log_native_failure(
                "slPCLSetMarker SimulationStart",
                set_marker(PCL_MARKER_SIMULATION_START, token),
            );
            log_native_failure(
                "slPCLSetMarker SimulationEnd",
                set_marker(PCL_MARKER_SIMULATION_END, token),
            );
            log_native_failure(
                "slPCLSetMarker RenderSubmitStart",
                set_marker(PCL_MARKER_RENDER_SUBMIT_START, token),
            );
        }
    }

    let viewport = ViewportHandle::new(0);
    if sl.last_constants_frame != Some(frame_index) {
        unsafe {
            log_native_failure(
                "slSetConstants",
                (sl.core.sl_set_constants)(constants, token, &viewport),
            );
        }
        sl.last_constants_frame = Some(frame_index);
    }

    // Tag Depth and MotionVectors for this frame token.
    // On Vulkan, omit optional HUDLessColor so `sl.dlss_g` reads directly from
    // the swapchain backbuffer (guaranteed exact swapchain extent & PRESENT_SRC
    // layout at vkQueuePresentKHR, avoiding resize/fullscreen mismatches).
    if enabled {
        let mut depth_res = make_sl_resource(sl, depth, false);
        let mut mv_res = make_sl_resource(sl, motion_vectors, false);
        // HUD-less color is optional. Until a dedicated retained HUD-less
        // texture is provided, let the provider consume its swapchain image.
        let mut color_res: Option<Resource> = None;
        let render_extent = Extent {
            top: 0,
            left: 0,
            width: render_size[0],
            height: render_size[1],
        };
        let display_extent = Extent {
            top: 0,
            left: 0,
            width: display_size[0],
            height: display_size[1],
        };
        let mut tags = Vec::with_capacity(3);
        if let Some(r) = depth_res.as_mut() {
            tags.push(ResourceTag::new(r, K_BUFFER_TYPE_DEPTH, render_extent));
        }
        if let Some(r) = mv_res.as_mut() {
            tags.push(ResourceTag::new(
                r,
                K_BUFFER_TYPE_MOTION_VECTORS,
                render_extent,
            ));
        }
        if let Some(r) = color_res.as_mut() {
            tags.push(ResourceTag::new(
                r,
                K_BUFFER_TYPE_HUD_LESS_COLOR,
                display_extent,
            ));
        }
        if !tags.is_empty() {
            let cmd_buffer = cmd_buffer.ok_or(Error::UnsupportedBackend)?;
            barrier_encoder.transition_resources(
                std::iter::empty(),
                [depth, motion_vectors]
                    .into_iter()
                    .map(|view| wgpu::TextureTransition {
                        texture: view.texture(),
                        selector: None,
                        state: wgpu::TextureUses::RESOURCE,
                    }),
            );
            with_dlss_command_buffer(sl.is_dx12, cmd_buffer, |raw_cmd| {
                let tag_res = unsafe {
                    (sl.core.sl_set_tag_for_frame)(
                        token,
                        &viewport,
                        tags.as_ptr(),
                        tags.len() as u32,
                        raw_cmd,
                    )
                };
                if tag_res != SL_RESULT_OK {
                    return Err(Error::Api("slSetTagForFrame", tag_res));
                }
                Ok(())
            })?;
        }
    }

    let target_mode = if enabled {
        DLSSG_MODE_ON
    } else {
        DLSSG_MODE_OFF
    };
    let (mvec_fmt, depth_fmt, hudless_fmt) = if sl.is_dx12 {
        (0, 0, 0)
    } else {
        let native_format = |view: &wgpu::TextureView| {
            unsafe {
                view.as_hal::<wgpu::hal::api::Vulkan>()
                    .map(|h| h.raw_format().as_raw() as u32)
            }
            .ok_or(Error::UnsupportedBackend)
        };
        (
            native_format(motion_vectors)?,
            native_format(depth)?,
            native_format(hudless_color)?,
        )
    };
    let opts = DlssGOptions {
        mode: target_mode,
        on_error_callback: on_dlssg_api_error as *const c_void,
        num_frames_to_generate: clamped_gen.max(1),
        // Let the interposed swapchain provide its actual image count; wgpu
        // can change latency/image count across surface configurations.
        num_back_buffers: 0,
        mvec_depth_width: render_size[0],
        mvec_depth_height: render_size[1],
        color_width: display_size[0],
        color_height: display_size[1],
        mvec_buffer_format: mvec_fmt,
        depth_buffer_format: depth_fmt,
        hud_less_buffer_format: hudless_fmt,
        ..DlssGOptions::default()
    };
    let out_state = FrameGenState {
        max_generated_frames: sl.capabilities.max_generated_frames,
        // Recording a request does not measure presentation. Counts are read
        // separately by the present hook after the native Present call.
        num_frames_actually_presented: 0,
        ..Default::default()
    };
    let mode_changed = sl.last_mode != target_mode
        || sl.last_num_frames != clamped_gen
        || sl.last_dlssg_opts.mvec_depth_width != render_size[0]
        || sl.last_dlssg_opts.mvec_depth_height != render_size[1]
        || sl.last_dlssg_opts.color_width != display_size[0]
        || sl.last_dlssg_opts.color_height != display_size[1]
        || sl.last_dlssg_opts.mvec_buffer_format != mvec_fmt
        || sl.last_dlssg_opts.depth_buffer_format != depth_fmt;
    if let Some(set_opts) = sl.features.dlss_g_set_options
        && mode_changed
    {
        let res = unsafe { set_opts(&viewport, &opts) };
        tracing::info!(target: "solarik_fg_diagnostics", generation = sl.diagnostics.generation, ordinal = sl.diagnostics.ordinal, frame_index, token = ?token, result = res, options = ?opts, thread = ?std::thread::current().id(), "slDLSSGSetOptions frame request; not adoption acknowledgment");
        if res != SL_RESULT_OK {
            return Err(Error::Api("slDLSSGSetOptions", res));
        }
        sl.last_mode = target_mode;
        sl.has_enabled_dlssg |= target_mode == DLSSG_MODE_ON;
        sl.last_num_frames = clamped_gen;
        sl.last_dlssg_opts = opts;
    }

    if let Some(set_marker) = sl.features.pcl_set_marker {
        unsafe {
            log_native_failure(
                "slPCLSetMarker RenderSubmitEnd",
                set_marker(PCL_MARKER_RENDER_SUBMIT_END, token),
            );
        }
    }

    sl.last_recorded_state = out_state;
    Ok(out_state)
}

fn make_sl_resource(
    sl: &mut Streamline,
    view: &wgpu::TextureView,
    is_uav: bool,
) -> Option<Resource> {
    let tex = view.texture();
    let width = tex.width();
    let height = tex.height();
    if sl.is_dx12 {
        #[cfg(windows)]
        {
            let ptr = unsafe { tex.as_hal::<wgpu::hal::api::Dx12>() }?.raw_resource_ptr();
            // D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE (0x40) | PIXEL_SHADER_RESOURCE (0x80)
            // vs D3D12_RESOURCE_STATE_UNORDERED_ACCESS (0x8)
            let state = if is_uav { 0x8 } else { 0xC0 };
            Some(Resource::new_tex2d(ptr, state, width, height, 0))
        }
        #[cfg(not(windows))]
        {
            let _ = (is_uav, width, height);
            None
        }
    } else {
        let image = unsafe {
            tex.as_hal::<wgpu::hal::api::Vulkan>()
                .map(|h| h.raw_handle())
        }?;
        let (raw_view, vk_format, range) = unsafe {
            view.as_hal::<wgpu::hal::api::Vulkan>().map(|h| {
                (
                    h.raw_handle().as_raw() as usize as *mut c_void,
                    h.raw_format().as_raw() as u32,
                    h.raw_subresource_range(),
                )
            })
        }?;
        let vk_usage = wgpu_usage_to_vk(tex.usage(), tex.format());
        let layout = vulkan_resource_layout(tex.format(), is_uav);
        let mut res = Resource::new_tex2d(
            image.as_raw() as usize as *mut c_void,
            layout,
            width,
            height,
            vk_format,
        );
        res.view = raw_view;
        res.base.next = sl.resource_metadata.range_pointer(range);
        res.mip_levels = tex.mip_level_count();
        res.array_layers = tex.depth_or_array_layers();
        res.usage = vk_usage;
        Some(res)
    }
}

fn vulkan_resource_layout(format: wgpu::TextureFormat, storage: bool) -> u32 {
    // Match wgpu-hal's derive_image_layout for the RESOURCE/STORAGE states
    // established by transition_dlss_resources. Sampled depth is NOT layout 5.
    let layout = if format.is_depth_stencil_format() {
        ash::vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL
    } else if storage {
        ash::vk::ImageLayout::GENERAL
    } else {
        ash::vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
    };
    layout.as_raw() as u32
}

fn wgpu_usage_to_vk(usage: wgpu::TextureUsages, format: wgpu::TextureFormat) -> u32 {
    let mut flags = ash::vk::ImageUsageFlags::empty();
    if usage.contains(wgpu::TextureUsages::COPY_SRC) {
        flags |= ash::vk::ImageUsageFlags::TRANSFER_SRC;
    }
    if usage.contains(wgpu::TextureUsages::COPY_DST) {
        flags |= ash::vk::ImageUsageFlags::TRANSFER_DST;
    }
    if usage.contains(wgpu::TextureUsages::TEXTURE_BINDING) {
        flags |= ash::vk::ImageUsageFlags::SAMPLED;
    }
    if usage.contains(wgpu::TextureUsages::STORAGE_BINDING) {
        flags |= ash::vk::ImageUsageFlags::STORAGE;
    }
    if usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT) {
        if format.has_depth_aspect() || format.has_stencil_aspect() {
            flags |= ash::vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT;
        } else {
            flags |= ash::vk::ImageUsageFlags::COLOR_ATTACHMENT;
        }
    }
    flags.as_raw()
}

impl Streamline {
    /// Whether Streamline owns a successful SDK initialization, including preinit
    /// before device attachment or after an attachment failure. This is ownership,
    /// not a feature-capability or renderer-readiness signal.
    pub fn is_initialized() -> bool {
        is_ready() || pre_init_slot().lock().unwrap().is_some()
    }

    fn shutdown_native(&mut self) {
        if self.native_shutdown {
            return;
        }
        self.native_shutdown = true;
        // SAFETY: final owner has stopped producers and waited for submitted GPU work; the
        // runtime and its export library remain alive for this native shutdown invocation.
        tracing::info!("Streamline native shutdown begin");
        let result = unsafe { (self.core.sl_shutdown)() };
        tracing::info!(result, "Streamline native shutdown end");
    }
}
impl Drop for Streamline {
    fn drop(&mut self) {
        self.shutdown_native();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_diagnostics_snapshot_budget_and_generations() {
        let mut d = PresentDiagnostics::default();
        let mut snapshots = Vec::new();
        for n in 1..=150 {
            if d.observe((0, 0, 2, 123, n)) {
                snapshots.push(n);
            }
        }
        assert_eq!(snapshots, [1, 30, 60, 120]);
        assert!(d.observe((0, 4, 1, 123, 151)));
        assert!(!d.observe((0, 4, 1, 123, 152)));
        d.reconfigured();
        assert_eq!((d.generation, d.ordinal, d.since_change), (1, 152, 0));
        assert!(d.observe((0, 4, 1, 123, 153)));
        assert_eq!((d.ordinal, d.since_change), (153, 1));
    }

    #[test]
    fn proxy_preferences_do_not_enable_manual_hooking() {
        let dx12 = preference_flags(true, true);
        assert_eq!(dx12 & PREF_USE_MANUAL_HOOKING, 0);
        assert_ne!(dx12 & (1 << 5), 0); // SDK eUseDXGIFactoryProxy
        let vulkan = preference_flags(false, true);
        assert_eq!(vulkan & (PREF_USE_MANUAL_HOOKING | (1 << 5)), 0);
        let manual = preference_flags(false, false);
        assert_ne!(manual & PREF_USE_MANUAL_HOOKING, 0);
        for flags in [dx12, vulkan, manual] {
            assert_ne!(flags & PREF_DISABLE_CL_STATE_TRACKING, 0);
            assert_ne!(flags & PREF_USE_FRAME_BASED_RESOURCE_TAGGING, 0);
        }
    }

    #[test]
    fn subresource_metadata_matches_header_and_survives_shallow_copies_and_owner_moves() {
        use ash::vk::{ImageAspectFlags as Aspect, ImageSubresourceRange as Range};
        let mut owner = ResourceMetadata::default();
        let range = Range {
            aspect_mask: Aspect::DEPTH,
            base_mip_level: 2,
            level_count: 3,
            base_array_layer: 4,
            layer_count: 5,
        };
        let mut resource = Resource::new_tex2d(ptr::null_mut(), 4, 64, 32, 126);
        resource.base.next = owner.range_pointer(range);
        let copied = resource; // ResourceTaggingForFrame keeps this kind of shallow copy.
        let pointer = copied.base.next;
        assert_eq!(pointer, owner.range_pointer(range));
        for mip in 10..1034 {
            owner.range_pointer(Range {
                base_mip_level: mip,
                ..range
            });
        }
        let mut moved_owner = Box::new(owner);
        assert_eq!(pointer, moved_owner.range_pointer(range));
        // SAFETY: moved_owner retains the immutable boxed extension allocation.
        let extension = unsafe { &*copied.base.next.cast::<SubresourceRange>() };
        assert_eq!(size_of::<SubresourceRange>(), 56);
        assert_eq!(align_of::<SubresourceRange>(), 8);
        assert_eq!(
            extension.base.struct_type,
            StructType {
                data1: 0x8d4c316c,
                data2: 0xd402,
                data3: 0x4524,
                data4: [0x89, 0xa7, 0x14, 0xe7, 0x9e, 0x63, 0x8e, 0x3a],
            }
        );
        assert_eq!(extension.base.struct_version, 1);
        assert!(extension.base.next.is_null());
        assert_eq!(
            [
                extension.aspect_mask,
                extension.base_mip_level,
                extension.level_count,
                extension.base_array_layer,
                extension.layer_count,
            ],
            [2, 2, 3, 4, 5]
        );
        // Every tuple component, including the exact view aspect, participates in the key.
        for changed in [
            Range {
                aspect_mask: Aspect::STENCIL,
                ..range
            },
            Range {
                aspect_mask: Aspect::DEPTH | Aspect::STENCIL,
                ..range
            },
            Range {
                aspect_mask: Aspect::COLOR,
                ..range
            },
            Range {
                base_mip_level: 1,
                ..range
            },
            Range {
                level_count: 1,
                ..range
            },
            Range {
                base_array_layer: 1,
                ..range
            },
            Range {
                layer_count: 1,
                ..range
            },
        ] {
            let other = moved_owner.range_pointer(changed);
            assert_ne!(pointer, other);
            // SAFETY: other points into moved_owner's live allocation.
            assert_eq!(
                unsafe { (*other.cast::<SubresourceRange>()).aspect_mask },
                changed.aspect_mask.as_raw()
            );
        }
        assert_eq!(moved_owner.ranges.len(), 1032);
    }

    #[test]
    fn resource_layout_matches_wgpu_sampled_depth_color_and_storage_states() {
        use wgpu::TextureFormat;
        assert_eq!(
            vulkan_resource_layout(TextureFormat::Depth32Float, false),
            4
        );
        assert_eq!(
            vulkan_resource_layout(TextureFormat::Depth24PlusStencil8, false),
            4
        );
        assert_eq!(vulkan_resource_layout(TextureFormat::R32Float, false), 5);
        assert_eq!(vulkan_resource_layout(TextureFormat::Rg16Float, false), 5);
        assert_eq!(vulkan_resource_layout(TextureFormat::Rgba16Float, true), 1);
    }

    #[test]
    fn presentation_counts_use_only_successful_native_observations() {
        assert_eq!(reported_presented_frames(SL_RESULT_OK, 0), 0);
        assert_eq!(reported_presented_frames(SL_RESULT_OK, 2), 2);
        assert_eq!(reported_presented_frames(19, 2), 0);
        let counts = [(0, 2), (0, 0), (0, 1), (19, 2), (0, 2)];
        let total: u64 = counts
            .into_iter()
            .map(|(result, count)| reported_presented_frames(result, count))
            .sum();
        assert_eq!(total, 5);
    }

    #[test]
    fn plugin_search_includes_separate_ngx_models_and_executable_without_duplicates() {
        let plugin = std::path::Path::new("native/x86_64-pc-windows-msvc/streamline");
        let models = std::path::Path::new("native/x86_64-pc-windows-msvc/dlss");
        let executable = std::path::Path::new("target/capture");
        let paths = plugin_and_model_paths(
            plugin,
            Some(executable),
            Some(std::path::Path::new("native")),
        );
        assert_eq!(
            paths,
            [
                plugin.to_path_buf(),
                models.to_path_buf(),
                executable.to_path_buf()
            ]
        );
        let installed = plugin_and_model_paths(executable, Some(executable), None);
        assert_eq!(
            installed
                .iter()
                .filter(|path| path.as_path() == executable)
                .count(),
            1
        );
    }

    #[test]
    fn sdk_version_matches_2_14_16_header() {
        assert_eq!(
            K_SDK_VERSION,
            (2u64 << 48) | (14u64 << 32) | (16u64 << 16) | 0xfedc
        );
    }

    #[test]
    fn default_structs_carry_header_guids_and_versions() {
        let vp = ViewportHandle::new(0);
        assert_eq!(vp.base.struct_type, GUID_VIEWPORT_HANDLE);
        assert_eq!(vp.base.struct_version, K_STRUCT_VERSION_1);

        let opts = DlssGOptions::default();
        assert_eq!(opts.base.struct_type, GUID_DLSSG_OPTIONS);
        assert_eq!(opts.base.struct_version, K_STRUCT_VERSION_5);
        assert_eq!(opts.mode, DLSSG_MODE_OFF);
        assert_eq!(opts.num_frames_to_generate, 1);

        let state = DlssGState::default();
        assert_eq!(state.base.struct_type, GUID_DLSSG_STATE);
        assert_eq!(state.base.struct_version, K_STRUCT_VERSION_4);

        let reflex = ReflexOptions::default();
        assert_eq!(reflex.base.struct_type, GUID_REFLEX_OPTIONS);
        assert_eq!(reflex.base.struct_version, K_STRUCT_VERSION_1);

        let vk = VulkanInfo::default();
        assert_eq!(vk.base.struct_type, GUID_VULKAN_INFO);
        assert_eq!(vk.base.struct_version, K_STRUCT_VERSION_3);
    }

    #[test]
    fn wide_path_encoding_is_nul_terminated() {
        let wide = to_wide_nul(std::path::Path::new("C:\\sl"));
        assert_eq!(wide.last(), Some(&0));
        assert!(wide.len() >= 5);
    }

    #[test]
    fn capabilities_default_to_unavailable_before_attach() {
        let caps = FrameGenCapabilities::default();
        assert!(!caps.dlss_g_supported);
        assert!(!caps.mfg_supported);
        assert_eq!(caps.max_generated_frames, 0);
    }

    #[test]
    fn feature_list_excludes_dlss_g_when_disabled_and_keeps_sr_rr_pcl_reflex() {
        let with_fg = features_to_load(true);
        assert!(with_fg.contains(&K_FEATURE_DLSS));
        assert!(with_fg.contains(&K_FEATURE_DLSS_RR));
        assert!(with_fg.contains(&K_FEATURE_PCL));
        assert!(with_fg.contains(&K_FEATURE_REFLEX));
        assert!(with_fg.contains(&K_FEATURE_DLSS_G));
        assert_eq!(with_fg.len(), 5);

        let without_fg = features_to_load(false);
        assert!(without_fg.contains(&K_FEATURE_DLSS));
        assert!(without_fg.contains(&K_FEATURE_DLSS_RR));
        assert!(without_fg.contains(&K_FEATURE_PCL));
        assert!(without_fg.contains(&K_FEATURE_REFLEX));
        assert!(!without_fg.contains(&K_FEATURE_DLSS_G));
        assert_eq!(without_fg.len(), 4);
    }
}
