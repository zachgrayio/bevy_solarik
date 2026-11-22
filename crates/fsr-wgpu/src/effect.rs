//! Generic `FfxApi` lifecycle for the three SDK 2.4.0 effects beyond temporal
//! upscaling: FSR Ray Regeneration (the denoiser), the neural radiance cache
//! and FSR Frame Generation.
//!
//! [`crate::Context`] owns the upscaler context. [`EffectContext`] is the same
//! create → query → dispatch → destroy lifecycle for the other effects, driven
//! through the *same* runtime-loaded five-function `FfxApi` ABI (see
//! [`crate::native`]). Before this module the [`effects`] descriptor structs were
//! pure ABI *data* with no call path: `ffxCreateContext`/`ffxDispatch` was only
//! ever issued for the upscaler. This closes that gap for `Denoiser`,
//! `RadianceCache` and `FrameGeneration`.
//!
//! Every `FfxApi` descriptor is `#[repr(C)]` and begins with an `FfxApiHeader`,
//! so a single `*FfxApiHeader` selects the operation. The create descriptor and
//! its chained backend descriptor are boxed and pinned for the context's
//! lifetime (`FfxApi` retains pointers into the create chain), exactly like
//! `native::pin_descriptors` does for the upscaler. The caller builds the
//! per-frame dispatch descriptor from the [`effects`] structs and supplies the
//! raw `VkCommandBuffer` / `ID3D12GraphicsCommandList` through
//! [`EffectContext::dispatch_vulkan`] (wgpu encoder) or by setting
//! `command_list` before [`EffectContext::dispatch`].
//!
//! Nothing links against an AMD binary: the provider is loaded at runtime by
//! [`Library`], and the in-process stub-FFI tests below exercise the entire
//! chain without a DLL, GPU, window or wgpu device.

use crate::{Error, Library, effects, native, vulkan};
use std::{ffi::c_void, ptr, sync::Arc};

pub use crate::native::FfxApiHeader;

/// The three SDK 2.4.0 effects that are not the temporal upscaler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    /// FSR Ray Regeneration (`ffx_denoiser.h`) — DX12 / RX 9000+.
    Denoiser,
    /// Neural radiance cache (`ffx_radiancecache.h`) — DX12 / RX 9000+.
    RadianceCache,
    /// FSR Frame Generation (`ffx_framegeneration.h`) — Vulkan or DX12.
    FrameGeneration,
}

impl EffectKind {
    /// Header-verified `(create_desc_type, dispatch_desc_type, version)` for
    /// this effect, from [`effects::desc_types`] and [`effects::versions`].
    pub fn descriptors(self) -> (u64, u64, u32) {
        use effects::{desc_types as d, versions as v};
        match self {
            Self::Denoiser => (d::DENOISER_CREATE, d::DENOISER_DISPATCH, v::DENOISER),
            Self::RadianceCache => (
                d::RADIANCECACHE_CREATE,
                d::RADIANCECACHE_DISPATCH,
                v::RADIANCECACHE,
            ),
            Self::FrameGeneration => (
                d::FRAMEGENERATION_CREATE,
                d::FRAMEGENERATION_DISPATCH,
                v::FRAMEGENERATION,
            ),
        }
    }

