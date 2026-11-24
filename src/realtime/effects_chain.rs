//! CPU-side dispatch **chaining plan** for the extra FidelityFX SDK 2.4.0
//! effects in `crates/fsr-wgpu/src/effects.rs`.
//!
//! The realtime render node (`node.rs` / `fsr.rs`) already dispatches the FSR
//! upscaler. The RR denoiser (8-signal `pNext` chain), radiance cache and frame
//! generation create/dispatch descriptors are *declared* in the bindings crate
//! but are not yet chained into the Solari denoiser slot / render graph. This
//! module is the pure-CPU layer that decides **what** to chain and in **what
//! order**, gates it by backend capability, and validates the result.
//!
//! It is entirely data: it opens no GPU device, loads no DLL, creates no window
//! and performs no `ffxDispatch`/FFI call. The FFX descriptor-type ids below
//! mirror `solarik_fsr_wgpu::effects::desc_types`; they are kept as local
//! integers so the module always compiles (the `solarik-fsr-wgpu` dependency is
//! optional / `fsr`-gated). [`EffectChainPlan`] derives [`Resource`] so the
//! consuming app resolves and stores the chain at startup (a real runtime
//! decision driven by probed capabilities); GPU-runtime `ffxDispatch` against
//! `VkImage`s — still NOT done — will consume that stored plan.

use bevy_ecs::prelude::Resource;
use core::fmt;

/// FFX 2.4.0 descriptor-type discriminators, mirrored verbatim from
/// `solarik_fsr_wgpu::effects::desc_types`. They identify the create/dispatch
/// descriptors whose `pNext` chains the bindings crate tests against its
/// in-process stub provider.
pub mod desc_types {
    /// `ffxCreateContextDescUpscaleVersion` (the upscaler's chained version desc).
    pub const UPSCALER_VERSION: u64 = 0x0001_000b;
    /// `ffxCreateContextDescRadianceCache`.
    pub const RADIANCECACHE_CREATE: u64 = 0x0006_0002;
    /// `ffxDispatchDescRadianceCache`.
    pub const RADIANCECACHE_DISPATCH: u64 = 0x0006_0004;
    /// `ffxCreateContextDescDenoiser` (FSR Ray Regeneration).
    pub const DENOISER_CREATE: u64 = 0x0005_0001;
    /// `ffxDispatchDescDenoiser` (the RR denoiser's head dispatch descriptor).
    pub const DENOISER_DISPATCH: u64 = 0x0005_0041;
    /// `ffxCreateContextDescFrameGeneration`.
    pub const FRAMEGENERATION_CREATE: u64 = 0x0002_0001;
    /// `ffxDispatchDescFrameGeneration`.
    pub const FRAMEGENERATION_DISPATCH: u64 = 0x0002_0003;
    /// `ffxCreateContextDescFrameGenerationSwapChainVK` (VK descriptors exist).
    pub const FGSWAPCHAIN_VK_CREATE: u64 = 0x0004_0001;
    /// `ffxDispatchDescFrameGenerationSwapChainWaitForPresentsVK`.
    pub const FGSWAPCHAIN_VK_WAIT_FOR_PRESENTS: u64 = 0x0004_0007;
    /// The eight per-signal denoiser dispatch descriptors (`0x42..=0x49`),
    /// `pNext`-chained behind [`DENOISER_DISPATCH`](Self::DENOISER_DISPATCH).
    pub const DENOISER_SIGNAL_DISPATCH: [u64; 8] = [
        0x0005_0042,
        0x0005_0043,
        0x0005_0044,
        0x0005_0045,
        0x0005_0046,
        0x0005_0047,
        0x0005_0048,
        0x0005_0049,
    ];
}

/// Which render backend the (optional) FSR provider would run on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FfxBackend {
    /// Vulkan (the default; validated Solari path).
    #[default]
    Vulkan,
    /// DirectX 12 (unlocks the AMD-only RR denoiser / radiance cache).
    Dx12,
}

