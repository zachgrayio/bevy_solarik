//! The only unsafe ABI boundary in this crate: the pinned AMD `FidelityFX` SDK
//! "`FfxApi`" C declarations and their runtime-loaded function table.
//!
//! Transcribed from the FidelityFX-SDK headers (`Kits/FidelityFX/api/include/ffx_api.h`,
//! `ffx_api_types.h`, `ffx_api_loader.h`, `vk/ffx_api_vk.h` and
//! `Kits/FidelityFX/upscalers/include/ffx_upscale.h`). `src/abi.h` records the
//! exact C declarations these `#[repr(C)]` types mirror. AMD's own
//! `ffx_api_loader.h` loads these five exports with `GetProcAddress` at runtime;
//! we do the same with `libloading`, so no AMD binary is ever a link-time input.

use std::{
    ffi::{OsString, c_void},
    path::PathBuf,
};

pub(crate) type Context = *mut c_void;
pub(crate) type ReturnCode = u32;
pub(crate) type StructType = u64;

/// `FFX_API_EFFECT_ID_UPSCALE` and the `FFX_API_MAKE_EFFECT_SUB_ID` macro from `ffx_api.h`.
const EFFECT_ID_UPSCALE: u32 = 0x0001_0000;
/// `FFX_API_MAKE_EFFECT_SUB_ID(FFX_API_EFFECT_ID_UPSCALE, 0x00)`.
pub(crate) const CREATE_CONTEXT_DESC_UPSCALE: StructType = 0x0001_0000;
/// `FFX_API_MAKE_EFFECT_SUB_ID(FFX_API_EFFECT_ID_UPSCALE, 0x01)`.
pub(crate) const DISPATCH_DESC_UPSCALE: StructType = 0x0001_0001;
/// `FFX_API_MAKE_EFFECT_SUB_ID(FFX_API_EFFECT_ID_UPSCALE, 0x02)`.
pub(crate) const QUERY_DESC_UPSCALE_RATIO: StructType = 0x0001_0002;
/// `FFX_API_MAKE_EFFECT_SUB_ID(FFX_API_EFFECT_ID_UPSCALE, 0x03)`.
pub(crate) const QUERY_DESC_RENDER_RESOLUTION: StructType = 0x0001_0003;
/// `FFX_API_MAKE_EFFECT_SUB_ID(FFX_API_EFFECT_ID_UPSCALE, 0x04)`.
pub(crate) const QUERY_DESC_JITTER_PHASE_COUNT: StructType = 0x0001_0004;
/// `FFX_API_MAKE_EFFECT_SUB_ID(FFX_API_EFFECT_ID_UPSCALE, 0x05)`.
pub(crate) const QUERY_DESC_JITTER_OFFSET: StructType = 0x0001_0005;
/// `FFX_API_CREATE_CONTEXT_DESC_TYPE_BACKEND_VK` is version-dependent. SDK
/// 2.4.0 defines `FFX_API_BACKEND_ID_VK = 0x01000000` and
/// `FFX_API_MAKE_BACKEND_SUB_ID(..., 0x03)` = `0x01000003`; SDK 1.1.x used
/// `0x00000003` (the `_DEPRECATED` descriptor). `0x02000003` belongs to the
/// Xbox backend id and is not a Vulkan type. The layout is identical, only the
/// discriminator differs, so context creation retries these in order unless
/// `FSR_BACKEND_VK_TYPE` overrides.
pub(crate) const BACKEND_VK_TYPES: [StructType; 2] = [0x0100_0003, 0x0000_0003];

const _: () = assert!(EFFECT_ID_UPSCALE == 0x0001_0000);

/// `ffxApiHeader` (also `ffxCreateContextDescHeader`, `ffxDispatchDescHeader`, ...).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfxApiHeader {
    /// Struct type (`FFX_API_*_DESC_TYPE_*`).
    pub kind: StructType,
    /// Next chained descriptor, or null.
    pub next: *mut FfxApiHeader,
}

impl FfxApiHeader {
    pub(crate) const fn new(kind: StructType) -> Self {
        Self {
            kind,
            next: std::ptr::null_mut(),
        }
    }
}