    /// Short static label for diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Self::Denoiser => "FSR Ray Regeneration (denoiser)",
            Self::RadianceCache => "FSR radiance cache",
            Self::FrameGeneration => "FSR frame generation",
        }
    }

    /// Map a header-verified `FfxApi` **create** descriptor kind to the effect it
    /// belongs to. Lets a consumer turn a resolved dispatch-chain plan (which
    /// stores descriptor ids) into the [`EffectContext`]s it must create.
    /// Returns `None` for upscaler/swapchain/unknown descriptors, which have
    /// their own lifecycle and are not driven by this module.
    pub fn from_create_desc(kind: u64) -> Option<Self> {
        use effects::desc_types as d;
        match kind {
            d::DENOISER_CREATE => Some(Self::Denoiser),
            d::RADIANCECACHE_CREATE => Some(Self::RadianceCache),
            d::FRAMEGENERATION_CREATE => Some(Self::FrameGeneration),
            _ => None,
        }
    }

    /// Map a header-verified `FfxApi` **dispatch** descriptor kind to its effect.
    pub fn from_dispatch_desc(kind: u64) -> Option<Self> {
        use effects::desc_types as d;
        match kind {
            d::DENOISER_DISPATCH => Some(Self::Denoiser),
            d::RADIANCECACHE_DISPATCH => Some(Self::RadianceCache),
            d::FRAMEGENERATION_DISPATCH => Some(Self::FrameGeneration),
            _ => None,
        }
    }

    /// The create descriptor kind for this effect (round-trips through
    /// [`EffectKind::from_create_desc`]).
    pub fn create_desc(self) -> u64 {
        self.descriptors().0
    }

    /// Whether `kind` is a dispatch descriptor this effect's context accepts.
    /// Frame generation also takes its per-frame prepare descriptors.
    pub fn accepts_dispatch(self, kind: u64) -> bool {
        use effects::desc_types as d;
        kind == self.descriptors().1
            || (self == Self::FrameGeneration
                && matches!(
                    kind,
                    d::FRAMEGENERATION_PREPARE | d::FRAMEGENERATION_PREPARE_V2
                ))
    }
}

/// Creation parameters shared by the three effects. Effect-specific fields are
/// ignored by effects that do not use them.
#[derive(Clone, Copy, Debug)]
pub struct EffectConfig {
    /// Maximum render resolution the context will ever see.
    pub max_render_size: [u32; 2],
    /// Display/presentation resolution (frame generation).
    pub display_size: [u32; 2],
    /// Effect creation flags.
    pub flags: u32,
    /// Denoiser signal-enable mask.
    pub signal_flags: u32,
    /// Denoiser checkerboard signal mask.
    pub checkerboard_signal_flags: u32,
    /// Backbuffer surface format (frame generation).
    pub backbuffer_format: u32,
    /// Radiance cache inference sample cap.
    pub max_inference_sample_count: u32,
    /// Radiance cache training sample cap.
    pub max_training_sample_count: u32,
}

impl Default for EffectConfig {
    fn default() -> Self {
        Self {
            max_render_size: [1920, 1080],
            display_size: [1920, 1080],
            flags: 0,
            signal_flags: 0,
            checkerboard_signal_flags: 0,
            backbuffer_format: 0,
            max_inference_sample_count: 1920 * 1080,
            max_training_sample_count: 1920 * 1080,
        }
    }
}

/// The backend create descriptor chained onto the effect create descriptor.
/// `Vulkan` extracts a `VkDeviceContext` from a wgpu device; `Dx12` carries a
/// raw `ID3D12Device*` supplied by the application's DX12 HAL interop.
pub enum BackendDevice<'a> {
    /// wgpu 30 Vulkan device.
    Vulkan(&'a wgpu::Device),
    /// Raw `ID3D12Device*` (DX12-only effects: RR, radiance cache).
    Dx12(*mut c_void),
}

impl<'a> BackendDevice<'a> {
    /// Detect the active wgpu backend (Vulkan or Windows DX12) and build the
    /// matching [`BackendDevice`].
    pub fn from_wgpu(device: &'a wgpu::Device) -> Result<Self, Error> {
        if vulkan::device_context(device).is_some() {
            return Ok(Self::Vulkan(device));
        }
        #[cfg(windows)]
        if let Some(ptr) = crate::dx12::device_ptr(device) {
            return Ok(Self::Dx12(ptr));
        }
        Err(Error::NoVulkan)
    }
}

/// Put `reads` in the shader-resource state and `writes` in the read-write storage
/// state on `encoder`, so a later raw `FfxApi` dispatch (recorded into its own
/// encoder: wgpu forbids mixing raw and wgpu encoding) finds them where its
/// resource descriptions say (`PIXEL_COMPUTE_READ` / `UNORDERED_ACCESS`).
pub fn transition_for_dispatch(
    encoder: &mut wgpu::CommandEncoder,
    reads: &[&wgpu::TextureView],
    writes: &[&wgpu::TextureView],
) {
    let read = reads.iter().map(|view| wgpu::TextureTransition {
        texture: view.texture(),
        selector: None,
        state: wgpu::TextureUses::RESOURCE,
    });
    let write = writes.iter().map(|view| wgpu::TextureTransition {
        texture: view.texture(),
        selector: None,
        state: wgpu::TextureUses::STORAGE_READ_WRITE,
    });
    encoder.transition_resources(std::iter::empty(), read.chain(write));
}