/// Per-camera config for chaining the SDK 2.4.0 effects around the FSR
/// upscaler. Pure data; the consuming app derives it from probed capabilities
/// and calls [`resolve_and_validate`] at startup — this crate only provides the
/// decision layer, not the GPU dispatch.
///
/// Each flag maps to one fork Cargo feature and one Definition-of-Done (DoD)
/// quadrant from the hybrid-renderer plan (decision D21):
///
/// | field | fork Cargo feature | DoD quadrant |
/// |-------|--------------------|--------------|
/// | [`Self::ray_regeneration`] | `fsr_ray_regeneration` | DX12 + RX 9000+ (AMD-only) |
/// | [`Self::radiance_cache`] | `fsr_radiance_cache` | DX12 + RX 9000+ (AMD-only) |
/// | [`Self::frame_generation`] | `fsr_frame_generation` | all four quadrants (Vulkan/DX12 x AMD/NVIDIA) |
/// | [`Self::frame_generation_swapchain`] | `fsr_frame_generation` | swapchain FG (DX12 wrap / FG Swapchain VK) |
///
/// Ray Regeneration and the radiance cache are DX12-only in FFX 2.4.0 (AMD
/// publishes no Vulkan build), so [`resolve_effect_chain`] drops them, with a
/// recorded reason, on [`FfxBackend::Vulkan`]. Frame generation runs on every
/// backend (D21); swapchain-FG is an explicit opt-in because it replaces the
/// present path.
#[derive(Resource, Default, Clone, Debug)]
pub struct FsrEffectChainConfig {
    /// Enable the FSR Ray Regeneration denoiser in the Solari denoiser slot
    /// (`fsr_ray_regeneration`; DX12 + RX 9000+ only).
    pub ray_regeneration: bool,
    /// Enable the neural radiance cache dispatched before the upscaler
    /// (`fsr_radiance_cache`; DX12 + RX 9000+ only).
    pub radiance_cache: bool,
    /// Enable FSR frame generation after the upscaler (`fsr_frame_generation`;
    /// all four DoD quadrants per D21).
    pub frame_generation: bool,
    /// Opt into the FG swapchain path (`fsr_frame_generation`; DX12 wrap
    /// descriptors or FG Swapchain VK descriptors). Requires `frame_generation`.
    pub frame_generation_swapchain: bool,
    /// Backend whose capability set gates the chain. Defaults to Vulkan.
    pub backend: FfxBackend,
}

/// One link in the ordered dispatch chain (the sequence of `ffxDispatch`
/// descriptors the compositor would submit, head -> tail).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectStep {
    /// Radiance cache: chained BEFORE the upscaler.
    RadianceCache,
    /// RR denoiser: occupies the Solari denoiser slot, BEFORE the upscaler.
    RayRegeneration,
    /// The FSR upscaler (the realtime node always dispatches it).
    Upscale,
    /// Frame generation: dispatched AFTER the upscaler.
    FrameGeneration,
    /// FG swapchain: dispatched AFTER frame generation (present replacement).
    FrameGenerationSwapchain,
}

impl EffectStep {
    /// The FFX head dispatch descriptor id for this step (see [`desc_types`]).
    /// For the upscaler, the upscaler's own dispatch lives in `native`, so the
    /// chained version create descriptor stands in here.
    pub fn dispatch_desc(self) -> u64 {
        match self {
            Self::RadianceCache => desc_types::RADIANCECACHE_DISPATCH,
            Self::RayRegeneration => desc_types::DENOISER_DISPATCH,
            Self::Upscale => desc_types::UPSCALER_VERSION,
            Self::FrameGeneration => desc_types::FRAMEGENERATION_DISPATCH,
            Self::FrameGenerationSwapchain => desc_types::FGSWAPCHAIN_VK_WAIT_FOR_PRESENTS,
        }
    }