/// `FfxApiDimensions2D`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Dimensions2D {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// `FfxApiFloatCoords2D`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FloatCoords2D {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate.
    pub y: f32,
}

/// `FfxApiResourceDescription`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ResourceDescription {
    /// Resource dimensionality (`FfxApiResourceType`).
    pub resource_type: u32,
    /// Surface format (`FfxApiSurfaceFormat`).
    pub format: u32,
    /// Width in pixels (or byte size for buffers).
    pub width: u32,
    /// Height in pixels (or byte stride for buffers).
    pub height: u32,
    /// Depth / array slice count.
    pub depth: u32,
    /// Mip level count.
    pub mip_count: u32,
    /// Resource flags.
    pub flags: u32,
    /// Resource usage bitmask (`FfxApiResourceUsage`).
    pub usage: u32,
}

/// `FfxApiResource`. For the Vulkan provider `resource` holds a `VkImage`/`VkImageView`
/// cast to `void*`, as produced by `ffxApiGetResourceVK`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Resource {
    /// Native `VkImage` or `ID3D12Resource*` handle.
    pub resource: *mut c_void,
    /// Resource dimensions, format and usage metadata.
    pub description: ResourceDescription,
    /// Current resource state (`FfxApiResourceState`).
    pub state: u32,
}

impl Default for Resource {
    fn default() -> Self {
        Self {
            resource: std::ptr::null_mut(),
            description: ResourceDescription::default(),
            state: 0,
        }
    }
}

/// `ffxAllocationCallbacks`.
#[repr(C)]
pub struct AllocationCallbacks {
    pub user_data: *mut c_void,
    pub alloc: *mut c_void,
    pub dealloc: *mut c_void,
}

/// `ffxCreateContextDescUpscale`.
#[repr(C)]
pub struct CreateContextDescUpscale {
    pub header: FfxApiHeader,
    pub flags: u32,
    pub max_render_size: Dimensions2D,
    pub max_upscale_size: Dimensions2D,
    /// `ffxApiMessage` (`void (*)(uint32_t, const wchar_t*)`); always null here.
    pub message: *mut c_void,
}

/// `VkDeviceContext` from `ffx_api_vk.h`. Handles are opaque pointers on `x86_64`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DeviceContextVk {
    /// `VkDevice`.
    pub device: *mut c_void,
    /// `VkPhysicalDevice`.
    pub physical_device: *mut c_void,
    /// `PFN_vkGetDeviceProcAddr`.
    pub get_device_proc_addr: *mut c_void,
}

/// `ffxCreateBackendVKDesc`, chained from `CreateContextDescUpscale::header::next`.
#[repr(C)]
pub struct CreateBackendVkDesc {
    pub header: FfxApiHeader,
    pub device_context: DeviceContextVk,
}

/// `ffxDispatchDescUpscale`.
#[repr(C)]
pub struct DispatchDescUpscale {
    pub header: FfxApiHeader,
    /// `VkCommandBuffer` cast to `void*`.
    pub command_list: *mut c_void,
    pub color: Resource,
    pub depth: Resource,
    pub motion_vectors: Resource,
    pub exposure: Resource,
    pub reactive: Resource,
    pub transparency_and_composition: Resource,
    pub output: Resource,
    pub jitter_offset: FloatCoords2D,
    pub motion_vector_scale: FloatCoords2D,
    pub render_size: Dimensions2D,
    pub upscale_size: Dimensions2D,
    pub enable_sharpening: bool,
    pub sharpness: f32,
    pub frame_time_delta: f32,
    pub pre_exposure: f32,
    pub reset: bool,
    pub camera_near: f32,
    pub camera_far: f32,
    pub camera_fov_angle_vertical: f32,
    pub view_space_to_meters_factor: f32,
    pub flags: u32,
}

/// `ffxQueryDescUpscaleGetUpscaleRatioFromQualityMode`.
#[repr(C)]
pub struct QueryDescUpscaleRatio {
    pub header: FfxApiHeader,
    pub quality_mode: u32,
    pub out_ratio: *mut f32,
}

/// `ffxQueryDescUpscaleGetRenderResolutionFromQualityMode`.
#[repr(C)]
pub struct QueryDescRenderResolution {
    pub header: FfxApiHeader,
    pub display_width: u32,
    pub display_height: u32,
    pub quality_mode: u32,
    pub out_render_width: *mut u32,
    pub out_render_height: *mut u32,
}