/// Convert a wgpu texture view into a `FfxApiResource` for the active backend
/// (Vulkan `VkImage` or Windows DX12 `ID3D12Resource*`).
pub fn resource_for_wgpu(view: &wgpu::TextureView, state: u32) -> Result<native::Resource, Error> {
    if let Some(res) = vulkan::resource(view, state) {
        return Ok(res);
    }
    #[cfg(windows)]
    if let Some(res) = crate::dx12::resource(view, state) {
        return Ok(res);
    }
    Err(Error::NoVulkan)
}

enum BackendDesc {
    Vk(native::CreateBackendVkDesc),
    Dx12(effects::CreateBackendDx12Desc),
}

impl BackendDesc {
    fn header_mut(&mut self) -> &mut FfxApiHeader {
        match self {
            Self::Vk(d) => &mut d.header,
            Self::Dx12(d) => &mut d.header,
        }
    }

    fn set_kind(&mut self, kind: u64) {
        self.header_mut().kind = kind;
    }

    /// Candidate backend descriptor kinds, mirroring the upscaler's
    /// cross-SDK-generation retry.
    fn kinds(&self) -> Vec<u64> {
        match self {
            Self::Vk(_) => crate::backend_types(),
            Self::Dx12(_) => vec![effects::desc_types::BACKEND_DX12],
        }
    }
}

/// A pinned effect create-descriptor chain. Kept boxed so the `pNext` pointer
/// from `create` to `backend` stays valid for the context's lifetime.
enum Chain {
    Denoiser {
        create: effects::CreateContextDescDenoiser,
        backend: BackendDesc,
    },
    RadianceCache {
        create: effects::CreateContextDescRadianceCache,
        backend: BackendDesc,
    },
    FrameGeneration {
        create: effects::CreateContextDescFrameGeneration,
        /// Selects the 4.0 API (`PrepareV2`); without it providers assume 3.1.x.
        version: effects::CreateContextDescFrameGenerationVersion,
        backend: BackendDesc,
    },
}

impl Chain {
    fn new(kind: EffectKind, device: BackendDevice<'_>, cfg: &EffectConfig) -> Result<Self, Error> {
        let (create_type, _, version_id) = kind.descriptors();
        let dims = |size: [u32; 2]| native::Dimensions2D {
            width: size[0],
            height: size[1],
        };
        let backend = match device {
            BackendDevice::Vulkan(wgpu_device) => {
                let device_context = vulkan::device_context(wgpu_device).ok_or(Error::NoVulkan)?;
                BackendDesc::Vk(native::CreateBackendVkDesc {
                    header: FfxApiHeader::new(native::BACKEND_VK_TYPES[0]),
                    device_context,
                })
            }
            BackendDevice::Dx12(raw) => BackendDesc::Dx12(effects::CreateBackendDx12Desc {
                header: FfxApiHeader::new(effects::desc_types::BACKEND_DX12),
                device: raw,
            }),
        };
        let header = || FfxApiHeader::new(create_type);
        Ok(match kind {
            EffectKind::Denoiser => Self::Denoiser {
                create: effects::CreateContextDescDenoiser {
                    header: header(),
                    version: version_id,
                    max_render_size: dims(cfg.max_render_size),
                    signal_flags: cfg.signal_flags,
                    checkerboard_signal_flags: cfg.checkerboard_signal_flags,
                    flags: cfg.flags,
                },
                backend,
            },
            EffectKind::RadianceCache => Self::RadianceCache {
                create: effects::CreateContextDescRadianceCache {
                    header: header(),
                    flags: cfg.flags,
                    version: version_id,
                    max_inference_sample_count: cfg.max_inference_sample_count,
                    max_training_sample_count: cfg.max_training_sample_count,
                },
                backend,
            },
            EffectKind::FrameGeneration => Self::FrameGeneration {
                create: effects::CreateContextDescFrameGeneration {
                    header: header(),
                    flags: cfg.flags,
                    display_size: dims(cfg.display_size),
                    max_render_size: dims(cfg.max_render_size),
                    backbuffer_format: cfg.backbuffer_format,
                },
                version: effects::CreateContextDescFrameGenerationVersion {
                    header: FfxApiHeader::new(effects::desc_types::FRAMEGENERATION_VERSION),
                    version: version_id,
                },
                backend,
            },
        })
    }