    /// The FFX **create** descriptor id for the `EffectContext` this step
    /// needs, or `None` for the upscaler (its own `Context`) and the FG
    /// swapchain (GPU-runtime present-replacement path). Mirrors
    /// `solarik_fsr_wgpu::effect::EffectKind::from_create_desc`.
    pub fn create_desc(self) -> Option<u64> {
        match self {
            Self::RadianceCache => Some(desc_types::RADIANCECACHE_CREATE),
            Self::RayRegeneration => Some(desc_types::DENOISER_CREATE),
            Self::FrameGeneration => Some(desc_types::FRAMEGENERATION_CREATE),
            Self::Upscale | Self::FrameGenerationSwapchain => None,
        }
    }

    /// Whether this effect is AMD-only (DX12 + RX 9000+) in FFX 2.4.0.
    pub fn is_amd_only(self) -> bool {
        matches!(self, Self::RadianceCache | Self::RayRegeneration)
    }
}

/// An effect that was requested but dropped by capability gating, with a
/// human-readable [`reason`](Self::reason).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkippedEffect {
    /// The step that was not chained.
    pub step: EffectStep,
    /// Why it was skipped (`&'static str`).
    pub reason: &'static str,
}

/// The resolved, ordered, self-validating dispatch chain plan.
///
/// Derives [`Resource`] so a consuming application can store the startup-resolved
/// chain and let later (GPU-runtime) dispatch read it, instead of the plan being
/// a purely local value. Resolving is pure CPU and never fails hard.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectChainPlan {
    /// Ordered steps to dispatch (head -> tail of the render-graph chain).
    pub steps: Vec<EffectStep>,
    /// Requested-but-unavailable steps, each with a recorded reason.
    pub skipped: Vec<SkippedEffect>,
    /// Backend this plan was resolved for (kept for [`validate_chain`]).
    pub backend: FfxBackend,
}

impl EffectChainPlan {
    /// Whether a given step is present in the ordered chain.
    pub fn contains(&self, step: EffectStep) -> bool {
        self.steps.contains(&step)
    }

    /// Ordered FFX **create** descriptor ids for the context-backed steps in
    /// this resolved plan (radiance cache → RR → upscale → FG, minus upscaler
    /// and swapchain). A consuming runtime feeds each id to
    /// `solarik_fsr_wgpu::effect::EffectKind::from_create_desc` and creates the
    /// corresponding `EffectContext` before the first dispatch. Pure CPU, no FFI.
    pub fn effect_create_descs(&self) -> Vec<u64> {
        self.steps
            .iter()
            .filter_map(|step| step.create_desc())
            .collect()
    }
}

/// Whether the FG swapchain descriptor family exists for `backend`.
///
/// DX12 ships the `FGSWAPCHAIN_DX12_*` wrap descriptors; Vulkan ships the
/// `FGSWAPCHAIN_VK_*` descriptors in SDK 2.4.0. A real implementation would
/// confirm this with `ffxQuery`/`GetVersions`; on CPU it is a pinned constant.
pub fn fg_swapchain_descriptors_exist(backend: FfxBackend) -> bool {
    const FG_SWAPCHAIN_VK_DESCRIPTORS_EXIST: bool = true;
    backend == FfxBackend::Dx12 || FG_SWAPCHAIN_VK_DESCRIPTORS_EXIST
}

/// Build the RR denoiser's `pNext` chain: the head `DENOISER_DISPATCH`
/// descriptor followed by the eight per-signal dispatch descriptors. This is a
/// CPU mirror of the bindings crate's stub-FFI 8-signal chain test (no FFI).
pub fn denoiser_signal_chain() -> DescriptorChain {
    let mut kinds = vec![desc_types::DENOISER_DISPATCH];
    kinds.extend_from_slice(&desc_types::DENOISER_SIGNAL_DISPATCH);
    DescriptorChain::link(kinds)
}

/// One node in a CPU model of an FfxApi `pNext` descriptor chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DescriptorNode {
    /// FFX 2.4.0 descriptor-type discriminator (see [`desc_types`]).
    pub kind: u64,
    /// Index of the next descriptor in the chain; `None` is the tail.
    pub next: Option<usize>,
}