/// `ffxQueryDescUpscaleGetJitterPhaseCount`.
#[repr(C)]
pub struct QueryDescJitterPhaseCount {
    pub header: FfxApiHeader,
    pub render_width: u32,
    pub display_width: u32,
    pub out_phase_count: *mut i32,
}

/// `ffxQueryDescUpscaleGetJitterOffset`.
#[repr(C)]
pub struct QueryDescJitterOffset {
    pub header: FfxApiHeader,
    pub index: i32,
    pub phase_count: i32,
    pub out_x: *mut f32,
    pub out_y: *mut f32,
}

// Pinned 64-bit ABI. A mismatch here means the SDK headers changed; regenerate
// src/abi.h and these bindings together (see docs/fsr.md).
const _: () = assert!(size_of::<FfxApiHeader>() == 16);
const _: () = assert!(size_of::<Dimensions2D>() == 8);
const _: () = assert!(size_of::<FloatCoords2D>() == 8);
const _: () = assert!(size_of::<ResourceDescription>() == 32);
const _: () = assert!(size_of::<Resource>() == 48);
const _: () = assert!(size_of::<AllocationCallbacks>() == 24);
const _: () = assert!(size_of::<CreateContextDescUpscale>() == 48);
const _: () = assert!(size_of::<DeviceContextVk>() == 24);
const _: () = assert!(size_of::<CreateBackendVkDesc>() == 40);
const _: () = assert!(size_of::<DispatchDescUpscale>() == 432);
const _: () = assert!(size_of::<QueryDescUpscaleRatio>() == 32);
const _: () = assert!(size_of::<QueryDescRenderResolution>() == 48);
const _: () = assert!(size_of::<QueryDescJitterPhaseCount>() == 32);
const _: () = assert!(size_of::<QueryDescJitterOffset>() == 40);

/// The five `FfxApi` exports, mirroring `ffxFunctions` in `ffx_api_loader.h`.
#[derive(Clone, Copy)]
pub(crate) struct Functions {
    pub create_context: unsafe extern "C" fn(
        *mut Context,
        *mut FfxApiHeader,
        *const AllocationCallbacks,
    ) -> ReturnCode,
    pub destroy_context:
        unsafe extern "C" fn(*mut Context, *const AllocationCallbacks) -> ReturnCode,
    /// `ffxConfigure`: per-frame frame-generation configuration.
    pub configure: unsafe extern "C" fn(*mut Context, *const FfxApiHeader) -> ReturnCode,
    pub query: unsafe extern "C" fn(*mut Context, *mut FfxApiHeader) -> ReturnCode,
    pub dispatch: unsafe extern "C" fn(*mut Context, *const FfxApiHeader) -> ReturnCode,
}

/// Why the runtime provider could not be loaded.
#[derive(Debug)]
pub(crate) enum LoadError {
    /// None of the candidate paths could be opened.
    NotFound(Vec<String>),
    /// A library opened but lacked a required export.
    Symbol(&'static str, String),
}

/// Owns the loaded provider module and its resolved function table. The library
/// handle is `None` only for the in-process stub used by unit tests, where the
/// function pointers are Rust functions with static lifetime.
pub(crate) struct Module {
    _library: Option<libloading::Library>,
    pub functions: Functions,
}

// SAFETY: the provider exports are plain C functions with no thread affinity and
// the module handle is immutable once loaded. All mutable FfxApi state lives in
// Context, which the renderer guards with a Mutex.
unsafe impl Send for Module {}
unsafe impl Sync for Module {}

/// Graphics API backend for which to load the `FidelityFX` provider DLL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    /// Vulkan (`amd_fidelityfx_loader_vk.dll` / `.so`).
    Vulkan,
    /// DirectX 12 (`amd_fidelityfx_loader_dx12.dll`, Windows only).
    Dx12,
}

const WINDOWS_VK_NAMES: &[&str] = &["amd_fidelityfx_loader_vk.dll", "amd_fidelityfx_vk.dll"];
const WINDOWS_DX12_NAMES: &[&str] = &["amd_fidelityfx_loader_dx12.dll", "amd_fidelityfx_dx12.dll"];
const UNIX_VK_NAMES: &[&str] = &["libamd_fidelityfx_loader_vk.so", "libamd_fidelityfx_vk.so"];