    fn backend_kinds(&self) -> Vec<u64> {
        match self {
            Self::Denoiser { backend, .. }
            | Self::RadianceCache { backend, .. }
            | Self::FrameGeneration { backend, .. } => backend.kinds(),
        }
    }

    fn set_backend_kind(&mut self, kind: u64) {
        match self {
            Self::Denoiser { backend, .. }
            | Self::RadianceCache { backend, .. }
            | Self::FrameGeneration { backend, .. } => backend.set_kind(kind),
        }
    }

    /// Link `create.header.next` to the backend descriptor and return the
    /// create header pointer for `ffxCreateContext`. Called on the boxed
    /// (never-moved) chain immediately before each FFI attempt.
    fn link(&mut self) -> *mut FfxApiHeader {
        macro_rules! arm {
            ($create:expr, $backend:expr) => {{
                let create: &mut FfxApiHeader = &mut $create.header;
                let backend: &mut FfxApiHeader = $backend.header_mut();
                create.next = ptr::from_mut(backend);
                ptr::from_mut(create)
            }};
        }
        match self {
            Self::Denoiser { create, backend } => arm!(create, backend),
            Self::RadianceCache { create, backend } => arm!(create, backend),
            Self::FrameGeneration {
                create,
                version,
                backend,
            } => {
                let backend: &mut FfxApiHeader = backend.header_mut();
                version.header.next = ptr::from_mut(backend);
                create.header.next = ptr::from_mut(&mut version.header);
                ptr::from_mut(&mut create.header)
            }
        }
    }
}

/// A descriptor that can be passed to [`EffectContext::dispatch_vulkan`].
/// Implemented for the `#[repr(C)]` `FfxApi` dispatch descriptors in [`effects`]
/// whose first field is the header and second field is `command_list`.
pub trait EffectDispatch {
    /// The descriptor's header (start of the struct).
    fn header(&self) -> &FfxApiHeader;
    /// Set the raw `VkCommandBuffer` / `ID3D12GraphicsCommandList` slot.
    fn set_command_list(&mut self, command_list: *mut c_void);
}

macro_rules! impl_effect_dispatch {
    ($($ty:ty),+ $(,)?) => {$(
        impl EffectDispatch for $ty {
            fn header(&self) -> &FfxApiHeader {
                &self.header
            }
            fn set_command_list(&mut self, command_list: *mut c_void) {
                self.command_list = command_list;
            }
        }
    )+};
}

impl_effect_dispatch!(
    effects::DispatchDescDenoiser,
    effects::DispatchDescRadianceCache,
    effects::DispatchDescFrameGeneration,
    effects::DispatchDescFrameGenerationPrepareV2,
);

/// An owned `FfxApi` context for one of [`EffectKind`]. Implements `Drop` so the
/// provider context is always destroyed (mirrors [`crate::Context`]).
pub struct EffectContext {
    library: Arc<native::Module>,
    handle: native::Handle,
    _chain: Box<Chain>,
    kind: EffectKind,
}

// SAFETY: `Chain` stays pinned in a never-relocated heap allocation, `Handle`
// has no thread affinity, and callers serialize access via `&mut EffectContext`.
#[allow(
    unsafe_code,
    reason = "Pinned FfxApi effect descriptor chain and handle are Send (matching Context)"
)]
unsafe impl Send for EffectContext {}