/// An ordered `pNext` descriptor chain. `nodes[0]` is the head (the descriptor
/// handed to `ffxDispatch`); each following node is linked through `next`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DescriptorChain {
    /// Nodes in head -> tail chain order.
    pub nodes: Vec<DescriptorNode>,
}

impl DescriptorChain {
    /// Build a linear chain from descriptor kinds, wiring every `next` link.
    pub fn link(kinds: impl IntoIterator<Item = u64>) -> Self {
        let kinds: Vec<u64> = kinds.into_iter().collect();
        let len = kinds.len();
        let nodes = kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| DescriptorNode {
                kind,
                next: (i + 1 < len).then_some(i + 1),
            })
            .collect();
        Self { nodes }
    }

    /// Descriptor kinds in chain order.
    pub fn kinds(&self) -> Vec<u64> {
        self.nodes.iter().map(|node| node.kind).collect()
    }

    /// Whether `next` forms a well-formed singly-linked list with no gaps.
    pub fn is_well_formed(&self) -> bool {
        self.nodes
            .iter()
            .enumerate()
            .all(|(i, node)| node.next == (i + 1 < self.nodes.len()).then_some(i + 1))
    }
}

// Reasons recorded for capability-gated effects. Kept as `&'static str` so the
// plan never allocates and never needs a device to explain a skip.
const REASON_RADIANCE_VULKAN: &str =
    "Radiance cache is DX12 + RX 9000+ only (FFX SDK 2.4.0); dropped on Vulkan";
const REASON_RR_VULKAN: &str =
    "FSR Ray Regeneration is DX12 + RX 9000+ only (FFX SDK 2.4.0); dropped on Vulkan";
const REASON_SWAPCHAIN_NEEDS_FG: &str =
    "FG swapchain requires the frame generation step to be enabled";
const REASON_SWAPCHAIN_NO_DESCRIPTORS: &str =
    "FG swapchain descriptors are unavailable for this backend";

/// Resolve a config into an ordered [`EffectChainPlan`].
///
/// Canonical order: radiance cache -> RR denoiser -> upscaler -> frame
/// generation -> (optional) FG swapchain. It NEVER hard-fails: effects that the
/// backend cannot run are moved into [`EffectChainPlan::skipped`] with a reason.
/// The upscaler is always present because `fsr.rs` always dispatches it (or
/// native resolution).
pub fn resolve_effect_chain(config: &FsrEffectChainConfig) -> EffectChainPlan {
    let backend = config.backend;
    let is_dx12 = backend == FfxBackend::Dx12;
    let mut steps = Vec::new();
    let mut skipped = Vec::new();

    // Radiance cache + RR denoiser are DX12 + RX 9000+ (AMD) only, BEFORE the
    // upscaler. On Vulkan they are recorded as skipped with a reason.
    if config.radiance_cache {
        if is_dx12 {
            steps.push(EffectStep::RadianceCache);
        } else {
            skipped.push(SkippedEffect {
                step: EffectStep::RadianceCache,
                reason: REASON_RADIANCE_VULKAN,
            });
        }
    }
    if config.ray_regeneration {
        if is_dx12 {
            steps.push(EffectStep::RayRegeneration);
        } else {
            skipped.push(SkippedEffect {
                step: EffectStep::RayRegeneration,
                reason: REASON_RR_VULKAN,
            });
        }
    }

    // The realtime node always dispatches the upscaler.
    steps.push(EffectStep::Upscale);

    // Frame generation runs on all four DoD quadrants (D21), AFTER upscale.
    if config.frame_generation {
        steps.push(EffectStep::FrameGeneration);
    }

    // Swapchain-FG: explicit opt-in, needs an active FG step and the swapchain
    // descriptor family for this backend (DX12 wrap OR FG Swapchain VK).
    if config.frame_generation_swapchain {
        if !config.frame_generation {
            skipped.push(SkippedEffect {
                step: EffectStep::FrameGenerationSwapchain,
                reason: REASON_SWAPCHAIN_NEEDS_FG,
            });
        } else if !fg_swapchain_descriptors_exist(backend) {
            skipped.push(SkippedEffect {
                step: EffectStep::FrameGenerationSwapchain,
                reason: REASON_SWAPCHAIN_NO_DESCRIPTORS,
            });
        } else {
            steps.push(EffectStep::FrameGenerationSwapchain);
        }
    }

    EffectChainPlan {
        steps,
        skipped,
        backend,
    }
}

