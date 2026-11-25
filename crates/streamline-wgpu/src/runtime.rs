//! Dynamic loader and runtime controller for NVIDIA Streamline 2.14.16.

use crate::abi::*;
use ash::vk::Handle as _;
use std::{
    ffi::{CString, c_char, c_void},
    path::PathBuf,
    ptr,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
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

type FnDlssSetOptions =
    unsafe extern "C" fn(*const ViewportHandle, *const DlssOptions) -> SlResult;
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

/// Process-global Streamline controller.
pub struct Streamline {
    core: CoreExports,
    features: FeatureExports,
    capabilities: FrameGenCapabilities,
    active_frame_token: *mut BaseStructure,
    last_mode: u32,
    last_num_frames: u32,
    is_dx12: bool,
    native_shutdown: bool,
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
}

static PRE_INIT: OnceLock<Mutex<Option<PreInit>>> = OnceLock::new();

/// Whether [`preinit`] routed Vulkan entry points through the interposer, so its
/// swapchain proxies can own present for DLSS-G.
static VULKAN_INTERPOSED: AtomicBool = AtomicBool::new(false);

fn pre_init_slot() -> &'static Mutex<Option<PreInit>> {
    PRE_INIT.get_or_init(|| Mutex::new(None))
}

/// Issue `slInit` with the preferences this crate always uses.
fn init_streamline(
    core: &CoreExports,
    project_id_c: &CString,
    plugin_dir_wide: &[u16],
    is_dx12: bool,
    use_interposer_proxies: bool,
) -> Result<(), Error> {
    let plugin_paths = [plugin_dir_wide.as_ptr()];
    let features_to_load = [
        K_FEATURE_DLSS,
        K_FEATURE_DLSS_RR,
        K_FEATURE_PCL,
        K_FEATURE_REFLEX,
        K_FEATURE_DLSS_G,
    ];
    let engine_ver = c"1.0.0";
    let mut flags = PREF_DISABLE_CL_STATE_TRACKING | PREF_USE_FRAME_BASED_RESOURCE_TAGGING;
    if !use_interposer_proxies {
        flags |= PREF_USE_MANUAL_HOOKING;
    }
    let prefs = Preferences {
        base: BaseStructure::new(GUID_PREFERENCES, K_STRUCT_VERSION_1),
        show_console: false,
        log_level: LOG_LEVEL_DEFAULT,
        paths_to_plugins: plugin_paths.as_ptr(),
        num_paths_to_plugins: plugin_paths.len() as u32,
        path_to_logs_and_data: ptr::null(),
        allocate_callback: ptr::null(),
        release_callback: ptr::null(),
        log_message_callback: ptr::null(),
        flags,
        features_to_load: features_to_load.as_ptr(),
        num_features_to_load: features_to_load.len() as u32,
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
    if pre_init_slot().lock().unwrap().is_some() {
        return Ok(());
    }
    let (core, plugin_dir) = load_core()?;
    let plugin_dir_wide = to_wide_nul(&plugin_dir);
    let project_id_c = CString::new(project_id_uuid).unwrap_or_else(|_| CString::new("").unwrap());
    init_streamline(&core, &project_id_c, &plugin_dir_wide, is_dx12, !is_dx12)?;
    if !is_dx12 {
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
    let (core, plugin_dir_wide, project_id_c) = match pre {
        Some(pre) if pre.is_dx12 == is_dx12 => (pre.core, pre.plugin_dir_wide, pre.project_id_c),
        _ => {
            let (core, plugin_dir) = load_core()?;
            let plugin_dir_wide = to_wide_nul(&plugin_dir);
            let project_id_c =
                CString::new(project_id_uuid).unwrap_or_else(|_| CString::new("").unwrap());
            init_streamline(&core, &project_id_c, &plugin_dir_wide, is_dx12, false)?;
            (core, plugin_dir_wide, project_id_c)
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
                unsafe {
                    let _ = (core.sl_shutdown)();
                }
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
                unsafe {
                    let _ = (core.sl_shutdown)();
                }
                return Err(Error::Api("slSetVulkanInfo", set_res));
            }
        }
    }

    let dlss_sr_res = unsafe { (core.sl_is_feature_supported)(K_FEATURE_DLSS, &adapter_info) };
    let dlss_rr_res = unsafe { (core.sl_is_feature_supported)(K_FEATURE_DLSS_RR, &adapter_info) };
    let dlss_g_res = unsafe { (core.sl_is_feature_supported)(K_FEATURE_DLSS_G, &adapter_info) };
    let reflex_res = unsafe { (core.sl_is_feature_supported)(K_FEATURE_REFLEX, &adapter_info) };
    let features = FeatureExports {
        dlss_set_options: unsafe {
            query_feature_fn(&core, K_FEATURE_DLSS, b"slDLSSSetOptions\0")
        },
        dlss_get_optimal_settings: unsafe {
            query_feature_fn(&core, K_FEATURE_DLSS, b"slDLSSGetOptimalSettings\0")
        },
        dlss_d_set_options: unsafe {
            query_feature_fn(&core, K_FEATURE_DLSS_RR, b"slDLSSDSetOptions\0")
        },
        dlss_d_get_optimal_settings: unsafe {
            query_feature_fn(&core, K_FEATURE_DLSS_RR, b"slDLSSDGetOptimalSettings\0")
        },
        dlss_g_set_options: unsafe {
            query_feature_fn(&core, K_FEATURE_DLSS_G, b"slDLSSGSetOptions\0")
        },
        dlss_g_get_state: unsafe {
            query_feature_fn(&core, K_FEATURE_DLSS_G, b"slDLSSGGetState\0")
        },
        reflex_set_options: unsafe {
            query_feature_fn(&core, K_FEATURE_REFLEX, b"slReflexSetOptions\0")
        },
        reflex_sleep: unsafe { query_feature_fn(&core, K_FEATURE_REFLEX, b"slReflexSleep\0") },
        pcl_set_marker: unsafe { query_feature_fn(&core, K_FEATURE_PCL, b"slPCLSetMarker\0") },
    };

    let mut max_generated_frames = if dlss_g_res == SL_RESULT_OK { 1 } else { 0 };
    if dlss_g_res == SL_RESULT_OK
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
    let dlss_g_supported =
        interposed && dlss_g_res == SL_RESULT_OK && features.dlss_g_set_options.is_some();
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
        last_mode: DLSSG_MODE_OFF,
        last_num_frames: 0,
        is_dx12,
        native_shutdown: false,
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

/// Native shutdown with the interposer/plugin DLL owner retained. Drop the returned value only
/// after all queue/device/swapchain objects that can call provider code have been destroyed.
pub fn shutdown_retaining_runtime() -> Option<RetiredRuntime> {
    #[cfg(windows)]
    wgpu::hal::dx12::set_swapchain_hooks(wgpu::hal::dx12::Dx12SwapchainHooks::default());
    wgpu::hal::vulkan::set_swapchain_interceptor(
        wgpu::hal::vulkan::VulkanSwapchainInterceptor::default(),
    );
    let mut runtime = global_slot().lock().unwrap().take()?;
    let plugins = runtime.retain_loaded_plugins();
    runtime.shutdown_native();
    Some(RetiredRuntime {
        runtime: Some(runtime),
        plugins,
    })
}

/// Shutting down SL can release its own module references. This owner additionally retains
/// already-loaded plugin DLLs and the interposer until the final device/queue teardown.
pub struct RetiredRuntime {
    runtime: Option<Streamline>,
    plugins: Vec<libloading::Library>,
}
impl Drop for RetiredRuntime {
    fn drop(&mut self) {
        // Plugin DLL detach runs while the interposer is still loaded.
        self.plugins.clear();
        drop(self.runtime.take());
    }
}

/// Install the `wgpu-hal` swapchain/present interception hooks for the active
/// backend (DX12 `slUpgradeInterface` or Vulkan `sl.interposer.dll` proxy fns).
pub fn install_swapchain_interception() {
    let guard = global_slot().lock().unwrap();
    let Some(sl) = guard.as_ref() else {
        return;
    };
    if sl.is_dx12 {
        #[cfg(windows)]
        {
            wgpu::hal::dx12::set_swapchain_hooks(wgpu::hal::dx12::Dx12SwapchainHooks {
                on_swapchain_created: Some(dx12_upgrade_swapchain),
                on_present_start: Some(on_present_start),
                on_present_end: Some(on_present_end),
                ..Default::default()
            });
            tracing::info!("Streamline DX12 swapchain/present interception hooks installed");
        }
    } else {
        // When `preinit` routed `ash::Entry` through `sl.interposer.dll`,
        // `ash::khr::swapchain::Device` already holds the interposer's own
        // `vkCreateSwapchainKHR`/`vkQueuePresentKHR` proxies — do not double-wrap.
        // Attach only the present markers (PCL/Reflex).
        wgpu::hal::vulkan::set_swapchain_interceptor(
            wgpu::hal::vulkan::VulkanSwapchainInterceptor {
                create_swapchain: None,
                destroy_swapchain: None,
                get_swapchain_images: None,
                acquire_next_image: None,
                queue_present: None,
                on_present_start: Some(on_present_start),
                on_present_end: Some(on_present_end),
            },
        );
        tracing::info!("Streamline Vulkan present-marker hooks installed");
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

fn on_present_start() {
    if let Ok(guard) = global_slot().try_lock()
        && let Some(sl) = guard.as_ref()
        && !sl.active_frame_token.is_null()
        && let Some(set_marker) = sl.features.pcl_set_marker
    {
        unsafe {
            let _ = set_marker(PCL_MARKER_PRESENT_START, sl.active_frame_token);
        }
    }
}

fn on_present_end() {
    if let Ok(guard) = global_slot().try_lock()
        && let Some(sl) = guard.as_ref()
        && !sl.active_frame_token.is_null()
        && let Some(set_marker) = sl.features.pcl_set_marker
    {
        unsafe {
            let _ = set_marker(PCL_MARKER_PRESENT_END, sl.active_frame_token);
        }
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
            return Some([settings.optimal_render_width, settings.optimal_render_height]);
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
            return Some([settings.optimal_render_width, settings.optimal_render_height]);
        }
    }
    None
}

/// Evaluate Streamline DLSS Super Resolution (`kFeatureDLSS`) on the active
/// command encoder.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_dlss_sr(
    frame_index: u32,
    mode: u32,
    output_size: [u32; 2],
    render_size: [u32; 2],
    jitter_offset: [f32; 2],
    reset: bool,
    color_in: &wgpu::TextureView,
    color_out: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    motion_vectors: &wgpu::TextureView,
    exposure: Option<&wgpu::TextureView>,
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
    let constants = Constants {
        jitter_offset,
        mvec_scale: [1.0, 1.0],
        reset: if reset { BOOLEAN_TRUE } else { BOOLEAN_FALSE },
        ..Constants::default()
    };
    unsafe {
        let _ = (sl.core.sl_set_constants)(&constants, token, &viewport);
    }

    let opts = DlssOptions {
        mode,
        output_width: output_size[0],
        output_height: output_size[1],
        ..DlssOptions::default()
    };
    let opt_res = unsafe { set_opts(&viewport, &opts) };
    if opt_res != SL_RESULT_OK {
        return Err(Error::Api("slDLSSSetOptions", opt_res));
    }

    let mut in_res = make_sl_resource(sl.is_dx12, color_in, false);
    let mut out_res = make_sl_resource(sl.is_dx12, color_out, true);
    let mut depth_res = make_sl_resource(sl.is_dx12, depth, false);
    let mut mv_res = make_sl_resource(sl.is_dx12, motion_vectors, false);
    let mut exp_res = exposure.and_then(|v| make_sl_resource(sl.is_dx12, v, false));

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
    push_tag(in_res.as_mut(), K_BUFFER_TYPE_SCALING_INPUT_COLOR, render_ext);
    push_tag(out_res.as_mut(), K_BUFFER_TYPE_SCALING_OUTPUT_COLOR, out_ext);
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

    let raw_cmd = extract_cmd_ptr(sl.is_dx12, Some(cmd_buffer));
    unsafe {
        let _ = (sl.core.sl_set_tag_for_frame)(
            token,
            &viewport,
            tags.as_ptr(),
            tags.len() as u32,
            raw_cmd,
        );
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
}

/// Evaluate Streamline DLSS Ray Reconstruction (`kFeatureDLSS_RR`) on the active
/// command encoder.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_dlss_rr(
    frame_index: u32,
    mode: u32,
    output_size: [u32; 2],
    render_size: [u32; 2],
    jitter_offset: [f32; 2],
    reset: bool,
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
    let constants = Constants {
        jitter_offset,
        mvec_scale: [-(render_size[0] as f32), -(render_size[1] as f32)],
        reset: if reset { BOOLEAN_TRUE } else { BOOLEAN_FALSE },
        ..Constants::default()
    };
    unsafe {
        let _ = (sl.core.sl_set_constants)(&constants, token, &viewport);
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

    // Solari writes color_in and all RR guide textures via storage compute
    // passes (`VK_IMAGE_LAYOUT_GENERAL` = 1 on Vulkan).
    let mut in_res = make_sl_resource(sl.is_dx12, color_in, true);
    let mut out_res = make_sl_resource(sl.is_dx12, color_out, true);
    let mut depth_res = make_sl_resource(sl.is_dx12, depth, true);
    let mut mv_res = make_sl_resource(sl.is_dx12, motion_vectors, false);
    let mut diff_res = make_sl_resource(sl.is_dx12, diffuse_albedo, true);
    let mut spec_res = make_sl_resource(sl.is_dx12, specular_albedo, true);
    let mut norm_res = make_sl_resource(sl.is_dx12, normal_roughness, true);
    let mut spec_mv_res =
        specular_motion_vectors.and_then(|v| make_sl_resource(sl.is_dx12, v, true));
    let mut exp_res = exposure.and_then(|v| make_sl_resource(sl.is_dx12, v, false));

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
    push_tag(in_res.as_mut(), K_BUFFER_TYPE_SCALING_INPUT_COLOR, render_ext);
    push_tag(out_res.as_mut(), K_BUFFER_TYPE_SCALING_OUTPUT_COLOR, out_ext);
    push_tag(depth_res.as_mut(), K_BUFFER_TYPE_DEPTH, render_ext);
    push_tag(mv_res.as_mut(), K_BUFFER_TYPE_MOTION_VECTORS, render_ext);
    push_tag(diff_res.as_mut(), K_BUFFER_TYPE_ALBEDO, render_ext);
    push_tag(spec_res.as_mut(), K_BUFFER_TYPE_SPECULAR_ALBEDO, render_ext);
    push_tag(norm_res.as_mut(), K_BUFFER_TYPE_NORMAL_ROUGHNESS, render_ext);
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

    let raw_cmd = extract_cmd_ptr(sl.is_dx12, Some(cmd_buffer));
    unsafe {
        let _ = (sl.core.sl_set_tag_for_frame)(
            token,
            &viewport,
            tags.as_ptr(),
            tags.len() as u32,
            raw_cmd,
        );
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
    cmd_buffer: Option<&mut wgpu::CommandEncoder>,
) -> Result<FrameGenState, Error> {
    let mut guard = global_slot().lock().unwrap();
    let sl = guard.as_mut().ok_or(Error::NotInitialized)?;

    let enabled = num_frames_to_generate > 0 && sl.capabilities.dlss_g_supported;
    if !enabled && sl.last_mode == DLSSG_MODE_OFF {
        return Ok(FrameGenState {
            max_generated_frames: sl.capabilities.max_generated_frames,
            ..Default::default()
        });
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
            let _ = set_reflex(&reflex_opts);
        }
    }
    if enabled && let Some(sleep) = sl.features.reflex_sleep {
        unsafe {
            let _ = sleep(token);
        }
    }
    if let Some(set_marker) = sl.features.pcl_set_marker {
        unsafe {
            let _ = set_marker(PCL_MARKER_SIMULATION_START, token);
            let _ = set_marker(PCL_MARKER_SIMULATION_END, token);
            let _ = set_marker(PCL_MARKER_RENDER_SUBMIT_START, token);
        }
    }

    let viewport = ViewportHandle::new(0);
    unsafe {
        let _ = (sl.core.sl_set_constants)(constants, token, &viewport);
    }

    // Tag Depth, MotionVectors, and HUDLessColor for this frame token.
    if enabled {
        let mut depth_res = make_sl_resource(sl.is_dx12, depth, false);
        let mut mv_res = make_sl_resource(sl.is_dx12, motion_vectors, false);
        let mut color_res = make_sl_resource(sl.is_dx12, hudless_color, false);
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
            let raw_cmd = extract_cmd_ptr(sl.is_dx12, cmd_buffer);
            unsafe {
                let _ = (sl.core.sl_set_tag_for_frame)(
                    token,
                    &viewport,
                    tags.as_ptr(),
                    tags.len() as u32,
                    raw_cmd,
                );
            }
        }
    }

    let target_mode = if enabled {
        DLSSG_MODE_ON
    } else {
        DLSSG_MODE_OFF
    };
    let opts = DlssGOptions {
        mode: target_mode,
        num_frames_to_generate: clamped_gen.max(1),
        num_back_buffers: 3,
        mvec_depth_width: render_size[0],
        mvec_depth_height: render_size[1],
        color_width: display_size[0],
        color_height: display_size[1],
        ..DlssGOptions::default()
    };
    if let Some(set_opts) = sl.features.dlss_g_set_options
        && (sl.last_mode != target_mode || sl.last_num_frames != clamped_gen)
    {
        let res = unsafe { set_opts(&viewport, &opts) };
        if res == SL_RESULT_OK {
            sl.last_mode = target_mode;
            sl.last_num_frames = clamped_gen;
        }
    }

    let mut out_state = FrameGenState {
        max_generated_frames: sl.capabilities.max_generated_frames,
        ..Default::default()
    };
    if let Some(get_state) = sl.features.dlss_g_get_state {
        let mut raw_state = DlssGState::default();
        if unsafe { get_state(&viewport, &mut raw_state, &opts) } == SL_RESULT_OK {
            out_state.status = raw_state.status;
            out_state.num_frames_actually_presented = raw_state.num_frames_actually_presented;
            if raw_state.num_frames_to_generate_max > 0 {
                out_state.max_generated_frames = raw_state.num_frames_to_generate_max;
            }
        }
    }

    if let Some(set_marker) = sl.features.pcl_set_marker {
        unsafe {
            let _ = set_marker(PCL_MARKER_RENDER_SUBMIT_END, token);
        }
    }

    Ok(out_state)
}

fn make_sl_resource(is_dx12: bool, view: &wgpu::TextureView, is_uav: bool) -> Option<Resource> {
    let tex = view.texture();
    let width = tex.width();
    let height = tex.height();
    if is_dx12 {
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
        let raw_view = unsafe {
            view.as_hal::<wgpu::hal::api::Vulkan>()
                .map(|h| h.raw_handle().as_raw() as usize as *mut c_void)
        }
        .unwrap_or(ptr::null_mut());
        let vk_format = wgpu_format_to_vk(tex.format());
        let vk_usage = wgpu_usage_to_vk(tex.usage(), tex.format());
        // VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL (5) vs VK_IMAGE_LAYOUT_GENERAL (1)
        let layout = if is_uav { 1 } else { 5 };
        let mut res = Resource::new_tex2d(
            image.as_raw() as usize as *mut c_void,
            layout,
            width,
            height,
            vk_format,
        );
        res.view = raw_view;
        res.usage = vk_usage;
        Some(res)
    }
}

fn wgpu_format_to_vk(format: wgpu::TextureFormat) -> u32 {
    use wgpu::TextureFormat as F;
    match format {
        F::Rgba8Unorm => ash::vk::Format::R8G8B8A8_UNORM.as_raw() as u32,
        F::Rgba8UnormSrgb => ash::vk::Format::R8G8B8A8_SRGB.as_raw() as u32,
        F::Bgra8Unorm => ash::vk::Format::B8G8R8A8_UNORM.as_raw() as u32,
        F::Bgra8UnormSrgb => ash::vk::Format::B8G8R8A8_SRGB.as_raw() as u32,
        F::Rgba16Float => ash::vk::Format::R16G16B16A16_SFLOAT.as_raw() as u32,
        F::Rgba32Float => ash::vk::Format::R32G32B32A32_SFLOAT.as_raw() as u32,
        F::Rg16Float => ash::vk::Format::R16G16_SFLOAT.as_raw() as u32,
        F::Rg32Float => ash::vk::Format::R32G32_SFLOAT.as_raw() as u32,
        F::R32Float => ash::vk::Format::R32_SFLOAT.as_raw() as u32,
        F::R16Float => ash::vk::Format::R16_SFLOAT.as_raw() as u32,
        F::Depth32Float => ash::vk::Format::D32_SFLOAT.as_raw() as u32,
        F::Depth32FloatStencil8 => ash::vk::Format::D32_SFLOAT_S8_UINT.as_raw() as u32,
        F::Depth24Plus | F::Depth24PlusStencil8 => ash::vk::Format::D24_UNORM_S8_UINT.as_raw() as u32,
        F::Rgb10a2Unorm => ash::vk::Format::A2B10G10R10_UNORM_PACK32.as_raw() as u32,
        F::Rg11b10Ufloat => ash::vk::Format::B10G11R11_UFLOAT_PACK32.as_raw() as u32,
        _ => ash::vk::Format::UNDEFINED.as_raw() as u32,
    }
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

fn extract_cmd_ptr(is_dx12: bool, cmd_buffer: Option<&mut wgpu::CommandEncoder>) -> *mut c_void {
    let Some(enc) = cmd_buffer else {
        return ptr::null_mut();
    };
    if is_dx12 {
        #[cfg(windows)]
        {
            unsafe {
                enc.as_hal_mut::<wgpu::hal::api::Dx12, _, _>(|hal| {
                    hal.and_then(|h| h.raw_command_list_ptr())
                        .unwrap_or(ptr::null_mut())
                })
            }
        }
        #[cfg(not(windows))]
        {
            let _ = enc;
            ptr::null_mut()
        }
    } else {
        unsafe {
            enc.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|hal| {
                hal.map(|h| h.raw_handle().as_raw() as usize as *mut c_void)
                    .unwrap_or(ptr::null_mut())
            })
        }
    }
}

impl Streamline {
    fn retain_loaded_plugins(&self) -> Vec<libloading::Library> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            let length = self
                ._plugin_dir_wide
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(self._plugin_dir_wide.len());
            let directory = PathBuf::from(std::ffi::OsString::from_wide(
                &self._plugin_dir_wide[..length],
            ));
            let mut retained = Vec::new();
            if let Ok(entries) = std::fs::read_dir(directory) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                    if name.starts_with("sl.") && name.ends_with(".dll") {
                        // This only AddRefs an existing module; it never initializes a new
                        // provider, changes enabled features, or permanently pins a DLL.
                        if let Ok(library) =
                            libloading::os::windows::Library::open_already_loaded(entry.path())
                        {
                            retained.push(library.into());
                        }
                    }
                }
            }
            retained
        }
        #[cfg(not(windows))]
        {
            Vec::new()
        }
    }

    fn shutdown_native(&mut self) {
        if self.native_shutdown {
            return;
        }
        self.native_shutdown = true;
        // SAFETY: final owner has stopped producers and waited for submitted GPU work; the
        // runtime and its export library remain alive for this native shutdown invocation.
        unsafe {
            let _ = (self.core.sl_shutdown)();
        }
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
}