impl EffectContext {
    /// Create an effect context for `kind` on `device` through
    /// `ffxCreateContext`. Retries known backend descriptor kinds across SDK
    /// generations, exactly like the upscaler path.
    pub fn create(
        library: &Library,
        kind: EffectKind,
        device: BackendDevice<'_>,
        cfg: &EffectConfig,
    ) -> Result<Self, Error> {
        for size in [cfg.max_render_size, cfg.display_size] {
            if size[0] == 0
                || size[1] == 0
                || size[0] > u16::MAX as u32
                || size[1] > u16::MAX as u32
            {
                return Err(Error::Dimensions);
            }
        }
        let mut chain = Box::new(Chain::new(kind, device, cfg)?);
        let mut handle: native::Context = ptr::null_mut();
        let mut status = 0u32;
        for backend_kind in chain.backend_kinds() {
            chain.set_backend_kind(backend_kind);
            let header = chain.link();
            status = library.module.create_context(&mut handle, header);
            if status == 0 {
                break;
            }
            // Descriptor/provider mismatches are expected across SDK generations.
            if !matches!(status, 1 | 2 | 4 | 6) {
                return Err(Error::Status(status));
            }
            handle = ptr::null_mut();
        }
        if handle.is_null() {
            return Err(Error::Status(status));
        }
        Ok(Self {
            library: Arc::clone(&library.module),
            handle: native::Handle(handle),
            _chain: chain,
            kind,
        })
    }

    /// The effect this context drives.
    pub fn kind(&self) -> EffectKind {
        self.kind
    }

    /// Issue `ffxQuery` for a caller-built query descriptor chain.
    pub fn query(&mut self, desc: &mut FfxApiHeader) -> Result<(), Error> {
        let mut handle = self.handle.0;
        crate::status_ok(self.library.query(&mut handle, ptr::from_mut(desc)))
    }

    /// Issue `ffxDispatch` for a caller-built dispatch descriptor chain.
    /// Validates that the header kind matches this context's effect before
    /// the FFI call, so an RR context cannot be handed an FG descriptor.
    pub fn dispatch(&mut self, desc: &FfxApiHeader) -> Result<(), Error> {
        self.check_dispatch(desc.kind)?;
        let mut handle = self.handle;
        crate::status_ok(self.library.dispatch(&mut handle.0, ptr::from_ref(desc)))
    }

    /// Issue `ffxConfigure` for a caller-built configure descriptor chain.
    pub fn configure(&mut self, desc: &FfxApiHeader) -> Result<(), Error> {
        let mut handle = self.handle;
        crate::status_ok(self.library.configure(&mut handle.0, ptr::from_ref(desc)))
    }

    /// The provider module and raw context handle, for present-time callbacks
    /// that must call `ffxDispatch` without borrowing this context.
    pub(crate) fn raw_parts(&self) -> (Arc<native::Module>, native::Handle) {
        (Arc::clone(&self.library), self.handle)
    }

    fn check_dispatch(&self, kind: u64) -> Result<(), Error> {
        if self.kind.accepts_dispatch(kind) {
            Ok(())
        } else {
            Err(Error::Descriptor {
                expected: self.kind.descriptors().1,
                found: kind,
            })
        }
    }

    /// Validate `desc`, inject the live `VkCommandBuffer` from a wgpu encoder,
    /// and issue `ffxDispatch` while the encoder is open for HAL interop. This
    /// is the Vulkan render-node entry point; DX12 callers fill `command_list`
    /// themselves and use [`EffectContext::dispatch`].
    pub fn dispatch_vulkan<D: EffectDispatch>(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        desc: &mut D,
    ) -> Result<(), Error> {
        self.check_dispatch(desc.header().kind)?;
        let library = Arc::clone(&self.library);
        let mut handle = self.handle;
        let out = vulkan::with_command_buffer(encoder, |command_list| {
            desc.set_command_list(command_list);
            crate::status_ok(library.dispatch(&mut handle.0, ptr::from_ref(desc.header())))
        });
        out.unwrap_or(Err(Error::NoVulkan))
    }

    /// Validate `desc`, inject the live `VkCommandBuffer` or
    /// `ID3D12GraphicsCommandList*` from a wgpu encoder, and issue `ffxDispatch`.
    pub fn dispatch_wgpu<D: EffectDispatch>(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        desc: &mut D,
    ) -> Result<(), Error> {
        self.check_dispatch(desc.header().kind)?;
        let library = Arc::clone(&self.library);
        let mut handle = self.handle;
        if let Some(res) = vulkan::with_command_buffer(encoder, |command_list| {
            desc.set_command_list(command_list);
            crate::status_ok(library.dispatch(&mut handle.0, ptr::from_ref(desc.header())))
        }) {
            return res;
        }
        #[cfg(windows)]
        if let Some(res) = crate::dx12::with_command_list(encoder, |command_list| {
            desc.set_command_list(command_list);
            crate::status_ok(library.dispatch(&mut handle.0, ptr::from_ref(desc.header())))
        }) {
            return res;
        }
        Err(Error::NoVulkan)
    }
}