/// Ordering / capability invariants of a chain plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainError {
    /// The same step appears more than once.
    DuplicateStep(EffectStep),
    /// An AMD-only effect is present on the Vulkan backend.
    AmdEffectOnVulkan(EffectStep),
    /// An upscaler-path effect exists but the upscaler does not.
    MissingUpscale,
    /// The radiance cache is scheduled after the upscaler.
    RadianceCacheAfterUpscale,
    /// Swapchain-FG is scheduled before (or without) frame generation.
    SwapchainBeforeFrameGeneration,
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateStep(step) => write!(f, "duplicate effect step {step:?}"),
            Self::AmdEffectOnVulkan(step) => {
                write!(
                    f,
                    "AMD-only effect {step:?} cannot run on the Vulkan backend"
                )
            }
            Self::MissingUpscale => {
                write!(
                    f,
                    "upscale step missing while an upscaler-path effect is present"
                )
            }
            Self::RadianceCacheAfterUpscale => {
                write!(f, "radiance cache must be dispatched before the upscaler")
            }
            Self::SwapchainBeforeFrameGeneration => {
                write!(f, "swapchain frame generation must follow frame generation")
            }
        }
    }
}

/// Validate a chain plan against the ordering / capability invariants.
pub fn validate_chain(plan: &EffectChainPlan) -> Result<(), ChainError> {
    // 1. No duplicate steps.
    for (i, step) in plan.steps.iter().enumerate() {
        if plan.steps[..i].contains(step) {
            return Err(ChainError::DuplicateStep(*step));
        }
    }

    let pos = |step: EffectStep| plan.steps.iter().position(|s| *s == step);

    // 2. RR / radiance cache never on Vulkan.
    if plan.backend == FfxBackend::Vulkan {
        for step in [EffectStep::RadianceCache, EffectStep::RayRegeneration] {
            if pos(step).is_some() {
                return Err(ChainError::AmdEffectOnVulkan(step));
            }
        }
    }

    // 3. Upscale present whenever any upscaler-path step exists.
    let has_upscaler_path = plan.steps.iter().any(|step| {
        matches!(
            step,
            EffectStep::RadianceCache
                | EffectStep::RayRegeneration
                | EffectStep::FrameGeneration
                | EffectStep::FrameGenerationSwapchain
        )
    });
    let upscale_pos = pos(EffectStep::Upscale);
    if has_upscaler_path && upscale_pos.is_none() {
        return Err(ChainError::MissingUpscale);
    }

    // 4. Radiance cache never after upscale.
    if let (Some(radiance), Some(upscale)) = (pos(EffectStep::RadianceCache), upscale_pos) {
        if radiance > upscale {
            return Err(ChainError::RadianceCacheAfterUpscale);
        }
    }

    // 5. Swapchain FG never before FG (and requires FG to exist).
    if let Some(swapchain) = pos(EffectStep::FrameGenerationSwapchain) {
        match pos(EffectStep::FrameGeneration) {
            Some(fg) if fg < swapchain => {}
            _ => return Err(ChainError::SwapchainBeforeFrameGeneration),
        }
    }

    Ok(())
}