fn names_for(backend: BackendKind) -> &'static [&'static str] {
    match (cfg!(windows), backend) {
        (true, BackendKind::Vulkan) => WINDOWS_VK_NAMES,
        (true, BackendKind::Dx12) => WINDOWS_DX12_NAMES,
        (false, BackendKind::Vulkan) => UNIX_VK_NAMES,
        (false, BackendKind::Dx12) => &[],
    }
}

/// Ordered search: compile-time/runtime `FSR_DLL`, then executable directory
/// (standalone shipped builds), then `FSR_RUNTIME_DIR`, then `FSR_NATIVE_DIR`,
/// then bare names left to the platform loader.
fn candidate_paths(backend: BackendKind) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let explicit =
        std::env::var_os("FSR_DLL").or_else(|| option_env!("FSR_DLL").map(OsString::from));
    if let Some(path) = explicit {
        out.push(PathBuf::from(path));
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        dirs.push(parent.to_path_buf());
    }
    for dir in [
        std::env::var_os("FSR_RUNTIME_DIR"),
        option_env!("FSR_RUNTIME_DIR").map(OsString::from),
        option_env!("FSR_NATIVE_DIR").map(OsString::from),
    ]
    .into_iter()
    .flatten()
    {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(root) =
        std::env::var_os("FSR_NATIVE_LIBS").or_else(|| std::env::var_os("BARD_NATIVE_LIBS"))
    {
        let target = if cfg!(windows) {
            "x86_64-pc-windows-msvc"
        } else {
            "x86_64-unknown-linux-gnu"
        };
        dirs.push(PathBuf::from(root).join(target).join("ffx"));
    }
    let names = names_for(backend);
    for dir in &dirs {
        for name in names {
            out.push(dir.join(name));
        }
    }
    for name in names {
        out.push(PathBuf::from(name));
    }
    out
}

impl Module {
    pub(crate) fn load_for_backend(backend: BackendKind) -> Result<Self, LoadError> {
        let mut attempted = Vec::new();
        for path in candidate_paths(backend) {
            let library = match unsafe { libloading::Library::new(&path) } {
                Ok(library) => library,
                Err(error) => {
                    attempted.push(format!("{} ({error})", path.display()));
                    continue;
                }
            };
            let functions = match unsafe { resolve(&library) } {
                Ok(functions) => functions,
                Err(error) => {
                    attempted.push(format!("{} ({error:?})", path.display()));
                    continue;
                }
            };
            tracing::info!(
                ?backend,
                path = %path.display(),
                "AMD FidelityFX FSR provider loaded"
            );
            return Ok(Self {
                _library: Some(library),
                functions,
            });
        }
        Err(LoadError::NotFound(attempted))
    }

    /// Build a module from a fixed function table, without loading any DLL.
    /// Used only by unit tests to exercise the `FfxApi` call chain in-process.
    #[cfg(test)]
    pub(crate) fn stub(functions: Functions) -> Self {
        Self {
            _library: None,
            functions,
        }
    }

    /// Safe wrappers keep every `unsafe` `FfxApi` call inside this `#[allow]` module.
    pub(crate) fn query(&self, context: *mut Context, desc: *mut FfxApiHeader) -> ReturnCode {
        // SAFETY: the caller owns the context and keeps the descriptor alive.
        unsafe { (self.functions.query)(context, desc) }
    }

    pub(crate) fn create_context(
        &self,
        context: *mut Context,
        desc: *mut FfxApiHeader,
    ) -> ReturnCode {
        // SAFETY: the descriptor chain is pinned by the caller until destroy;
        // NULL allocator callbacks select the provider's malloc/free.
        unsafe { (self.functions.create_context)(context, desc, std::ptr::null()) }
    }

    pub(crate) fn dispatch(&self, context: *mut Context, desc: *const FfxApiHeader) -> ReturnCode {
        // SAFETY: the context is live and the descriptor outlives the call.
        unsafe { (self.functions.dispatch)(context, desc) }
    }