impl std::fmt::Debug for EffectContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectContext")
            .field("effect", &self.kind)
            .finish_non_exhaustive()
    }
}

impl Drop for EffectContext {
    fn drop(&mut self) {
        let mut handle = self.handle.0;
        let status = self.library.destroy_context(&mut handle);
        if status != 0 {
            tracing::warn!(
                status,
                effect = self.kind.label(),
                "ffxDestroyContext failed"
            );
        }
    }
}

#[cfg(test)]
#[allow(
    unsafe_code,
    reason = "the in-process stub provider must use the exact unsafe FfxApi C ABI signatures"
)]
mod tests {
    //! CPU-only: drive the full create → query → dispatch → destroy chain for
    //! every effect through the stub `FfxApi` function table. No DLL/GPU/window.
    use super::*;
    use crate::native::{AllocationCallbacks, Context, Functions, Module, ReturnCode};
    use std::cell::RefCell;

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State::new()) };
    }

    struct State {
        kinds: Vec<u64>,
        destroys: u32,
        fail_create: u32,
    }

    impl State {
        const fn new() -> Self {
            Self {
                kinds: Vec::new(),
                destroys: 0,
                fail_create: 0,
            }
        }
    }

    fn reset(fail_create: u32) {
        STATE.with(|s| {
            *s.borrow_mut() = State {
                kinds: Vec::new(),
                destroys: 0,
                fail_create,
            };
        });
    }

    fn kinds() -> Vec<u64> {
        STATE.with(|s| s.borrow().kinds.clone())
    }

    fn destroys() -> u32 {
        STATE.with(|s| s.borrow().destroys)
    }

    unsafe fn record_chain(mut header: *mut FfxApiHeader) {
        while !header.is_null() {
            STATE.with(|s| s.borrow_mut().kinds.push(unsafe { (*header).kind }));
            header = unsafe { (*header).next };
        }
    }

    unsafe extern "C" fn stub_create(
        context: *mut Context,
        desc: *mut FfxApiHeader,
        callbacks: *const AllocationCallbacks,
    ) -> ReturnCode {
        assert!(callbacks.is_null());
        unsafe { record_chain(desc) };
        let fail = STATE.with(|s| s.borrow().fail_create);
        if fail != 0 {
            return fail;
        }
        unsafe { *context = std::ptr::NonNull::<u8>::dangling().as_ptr().cast() };
        0
    }

    unsafe extern "C" fn stub_dispatch(
        _context: *mut Context,
        desc: *const FfxApiHeader,
    ) -> ReturnCode {
        unsafe { record_chain(desc.cast_mut()) };
        0
    }

    unsafe extern "C" fn stub_query(_context: *mut Context, desc: *mut FfxApiHeader) -> ReturnCode {
        unsafe { record_chain(desc) };
        0
    }

    unsafe extern "C" fn stub_destroy(
        _context: *mut Context,
        _callbacks: *const AllocationCallbacks,
    ) -> ReturnCode {
        STATE.with(|s| s.borrow_mut().destroys += 1);
        0
    }

    unsafe extern "C" fn stub_configure(
        _context: *mut Context,
        desc: *const FfxApiHeader,
    ) -> ReturnCode {
        unsafe { record_chain(desc.cast_mut()) };
        0
    }

    fn stub_library(fail_create: u32) -> Library {
        reset(fail_create);
        crate::stub_library(Arc::new(Module::stub(Functions {
            create_context: stub_create,
            destroy_context: stub_destroy,
            configure: stub_configure,
            query: stub_query,
            dispatch: stub_dispatch,
        })))
    }

    fn fake_dx12() -> *mut c_void {
        std::ptr::NonNull::<u8>::dangling().as_ptr().cast()
    }

    #[test]
    fn every_effect_drives_create_query_dispatch_destroy() {
        use effects::{
            DispatchDescDenoiser, DispatchDescFrameGeneration, DispatchDescRadianceCache,
        };
        for kind in [
            EffectKind::Denoiser,
            EffectKind::RadianceCache,
            EffectKind::FrameGeneration,
        ] {
            let (create_type, dispatch_type, _) = kind.descriptors();
            let library = stub_library(0);
            let mut ctx = EffectContext::create(
                &library,
                kind,
                BackendDevice::Dx12(fake_dx12()),
                &EffectConfig::default(),
            )
            .expect("create must succeed against the stub provider");
            assert_eq!(ctx.kind(), kind);
            // The provider saw the effect create descriptor + DX12 backend chain.
            let recorded = kinds();
            assert!(recorded.contains(&create_type), "{kind:?}: {recorded:?}");
            assert!(recorded.contains(&effects::desc_types::BACKEND_DX12));

            // A query descriptor for this context.
            let mut query = FfxApiHeader::new(effects::desc_types::GET_VERSIONS);
            ctx.query(&mut query).unwrap();
            assert!(kinds().contains(&effects::desc_types::GET_VERSIONS));

            // A dispatch descriptor with the correct kind.
            match kind {
                EffectKind::Denoiser => {
                    let mut d: DispatchDescDenoiser = zeroed_denoiser();
                    d.header = FfxApiHeader::new(dispatch_type);
                    ctx.dispatch(&d.header).unwrap();
                }
                EffectKind::RadianceCache => {
                    let mut d: DispatchDescRadianceCache = zeroed_radiance();
                    d.header = FfxApiHeader::new(dispatch_type);
                    ctx.dispatch(&d.header).unwrap();
                }
                EffectKind::FrameGeneration => {
                    let mut d: DispatchDescFrameGeneration = zeroed_fg();
                    d.header = FfxApiHeader::new(dispatch_type);
                    ctx.dispatch(&d.header).unwrap();
                }
            }
            assert!(kinds().contains(&dispatch_type), "{kind:?}");

            drop(ctx);
            assert_eq!(destroys(), 1, "{kind:?} context must destroy once");
        }
    }

    #[test]
    fn dispatch_rejects_a_mismatched_descriptor_kind() {
        let library = stub_library(0);
        let mut ctx = EffectContext::create(
            &library,
            EffectKind::Denoiser,
            BackendDevice::Dx12(fake_dx12()),
            &EffectConfig::default(),
        )
        .unwrap();
        // Hand a frame-generation dispatch kind to a denoiser context.
        let wrong = FfxApiHeader::new(effects::desc_types::FRAMEGENERATION_DISPATCH);
        let err = ctx.dispatch(&wrong).unwrap_err();
        assert!(matches!(
            err,
            Error::Descriptor {
                expected,
                found
            } if expected == effects::desc_types::DENOISER_DISPATCH && found == wrong.kind
        ));
        // No ffxDispatch was issued for the rejected descriptor.
        assert!(!kinds().contains(&effects::desc_types::FRAMEGENERATION_DISPATCH));
    }

    #[test]
    fn create_rejects_invalid_dimensions_before_touching_the_provider() {
        let library = stub_library(0);
        for bad in [[0, 1080], [1920, 0], [u32::MAX, 1080]] {
            let cfg = EffectConfig {
                max_render_size: bad,
                ..Default::default()
            };
            let err = EffectContext::create(
                &library,
                EffectKind::FrameGeneration,
                BackendDevice::Dx12(fake_dx12()),
                &cfg,
            )
            .unwrap_err();
            assert!(matches!(err, Error::Dimensions));
        }
        assert!(kinds().is_empty(), "no FFI call for rejected dimensions");
    }

    #[test]
    fn create_propagates_a_fatal_provider_status_and_does_not_destroy() {
        // Status 3 is not one of the retriable descriptor mismatches.
        let library = stub_library(3);
        let err = EffectContext::create(
            &library,
            EffectKind::FrameGeneration,
            BackendDevice::Dx12(fake_dx12()),
            &EffectConfig::default(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Status(3)));
    }

    #[test]
    fn create_and_dispatch_descriptor_kinds_round_trip_to_the_effect() {
        use effects::desc_types as d;
        for kind in [
            EffectKind::Denoiser,
            EffectKind::RadianceCache,
            EffectKind::FrameGeneration,
        ] {
            let (create, dispatch, _) = kind.descriptors();
            assert_eq!(EffectKind::from_create_desc(create), Some(kind));
            assert_eq!(EffectKind::from_dispatch_desc(dispatch), Some(kind));
            assert_eq!(kind.create_desc(), create);
        }
        // Upscaler and swapchain descriptors are handled elsewhere, not here.
        assert_eq!(EffectKind::from_create_desc(d::UPSCALE_VERSION), None);
        assert_eq!(
            EffectKind::from_dispatch_desc(d::FGSWAPCHAIN_VK_WAIT_FOR_PRESENTS),
            None
        );
        assert_eq!(EffectKind::from_create_desc(0xdead_beef), None);
    }

    #[test]
    fn effect_kind_constants_are_header_verified() {
        assert_eq!(
            EffectKind::Denoiser.descriptors(),
            (
                effects::desc_types::DENOISER_CREATE,
                effects::desc_types::DENOISER_DISPATCH,
                effects::versions::DENOISER
            )
        );
        assert_eq!(
            EffectKind::RadianceCache.descriptors(),
            (
                effects::desc_types::RADIANCECACHE_CREATE,
                effects::desc_types::RADIANCECACHE_DISPATCH,
                effects::versions::RADIANCECACHE
            )
        );
        assert_eq!(
            EffectKind::FrameGeneration.descriptors(),
            (
                effects::desc_types::FRAMEGENERATION_CREATE,
                effects::desc_types::FRAMEGENERATION_DISPATCH,
                effects::versions::FRAMEGENERATION
            )
        );
    }

    // Minimal dispatch descriptors (layouts are size-asserted in effects.rs).
    // Only `header`/`command_list` matter to the FFI chain here. Types come
    // from `native` (privately imported by effects) or `effects` directly.
    fn zeroed_denoiser() -> effects::DispatchDescDenoiser {
        effects::DispatchDescDenoiser {
            header: FfxApiHeader::new(effects::desc_types::DENOISER_DISPATCH),
            command_list: ptr::null_mut(),
            linear_depth: native::Resource::default(),
            motion_vectors: native::Resource::default(),
            normals: native::Resource::default(),
            specular_albedo: native::Resource::default(),
            diffuse_albedo: native::Resource::default(),
            motion_vector_scale: effects::FloatCoords3D::default(),
            jitter_offsets: native::FloatCoords2D::default(),
            camera_position_delta: effects::FloatCoords3D::default(),
            view: effects::Matrix4x4::default(),
            projection: effects::Matrix4x4::default(),
            linear_depth_bounds: effects::FloatBounds::default(),
            render_size: native::Dimensions2D::default(),
            frame_index: 0,
            flags: 0,
        }
    }

    fn zeroed_radiance() -> effects::DispatchDescRadianceCache {
        effects::DispatchDescRadianceCache {
            header: FfxApiHeader::new(effects::desc_types::RADIANCECACHE_DISPATCH),
            command_list: ptr::null_mut(),
            prediction_inputs: native::Resource::default(),
            prediction_outputs: native::Resource::default(),
            train_inputs: native::Resource::default(),
            train_targets: native::Resource::default(),
            sample_counters: native::Resource::default(),
            flags: 0,
            overrides: effects::RadianceCacheOverrides::default(),
        }
    }

    fn zeroed_fg() -> effects::DispatchDescFrameGeneration {
        effects::DispatchDescFrameGeneration {
            header: FfxApiHeader::new(effects::desc_types::FRAMEGENERATION_DISPATCH),
            command_list: ptr::null_mut(),
            present_color: native::Resource::default(),
            outputs: [native::Resource::default(); 4],
            num_generated_frames: 1,
            reset: false,
            backbuffer_transfer_function: 0,
            min_max_luminance: [0.0, 1.0],
            generation_rect: effects::Rect2D::default(),
            frame_id: 0,
        }
    }
}