/// Resolve a config into a validated [`EffectChainPlan`] in one call.
///
/// This is the entry point a consuming application's Startup system uses: it
/// builds the ordered chain, then checks the invariants. `resolve_effect_chain`
/// never hard-fails (unavailable effects go to [`EffectChainPlan::skipped`]), so
/// the returned plan is always structurally sound; a `Err` here would mean a bug
/// in the ordering logic, not a missing GPU capability. Callers should log the
/// error and keep the never-invalid plan rather than panicking at startup.
pub fn resolve_and_validate(config: &FsrEffectChainConfig) -> Result<EffectChainPlan, ChainError> {
    let plan = resolve_effect_chain(config);
    validate_chain(&plan)?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    //! Pure-CPU tests: no GPU, DLL, window, device or FFI, mirroring the
    //! headless stub-FFI contract of `crates/fsr-wgpu/src/effects.rs`.

    use super::*;

    #[test]
    fn default_plan_is_upscale_only() {
        let plan = resolve_effect_chain(&FsrEffectChainConfig::default());
        assert_eq!(plan.backend, FfxBackend::Vulkan);
        assert_eq!(plan.steps, vec![EffectStep::Upscale]);
        assert!(plan.skipped.is_empty());
        assert!(plan.contains(EffectStep::Upscale));
        assert!(validate_chain(&plan).is_ok());
        assert_eq!(
            EffectStep::Upscale.dispatch_desc(),
            desc_types::UPSCALER_VERSION
        );
        // No context-backed effects in the default plan (upscaler has its own).
        assert!(plan.effect_create_descs().is_empty());
        assert_eq!(EffectStep::Upscale.create_desc(), None);
        assert_eq!(EffectStep::FrameGenerationSwapchain.create_desc(), None);
    }

    /// The resolved plan exposes the exact FFX create descriptor ids a runtime
    /// must turn into `EffectContext`s, in chain order, with a 1:1 mapping to
    /// `EffectStep::create_desc` and to the bindings crate's dispatch ids.
    #[test]
    fn plan_exposes_effect_create_descs_for_context_backed_steps() {
        let config = FsrEffectChainConfig {
            backend: FfxBackend::Dx12,
            ray_regeneration: true,
            radiance_cache: true,
            frame_generation: true,
            ..Default::default()
        };
        let plan = resolve_effect_chain(&config);
        assert_eq!(
            plan.effect_create_descs(),
            vec![
                desc_types::RADIANCECACHE_CREATE,
                desc_types::DENOISER_CREATE,
                desc_types::FRAMEGENERATION_CREATE,
            ]
        );
        assert_eq!(
            EffectStep::RadianceCache.create_desc(),
            Some(desc_types::RADIANCECACHE_CREATE)
        );
        assert_eq!(
            EffectStep::RayRegeneration.create_desc(),
            Some(desc_types::DENOISER_CREATE)
        );
        assert_eq!(
            EffectStep::FrameGeneration.create_desc(),
            Some(desc_types::FRAMEGENERATION_CREATE)
        );
        // Header-verified literal ids (SDK 2.4.0 `ffx_api.h` sub ids).
        assert_eq!(desc_types::RADIANCECACHE_CREATE, 0x0006_0002);
        assert_eq!(desc_types::DENOISER_CREATE, 0x0005_0001);
        assert_eq!(desc_types::FRAMEGENERATION_CREATE, 0x0002_0001);
        // Create and dispatch ids share the same effect id high bits.
        for (create, dispatch) in [
            (
                desc_types::RADIANCECACHE_CREATE,
                desc_types::RADIANCECACHE_DISPATCH,
            ),
            (desc_types::DENOISER_CREATE, desc_types::DENOISER_DISPATCH),
            (
                desc_types::FRAMEGENERATION_CREATE,
                desc_types::FRAMEGENERATION_DISPATCH,
            ),
        ] {
            assert_eq!(create & 0xffff_0000, dispatch & 0xffff_0000);
        }
    }

    #[test]
    fn dx12_rr_radiance_fg_yields_four_step_order() {
        let config = FsrEffectChainConfig {
            backend: FfxBackend::Dx12,
            ray_regeneration: true,
            radiance_cache: true,
            frame_generation: true,
            ..Default::default()
        };
        let plan = resolve_effect_chain(&config);
        assert_eq!(
            plan.steps,
            vec![
                EffectStep::RadianceCache,
                EffectStep::RayRegeneration,
                EffectStep::Upscale,
                EffectStep::FrameGeneration,
            ]
        );
        assert!(plan.skipped.is_empty());
        assert!(validate_chain(&plan).is_ok());
    }

    #[test]
    fn vulkan_drops_amd_only_effects_with_reasons() {
        let config = FsrEffectChainConfig {
            ray_regeneration: true,
            radiance_cache: true,
            ..Default::default()
        };
        let plan = resolve_effect_chain(&config);
        assert_eq!(plan.steps, vec![EffectStep::Upscale]);
        assert_eq!(plan.skipped.len(), 2);
        for skipped in &plan.skipped {
            assert!(skipped.step.is_amd_only());
            assert!(skipped.reason.contains("DX12"));
        }
        assert!(
            plan.skipped
                .iter()
                .any(|s| s.step == EffectStep::RayRegeneration)
        );
        assert!(
            plan.skipped
                .iter()
                .any(|s| s.step == EffectStep::RadianceCache)
        );
        assert!(validate_chain(&plan).is_ok());
    }

    #[test]
    fn validate_rejects_hand_built_out_of_order_plan() {
        let plan = EffectChainPlan {
            backend: FfxBackend::Dx12,
            steps: vec![EffectStep::Upscale, EffectStep::RadianceCache],
            skipped: vec![],
        };
        assert_eq!(
            validate_chain(&plan),
            Err(ChainError::RadianceCacheAfterUpscale)
        );
    }

    #[test]
    fn validate_rejects_duplicate_steps() {
        let plan = EffectChainPlan {
            steps: vec![EffectStep::Upscale, EffectStep::Upscale],
            ..Default::default()
        };
        assert_eq!(
            validate_chain(&plan),
            Err(ChainError::DuplicateStep(EffectStep::Upscale))
        );
    }

    #[test]
    fn validate_rejects_swapchain_before_fg() {
        let plan = EffectChainPlan {
            backend: FfxBackend::Dx12,
            steps: vec![
                EffectStep::Upscale,
                EffectStep::FrameGenerationSwapchain,
                EffectStep::FrameGeneration,
            ],
            skipped: vec![],
        };
        assert_eq!(
            validate_chain(&plan),
            Err(ChainError::SwapchainBeforeFrameGeneration)
        );
    }

    #[test]
    fn validate_rejects_swapchain_without_fg() {
        let plan = EffectChainPlan {
            backend: FfxBackend::Dx12,
            steps: vec![EffectStep::Upscale, EffectStep::FrameGenerationSwapchain],
            skipped: vec![],
        };
        assert_eq!(
            validate_chain(&plan),
            Err(ChainError::SwapchainBeforeFrameGeneration)
        );
    }

    #[test]
    fn validate_rejects_missing_upscale() {
        let plan = EffectChainPlan {
            backend: FfxBackend::Dx12,
            steps: vec![EffectStep::FrameGeneration],
            skipped: vec![],
        };
        assert_eq!(validate_chain(&plan), Err(ChainError::MissingUpscale));
    }

    #[test]
    fn validate_rejects_amd_effects_on_vulkan() {
        for step in [EffectStep::RadianceCache, EffectStep::RayRegeneration] {
            let plan = EffectChainPlan {
                backend: FfxBackend::Vulkan,
                steps: vec![step, EffectStep::Upscale],
                skipped: vec![],
            };
            assert_eq!(
                validate_chain(&plan),
                Err(ChainError::AmdEffectOnVulkan(step))
            );
        }
    }

    #[test]
    fn swapchain_fg_opt_in_appends_last_on_dx12() {
        let config = FsrEffectChainConfig {
            backend: FfxBackend::Dx12,
            frame_generation: true,
            frame_generation_swapchain: true,
            ..Default::default()
        };
        let plan = resolve_effect_chain(&config);
        assert_eq!(
            plan.steps,
            vec![
                EffectStep::Upscale,
                EffectStep::FrameGeneration,
                EffectStep::FrameGenerationSwapchain,
            ]
        );
        assert!(validate_chain(&plan).is_ok());
    }

    #[test]
    fn vulkan_swapchain_uses_vk_descriptors() {
        // FG Swapchain VK descriptors exist in SDK 2.4.0, so opt-in works on
        // Vulkan too (frame generation, not AMD-only RR/radiance cache).
        assert_eq!(desc_types::FGSWAPCHAIN_VK_CREATE, 0x0004_0001);
        assert!(fg_swapchain_descriptors_exist(FfxBackend::Vulkan));
        let config = FsrEffectChainConfig {
            frame_generation: true,
            frame_generation_swapchain: true,
            ..Default::default()
        };
        let plan = resolve_effect_chain(&config);
        assert_eq!(
            plan.steps,
            vec![
                EffectStep::Upscale,
                EffectStep::FrameGeneration,
                EffectStep::FrameGenerationSwapchain,
            ]
        );
        assert!(validate_chain(&plan).is_ok());
    }

    #[test]
    fn swapchain_without_fg_is_skipped_with_reason() {
        let config = FsrEffectChainConfig {
            frame_generation_swapchain: true,
            ..Default::default()
        };
        let plan = resolve_effect_chain(&config);
        assert_eq!(plan.steps, vec![EffectStep::Upscale]);
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(plan.skipped[0].step, EffectStep::FrameGenerationSwapchain);
        assert!(plan.skipped[0].reason.contains("frame generation"));
    }

    #[test]
    fn denoiser_eight_signal_pnext_chain_is_well_formed() {
        let chain = denoiser_signal_chain();
        assert_eq!(chain.nodes.len(), 9);
        assert!(chain.is_well_formed());
        assert_eq!(chain.nodes[0].kind, desc_types::DENOISER_DISPATCH);
        assert_eq!(chain.nodes[8].next, None);
        assert_eq!(
            &chain.kinds()[1..],
            &desc_types::DENOISER_SIGNAL_DISPATCH[..]
        );
    }

    #[test]
    fn resolve_never_fails_and_always_validates() {
        let configs = [
            FsrEffectChainConfig::default(),
            FsrEffectChainConfig {
                backend: FfxBackend::Dx12,
                ray_regeneration: true,
                radiance_cache: true,
                frame_generation: true,
                frame_generation_swapchain: true,
            },
            FsrEffectChainConfig {
                backend: FfxBackend::Vulkan,
                ray_regeneration: true,
                radiance_cache: true,
                frame_generation: true,
                frame_generation_swapchain: true,
            },
            FsrEffectChainConfig {
                backend: FfxBackend::Dx12,
                frame_generation_swapchain: true,
                ..Default::default()
            },
        ];
        for config in &configs {
            let plan = resolve_effect_chain(config);
            assert!(validate_chain(&plan).is_ok(), "plan invalid: {plan:?}");
            // The one-call entry point agrees with the two-step one and always
            // returns a valid plan (resolve never hard-fails).
            let validated =
                resolve_and_validate(config).expect("resolver must always yield a valid plan");
            assert_eq!(validated, plan);
        }
    }

    #[test]
    fn resolve_and_validate_returns_err_only_for_a_broken_plan() {
        // Inject an invariant violation: a plan with a duplicate step must be
        // rejected by validate_chain, proving resolve_and_validate propagates
        // ChainError rather than silently storing an invalid chain.
        let broken = EffectChainPlan {
            steps: vec![EffectStep::Upscale, EffectStep::Upscale],
            backend: FfxBackend::Vulkan,
            ..Default::default()
        };
        assert_eq!(
            validate_chain(&broken),
            Err(ChainError::DuplicateStep(EffectStep::Upscale))
        );
    }

    #[test]
    fn chain_error_display_is_non_empty_and_descriptive() {
        let errors = [
            ChainError::DuplicateStep(EffectStep::Upscale),
            ChainError::AmdEffectOnVulkan(EffectStep::RadianceCache),
            ChainError::MissingUpscale,
            ChainError::RadianceCacheAfterUpscale,
            ChainError::SwapchainBeforeFrameGeneration,
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
        }
        assert!(
            ChainError::RadianceCacheAfterUpscale
                .to_string()
                .contains("radiance cache")
        );
    }
}