    pub(crate) fn destroy_context(&self, context: *mut Context) -> ReturnCode {
        // SAFETY: the unique owned context is destroyed once with NULL callbacks.
        unsafe { (self.functions.destroy_context)(context, std::ptr::null()) }
    }

    /// Safe wrapper for `ffxConfigure` (per-frame frame-generation configuration
    /// and the stub-FFI tests). Keeps the `unsafe` call inside this module.
    pub(crate) fn configure(&self, context: *mut Context, desc: *const FfxApiHeader) -> ReturnCode {
        // SAFETY: the context is live and the descriptor outlives the call.
        unsafe { (self.functions.configure)(context, desc) }
    }
}

macro_rules! symbol {
    ($library:expr, $name:literal, $ty:ty) => {{
        let symbol: libloading::Symbol<$ty> = unsafe { $library.get($name.as_bytes()) }
            .map_err(|error| LoadError::Symbol($name, error.to_string()))?;
        *symbol
    }};
}

unsafe fn resolve(library: &libloading::Library) -> Result<Functions, LoadError> {
    Ok(Functions {
        create_context: symbol!(
            library,
            "ffxCreateContext\0",
            unsafe extern "C" fn(
                *mut Context,
                *mut FfxApiHeader,
                *const AllocationCallbacks,
            ) -> ReturnCode
        ),
        destroy_context: symbol!(
            library,
            "ffxDestroyContext\0",
            unsafe extern "C" fn(*mut Context, *const AllocationCallbacks) -> ReturnCode
        ),
        configure: symbol!(
            library,
            "ffxConfigure\0",
            unsafe extern "C" fn(*mut Context, *const FfxApiHeader) -> ReturnCode
        ),
        query: symbol!(
            library,
            "ffxQuery\0",
            unsafe extern "C" fn(*mut Context, *mut FfxApiHeader) -> ReturnCode
        ),
        dispatch: symbol!(
            library,
            "ffxDispatch\0",
            unsafe extern "C" fn(*mut Context, *const FfxApiHeader) -> ReturnCode
        ),
    })
}

/// Opaque, `Send`-able `FfxApi` context handle.
#[derive(Clone, Copy)]
pub(crate) struct Handle(pub Context);

// SAFETY: the FfxApi context has no thread affinity; every access goes through
// `&mut Context`, and the renderer serializes per-camera contexts behind a Mutex.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

/// Owns the create descriptor and its chained backend descriptor. `ffx_api.h`
/// requires every descriptor pointer passed to `ffxCreateContext` to stay live
/// until `ffxDestroyContext`, so both live on a stable heap allocation.
pub(crate) struct PinnedDescriptors {
    pub create: CreateContextDescUpscale,
    pub backend: CreateBackendVkDesc,
}

// SAFETY: the raw `pNext` pointer stays inside this never-relocated allocation;
// all access is serialized through the owning Context.
unsafe impl Send for PinnedDescriptors {}

pub(crate) fn pin_descriptors(
    create: CreateContextDescUpscale,
    backend: CreateBackendVkDesc,
) -> Box<PinnedDescriptors> {
    let mut pinned = Box::new(PinnedDescriptors { create, backend });
    pinned.create.header.next = std::ptr::from_mut(&mut pinned.backend.header);
    pinned
}

/// DX12 counterpart of [`PinnedDescriptors`].
#[cfg(windows)]
pub(crate) struct PinnedDx12Descriptors {
    pub create: CreateContextDescUpscale,
    pub backend: crate::effects::CreateBackendDx12Desc,
}

// SAFETY: `pNext` stays inside this never-relocated heap allocation and all
// access is serialized through the owning Context.
#[cfg(windows)]
unsafe impl Send for PinnedDx12Descriptors {}

#[cfg(windows)]
pub(crate) fn pin_dx12_descriptors(
    create: CreateContextDescUpscale,
    backend: crate::effects::CreateBackendDx12Desc,
) -> Box<PinnedDx12Descriptors> {
    let mut pinned = Box::new(PinnedDx12Descriptors { create, backend });
    pinned.create.header.next = std::ptr::from_mut(&mut pinned.backend.header);
    pinned
}

/// A fixed descriptor chain with sentinel handles, used only by `#[cfg(test)]`
/// stub-provider tests (including [`crate::Context::stub`]).
#[cfg(test)]
pub(crate) fn stub_descriptors() -> Box<PinnedDescriptors> {
    let sentinel = || {
        std::ptr::NonNull::<u8>::dangling()
            .as_ptr()
            .cast::<c_void>()
    };
    let create = CreateContextDescUpscale {
        header: FfxApiHeader::new(CREATE_CONTEXT_DESC_UPSCALE),
        flags: crate::flags::HIGH_DYNAMIC_RANGE,
        max_render_size: Dimensions2D {
            width: 1920,
            height: 1080,
        },
        max_upscale_size: Dimensions2D {
            width: 1920,
            height: 1080,
        },
        message: std::ptr::null_mut(),
    };
    let backend = CreateBackendVkDesc {
        header: FfxApiHeader::new(BACKEND_VK_TYPES[0]),
        device_context: DeviceContextVk {
            device: sentinel(),
            physical_device: sentinel(),
            get_device_proc_addr: sentinel(),
        },
    };
    pin_descriptors(create, backend)
}

#[cfg(test)]
mod tests {
    //! Exercise the whole `FfxApi` call chain in-process, without an AMD DLL.
    //! The stubs have the exact C ABI signatures and inspect the `#[repr(C)]`
    //! descriptors through raw pointers, just as a real provider does.

    use super::*;
    use crate::{Context as FsrContext, QualityMode, flags};
    use std::{ffi::c_void, sync::Arc};

    /// A fake, non-null opaque handle. It is never dereferenced.
    fn sentinel() -> *mut c_void {
        std::ptr::NonNull::<u8>::dangling().as_ptr().cast()
    }

    unsafe extern "C" fn stub_create(
        context: *mut Context,
        desc: *mut FfxApiHeader,
        callbacks: *const AllocationCallbacks,
    ) -> ReturnCode {
        assert!(callbacks.is_null());
        let header = unsafe { &*desc };
        assert_eq!(header.kind, CREATE_CONTEXT_DESC_UPSCALE);
        assert!(!header.next.is_null());
        let backend = header.next.cast::<CreateBackendVkDesc>();
        assert!(BACKEND_VK_TYPES.contains(&unsafe { (*backend).header.kind }));
        let create = desc.cast::<CreateContextDescUpscale>();
        assert_eq!(unsafe { (*create).max_upscale_size.width }, 1920);
        assert_ne!(unsafe { (*create).flags } & flags::HIGH_DYNAMIC_RANGE, 0);
        unsafe { *context = sentinel() };
        0
    }

    unsafe extern "C" fn stub_destroy(
        context: *mut Context,
        callbacks: *const AllocationCallbacks,
    ) -> ReturnCode {
        assert!(callbacks.is_null());
        assert!(!unsafe { *context }.is_null());
        unsafe { *context = std::ptr::null_mut() };
        0
    }

    unsafe extern "C" fn stub_configure(
        _context: *mut Context,
        _desc: *const FfxApiHeader,
    ) -> ReturnCode {
        0
    }

    unsafe extern "C" fn stub_query(context: *mut Context, desc: *mut FfxApiHeader) -> ReturnCode {
        // A real provider dereferences the context for these descriptors; assert
        // that queries are never issued with a null (global) context.
        assert!(!unsafe { *context }.is_null());
        let header = unsafe { &*desc };
        match header.kind {
            QUERY_DESC_UPSCALE_RATIO => {
                let query = desc.cast::<QueryDescUpscaleRatio>();
                let ratio = match unsafe { (*query).quality_mode } {
                    0 => 1.0,
                    1 => 1.5,
                    2 => 1.7,
                    3 => 2.0,
                    4 => 3.0,
                    _ => return 6,
                };
                unsafe { *(*query).out_ratio = ratio };
            }
            QUERY_DESC_RENDER_RESOLUTION => {
                let query = desc.cast::<QueryDescRenderResolution>();
                unsafe {
                    *(*query).out_render_width = (*query).display_width / 2;
                    *(*query).out_render_height = (*query).display_height / 2;
                }
            }
            QUERY_DESC_JITTER_PHASE_COUNT => {
                let query = desc.cast::<QueryDescJitterPhaseCount>();
                unsafe { *(*query).out_phase_count = 8 };
            }
            QUERY_DESC_JITTER_OFFSET => {
                let query = desc.cast::<QueryDescJitterOffset>();
                unsafe {
                    *(*query).out_x = (*query).index as f32 * 0.25;
                    *(*query).out_y = -(*query).index as f32 * 0.25;
                }
            }
            _ => return 2,
        }
        0
    }

    unsafe extern "C" fn stub_dispatch(
        context: *mut Context,
        desc: *const FfxApiHeader,
    ) -> ReturnCode {
        assert!(!unsafe { *context }.is_null());
        let header = unsafe { &*desc };
        assert_eq!(header.kind, DISPATCH_DESC_UPSCALE);
        let dispatch = desc.cast::<DispatchDescUpscale>();
        assert!(!unsafe { (*dispatch).command_list }.is_null());
        0
    }

    fn stub_functions() -> Functions {
        Functions {
            create_context: stub_create,
            destroy_context: stub_destroy,
            configure: stub_configure,
            query: stub_query,
            dispatch: stub_dispatch,
        }
    }

    fn stub_context() -> FsrContext {
        FsrContext::stub(Arc::new(Module::stub(stub_functions())))
    }

    #[test]
    fn queries_round_trip_through_the_ffi_table() {
        let context = stub_context();
        for (mode, ratio) in [
            (QualityMode::NativeAA, 1.0),
            (QualityMode::Quality, 1.5),
            (QualityMode::Balanced, 1.7),
            (QualityMode::Performance, 2.0),
            (QualityMode::UltraPerformance, 3.0),
        ] {
            assert_eq!(context.query_upscale_ratio(mode).unwrap(), ratio);
        }
        assert_eq!(
            context
                .query_render_resolution(QualityMode::Performance, [1920, 1080])
                .unwrap(),
            [960, 540]
        );
        assert_eq!(context.query_jitter_phase_count(960, 1920).unwrap(), 8);
        assert_eq!(context.query_jitter_offset(2, 8).unwrap(), [0.5, -0.5]);
    }

    #[test]
    fn pinned_descriptor_chain_stays_valid_when_retargeted() {
        let mut pinned = super::stub_descriptors();
        let next = pinned.create.header.next;
        assert!(!next.is_null());
        assert_eq!(unsafe { (*next).kind }, BACKEND_VK_TYPES[0]);
        for kind in BACKEND_VK_TYPES {
            pinned.backend.header.kind = kind;
            assert_eq!(unsafe { (*next).kind }, kind);
        }
    }

    #[test]
    fn create_dispatch_and_destroy_forward_through_the_ffi_table() {
        let module = Module::stub(stub_functions());
        let mut handle: Context = std::ptr::null_mut();
        let mut pinned = super::stub_descriptors();
        let header = std::ptr::from_mut(&mut pinned.create.header);
        assert_eq!(module.create_context(&mut handle, header), 0);
        assert!(!handle.is_null());

        let empty = Resource::default();
        let dispatch = DispatchDescUpscale {
            header: FfxApiHeader::new(DISPATCH_DESC_UPSCALE),
            command_list: sentinel(),
            color: empty,
            depth: empty,
            motion_vectors: empty,
            exposure: empty,
            reactive: empty,
            transparency_and_composition: empty,
            output: empty,
            jitter_offset: FloatCoords2D { x: 0.25, y: -0.25 },
            motion_vector_scale: FloatCoords2D {
                x: 1920.0,
                y: 1080.0,
            },
            render_size: Dimensions2D {
                width: 960,
                height: 540,
            },
            upscale_size: Dimensions2D {
                width: 1920,
                height: 1080,
            },
            enable_sharpening: false,
            sharpness: 0.0,
            frame_time_delta: 16.6,
            pre_exposure: 1.0,
            reset: true,
            camera_near: 0.1,
            camera_far: 1000.0,
            camera_fov_angle_vertical: 1.0,
            view_space_to_meters_factor: 1.0,
            flags: 0,
        };
        assert_eq!(
            module.dispatch(&mut handle, std::ptr::from_ref(&dispatch.header).cast_mut()),
            0
        );
        assert_eq!(module.destroy_context(&mut handle), 0);
        assert!(handle.is_null());
    }
}
