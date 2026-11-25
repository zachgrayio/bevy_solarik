//! Application-scoped ownership for shared D3D12 NGX/Streamline runtimes.
//!
//! The outer runner outlives the complete App. SDK leases additionally outlive every feature
//! and parameter destructor. A weak registry is bookkeeping only: no GPU or native SDK leaks.
use bevy_app::{App, AppExit};

/// Run a graphics application with its shared native runtime outside the complete App lifetime.
///
/// Pass the builder, not an already-running App: initialization failures also unwind through
/// this owner. Retained native SDK feature leases can safely extend it further. Applications
/// must not detach unleased renderer/device work beyond their App's own shutdown barrier.
pub fn run_app(build: impl FnOnce() -> App) -> AppExit {
    #[cfg(all(
        windows,
        any(feature = "dx12_ngx_init", feature = "dlss_frame_generation")
    ))]
    let session = runtime::registry().acquire(runtime::RuntimeSession::default);
    let mut app = build();
    tracing::info!("Native graphics session: entering runner");
    let exit = app.run();
    tracing::info!(?exit, "Native graphics session: runner returned");
    drop(app);
    tracing::info!("Native graphics session: App released");
    #[cfg(all(
        windows,
        any(feature = "dx12_ngx_init", feature = "dlss_frame_generation")
    ))]
    drop(session);
    exit
}

#[cfg(any(
    test,
    all(
        windows,
        any(feature = "dx12_ngx_init", feature = "dlss_frame_generation")
    )
))]
mod lifetime {
    use std::sync::{Arc, Condvar, Mutex, Weak};

    struct State<T> {
        active: Weak<T>,
        closing: Option<std::thread::ThreadId>,
    }

    pub(super) struct Registry<T> {
        state: Mutex<State<T>>,
        changed: Condvar,
    }
    impl<T> Registry<T> {
        pub(super) fn new() -> Self {
            Self {
                state: Mutex::new(State {
                    active: Weak::new(),
                    closing: None,
                }),
                changed: Condvar::new(),
            }
        }
        pub(super) fn acquire_existing(self: &Arc<Self>) -> Option<Arc<Lease<T>>> {
            let mut state = self.state.lock().unwrap();
            while let Some(owner) = state.closing {
                assert_ne!(
                    owner,
                    std::thread::current().id(),
                    "Cannot retain a graphics session during its own shutdown"
                );
                state = self.changed.wait(state).unwrap();
            }
            state.active.upgrade().map(|inner| {
                Arc::new(Lease {
                    registry: Arc::clone(self),
                    inner: Some(inner),
                })
            })
        }
        pub(super) fn acquire(self: &Arc<Self>, create: impl FnOnce() -> T) -> Arc<Lease<T>> {
            self.acquire_observing(create, || {})
        }
        pub(super) fn acquire_observing(
            self: &Arc<Self>,
            create: impl FnOnce() -> T,
            mut before_wait: impl FnMut(),
        ) -> Arc<Lease<T>> {
            let mut state = self.state.lock().unwrap();
            while let Some(owner) = state.closing {
                assert_ne!(
                    owner,
                    std::thread::current().id(),
                    "Cannot acquire a new graphics session from its own shutdown callback"
                );
                before_wait();
                state = self.changed.wait(state).unwrap();
            }
            let inner = state.active.upgrade().unwrap_or_else(|| {
                let inner = Arc::new(create());
                state.active = Arc::downgrade(&inner);
                inner
            });
            Arc::new(Lease {
                registry: Arc::clone(self),
                inner: Some(inner),
            })
        }
    }

    pub(super) struct Lease<T> {
        registry: Arc<Registry<T>>,
        inner: Option<Arc<T>>,
    }
    impl<T> Lease<T> {
        pub(super) fn get(&self) -> &T {
            self.inner.as_deref().expect("Live graphics lease")
        }
    }
    impl<T> Drop for Lease<T> {
        fn drop(&mut self) {
            let inner = self.inner.take().expect("Graphics lease released once");
            let mut state = self.registry.state.lock().unwrap();
            // Decide while our final strong reference still exists. A weak-only registry
            // without this closing barrier could recreate a runtime concurrently with Drop.
            let last = Arc::strong_count(&inner) == 1;
            if !last {
                // Decrement under the same lock as acquire/other releases. Otherwise two
                // concurrent non-last droppers could both see count=2 and bypass Closing.
                drop(inner);
                drop(state);
                return;
            }
            state.closing = Some(std::thread::current().id());
            state.active = Weak::new();
            drop(state);
            struct Finish<'a, T> {
                registry: &'a Registry<T>,
                last: bool,
            }
            impl<T> Drop for Finish<'_, T> {
                fn drop(&mut self) {
                    if self.last {
                        let mut state = self.registry.state.lock().unwrap();
                        state.closing = None;
                        self.registry.changed.notify_all();
                    }
                }
            }
            let _finish = Finish {
                registry: &self.registry,
                last,
            };
            // Native cleanup never executes under the registry mutex or an SDK mutex.
            drop(inner);
        }
    }

    pub(super) fn needs_gpu(direct_ngx: bool, streamline: bool) -> bool {
        direct_ngx || streamline
    }

    pub(super) fn shutdown_in_order<G, L>(
        gpu: G,
        stop_provider: impl FnOnce() -> L,
        stop_ngx: impl FnOnce(),
    ) {
        let code = stop_provider();
        stop_ngx();
        tracing::info!("Native graphics session: final GPU release begin");
        drop(gpu);
        tracing::info!("Native graphics session: final GPU release end; core release begin");
        // Only core/interposer code remains loaded through queue/device destruction;
        // feature/model DLLs must detach in SDK shutdown while the GPU is alive.
        drop(code);
        tracing::info!("Native graphics session: core release end");
    }
}

#[cfg(all(
    windows,
    any(feature = "dx12_ngx_init", feature = "dlss_frame_generation")
))]
mod runtime {
    use super::lifetime::{Registry, needs_gpu, shutdown_in_order};
    use bevy_app::{App, Plugin};
    use bevy_render::{
        RenderApp,
        renderer::{RenderDevice, RenderQueue},
    };
    use std::sync::{Arc, Mutex, OnceLock};

    struct GpuKeepalive {
        queue: Option<RenderQueue>,
        device: Option<RenderDevice>,
    }
    impl Drop for GpuKeepalive {
        fn drop(&mut self) {
            drop(self.queue.take());
            drop(self.device.take());
        }
    }
    struct RuntimeCode {
        #[cfg(feature = "dlss_frame_generation")]
        streamline: Option<solarik_streamline_wgpu::RetiredRuntime>,
    }
    impl RuntimeCode {
        fn shutdown_provider() -> Self {
            // SDKs own feature/model DLL references and release them during native
            // shutdown while the queue/device keepalive still exists. Retain only
            // Streamline's core interposer past GPU destruction.
            #[cfg(feature = "dlss_frame_generation")]
            let streamline = solarik_streamline_wgpu::shutdown_retaining_runtime();
            Self {
                #[cfg(feature = "dlss_frame_generation")]
                streamline,
            }
        }
    }
    impl Drop for RuntimeCode {
        fn drop(&mut self) {
            #[cfg(feature = "dlss_frame_generation")]
            drop(self.streamline.take());
        }
    }
    #[derive(Default)]
    pub(super) struct RuntimeSession {
        gpu: Mutex<Option<GpuKeepalive>>,
    }
    impl RuntimeSession {
        fn retain_gpu(&self, device: RenderDevice, queue: RenderQueue) {
            let mut gpu = self.gpu.lock().unwrap();
            if let Some(existing) = &*gpu {
                assert_eq!(
                    existing.device.as_ref().expect("Live device").wgpu_device(),
                    device.wgpu_device(),
                    "One shared NGX session cannot own different live devices"
                );
            } else {
                *gpu = Some(GpuKeepalive {
                    queue: Some(queue),
                    device: Some(device),
                });
            }
        }
    }
    impl Drop for RuntimeSession {
        fn drop(&mut self) {
            tracing::info!("Native graphics session: owner cleanup begin");
            let gpu = self.gpu.get_mut().unwrap().take();
            // Always retire Streamline, including successful preinit followed by a failed
            // App/device build. No attached runtime or direct NGX readiness is required.
            if let Some(gpu) = &gpu {
                tracing::info!("Native graphics session: tracked GPU wait begin");
                if let Err(error) = gpu
                    .device
                    .as_ref()
                    .expect("Live device")
                    .poll(bevy_render::render_resource::PollType::wait_indefinitely())
                {
                    tracing::warn!(?error, "GPU wait failed during native runtime shutdown");
                }
                tracing::info!("Native graphics session: tracked GPU wait end");
            }
            #[cfg(feature = "dx12_ngx_init")]
            let bootstrap = crate::dx12_ngx_init::prepare_shutdown();
            tracing::info!(
                "Native graphics session: provider shutdown after feature/parameter destructors"
            );
            shutdown_in_order(gpu, RuntimeCode::shutdown_provider, || {
                #[cfg(feature = "dx12_ngx_init")]
                drop(bootstrap);
            });
        }
    }
    pub(super) fn registry() -> &'static Arc<Registry<RuntimeSession>> {
        static REGISTRY: OnceLock<Arc<Registry<RuntimeSession>>> = OnceLock::new();
        REGISTRY.get_or_init(|| Arc::new(Registry::new()))
    }
    fn native_ready() -> bool {
        #[cfg(feature = "dx12_ngx_init")]
        let direct = crate::dx12_ngx_init::ngx_d3d12_ready();
        #[cfg(not(feature = "dx12_ngx_init"))]
        let direct = false;
        #[cfg(feature = "dlss_frame_generation")]
        let streamline = solarik_streamline_wgpu::Streamline::is_initialized();
        #[cfg(not(feature = "dlss_frame_generation"))]
        let streamline = false;
        needs_gpu(direct, streamline)
    }
    pub(crate) fn retain(app: &mut App) -> bool {
        if !native_ready() {
            return false;
        }
        let Some(render) = app.get_sub_app(RenderApp) else {
            // A later-registered SessionPlugin can clean up after pipelined rendering
            // transferred RenderApp. FrameGenerationPlugin::finish already retained it.
            return registry()
                .acquire_existing()
                .is_some_and(|session| session.get().gpu.lock().unwrap().is_some());
        };
        let Some(device) = render.world().get_resource::<RenderDevice>().cloned() else {
            return false;
        };
        let Some(queue) = render.world().get_resource::<RenderQueue>().cloned() else {
            return false;
        };
        let session = registry().acquire_existing().expect("D3D12 NGX applications must use bevy_solarik::run_app so native code outlives the complete App");
        session.get().retain_gpu(device, queue);
        #[cfg(feature = "dx12_ngx_init")]
        {
            let owner: Arc<dyn std::any::Any + Send + Sync> = session;
            bevy_anti_alias::dlss::retain_dlss_runtime(app, owner);
        }
        true
    }
    pub(crate) struct SessionPlugin;
    impl Plugin for SessionPlugin {
        fn build(&self, _: &mut App) {}
        fn cleanup(&self, app: &mut App) {
            // All finish hooks have completed; cleanup has not yet transferred
            // RenderApp to its rendering thread. Either native provider needs ownership.
            if native_ready() {
                assert!(
                    retain(app),
                    "NGX session must retain its live renderer before pipelined transfer"
                );
            }
        }
    }
}
#[cfg(all(
    windows,
    any(feature = "dx12_ngx_init", feature = "dlss_frame_generation")
))]
pub(crate) use runtime::{SessionPlugin, retain};

#[cfg(test)]
mod tests {
    use super::lifetime::{Registry, needs_gpu, shutdown_in_order};
    use std::sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    struct Tag(&'static str, Arc<Mutex<Vec<&'static str>>>);
    impl Drop for Tag {
        fn drop(&mut self) {
            self.1.lock().unwrap().push(self.0);
        }
    }
    struct MockGpu {
        queue: Option<Tag>,
        device: Option<Tag>,
    }
    impl Drop for MockGpu {
        fn drop(&mut self) {
            drop(self.queue.take());
            drop(self.device.take());
        }
    }
    struct Session {
        log: Arc<Mutex<Vec<&'static str>>>,
        gpu: Option<MockGpu>,
        provider: bool,
    }
    impl Drop for Session {
        fn drop(&mut self) {
            self.log.lock().unwrap().push("gpu_wait");
            self.log.lock().unwrap().push("bootstrap_parameters");
            shutdown_in_order(
                self.gpu.take(),
                || {
                    if self.provider {
                        self.log.lock().unwrap().push("sl_shutdown");
                    }
                    Tag("core_interposer_unload", Arc::clone(&self.log))
                },
                || {
                    self.log.lock().unwrap().push("ngx_shutdown");
                },
            );
        }
    }
    struct Sdk<T> {
        log: Arc<Mutex<Vec<&'static str>>>,
        _owner: Arc<T>,
    }
    impl<T> Drop for Sdk<T> {
        fn drop(&mut self) {
            self.log.lock().unwrap().push("parameters");
        }
    }
    struct Feature<T> {
        log: Arc<Mutex<Vec<&'static str>>>,
        _sdk: Arc<T>,
    }
    impl<T> Drop for Feature<T> {
        fn drop(&mut self) {
            self.log.lock().unwrap().push("feature_release");
        }
    }

    #[test]
    fn shared_lease_outlives_every_feature_parameters_queue_device_and_provider_code() {
        for pipeline in [false, true] {
            for provider in [false, true] {
                for owner_first in [false, true] {
                    let log = Arc::new(Mutex::new(Vec::new()));
                    let registry = Arc::new(Registry::new());
                    assert!(registry.acquire_existing().is_none());
                    let mut outer = Some(registry.acquire(|| Session {
                        log: Arc::clone(&log),
                        gpu: Some(MockGpu {
                            queue: Some(Tag("queue", Arc::clone(&log))),
                            device: Some(Tag("device", Arc::clone(&log))),
                        }),
                        provider,
                    }));
                    let mut main = Some(Arc::clone(outer.as_ref().unwrap()));
                    let render =
                        registry.acquire(|| panic!("A second live session cannot be initialized"));
                    let sdk = Arc::new(Sdk {
                        log: Arc::clone(&log),
                        _owner: Arc::clone(&render),
                    });
                    let sr = Feature {
                        log: Arc::clone(&log),
                        _sdk: Arc::clone(&sdk),
                    };
                    let rr = Feature {
                        log: Arc::clone(&log),
                        _sdk: Arc::clone(&sdk),
                    };
                    let fg = provider.then(|| Arc::clone(&render));
                    if owner_first {
                        drop(outer.take());
                        drop(main.take());
                    }
                    drop(sdk);
                    if pipeline {
                        drop(sr);
                        drop(render);
                        drop(rr);
                    } else {
                        drop(rr);
                        drop(render);
                        drop(sr);
                    }
                    drop(fg);
                    drop(main.take());
                    drop(outer.take());
                    let mut expected = vec![
                        "feature_release",
                        "feature_release",
                        "parameters",
                        "gpu_wait",
                        "bootstrap_parameters",
                    ];
                    if provider {
                        expected.push("sl_shutdown");
                    }
                    expected.extend(["ngx_shutdown", "queue", "device", "core_interposer_unload"]);
                    assert_eq!(*log.lock().unwrap(), expected);
                }
            }
        }
    }

    #[test]
    fn native_provider_flags_retain_without_capability_or_legacy_ngx() {
        for direct in [false, true] {
            for streamline in [false, true] {
                assert_eq!(needs_gpu(direct, streamline), direct || streamline);
            }
        }
    }

    #[test]
    fn streamline_only_and_preinit_failure_cleanup_once_after_app() {
        for initialized_device in [false, true] {
            let log = Arc::new(Mutex::new(Vec::new()));
            let registry = Arc::new(Registry::new());
            let outer = registry.acquire(|| Tag("outer_cleanup", Arc::clone(&log)));
            let app = Tag("app", Arc::clone(&log));
            let gpu = initialized_device.then(|| MockGpu {
                queue: Some(Tag("queue", Arc::clone(&log))),
                device: Some(Tag("device", Arc::clone(&log))),
            });
            drop(app);
            shutdown_in_order(
                gpu,
                || {
                    log.lock().unwrap().push("sl_shutdown");
                    Tag("core_interposer_unload", Arc::clone(&log))
                },
                || {}, // No direct NGX bootstrap or SDK shutdown in this configuration.
            );
            drop(outer);
            assert!(registry.acquire_existing().is_none());
            let expected = if initialized_device {
                vec![
                    "app",
                    "sl_shutdown",
                    "queue",
                    "device",
                    "core_interposer_unload",
                    "outer_cleanup",
                ]
            } else {
                vec![
                    "app",
                    "sl_shutdown",
                    "core_interposer_unload",
                    "outer_cleanup",
                ]
            };
            assert_eq!(*log.lock().unwrap(), expected);
        }
    }

    #[test]
    fn sdk_feature_dlls_detach_before_gpu_but_core_interposer_outlives_gpu() {
        for direct in [false, true] {
            for streamline in [false, true] {
                if !needs_gpu(direct, streamline) {
                    continue;
                }
                let log = Arc::new(Mutex::new(Vec::new()));
                let gpu = MockGpu {
                    queue: Some(Tag("queue", Arc::clone(&log))),
                    device: Some(Tag("device", Arc::clone(&log))),
                };
                shutdown_in_order(
                    gpu,
                    || {
                        if streamline {
                            log.lock().unwrap().push("sl_shutdown");
                            drop(Tag("sl_feature_dll_detach", Arc::clone(&log)));
                        }
                        streamline.then(|| Tag("core_interposer_unload", Arc::clone(&log)))
                    },
                    || {
                        if direct {
                            log.lock().unwrap().push("ngx_shutdown");
                            drop(Tag("ngx_model_dll_detach", Arc::clone(&log)));
                        }
                    },
                );
                let mut expected = Vec::new();
                if streamline {
                    expected.extend(["sl_shutdown", "sl_feature_dll_detach"]);
                }
                if direct {
                    expected.extend(["ngx_shutdown", "ngx_model_dll_detach"]);
                }
                expected.extend(["queue", "device"]);
                if streamline {
                    expected.push("core_interposer_unload");
                }
                assert_eq!(*log.lock().unwrap(), expected);
            }
        }
    }

    #[test]
    fn no_runtime_cleanup_does_not_create_a_session() {
        let registry: Arc<Registry<()>> = Arc::new(Registry::new());
        assert!(registry.acquire_existing().is_none());
        assert!(!needs_gpu(false, false));
        shutdown_in_order(None::<MockGpu>, || None::<Tag>, || {});
        assert!(registry.acquire_existing().is_none());
    }

    #[test]
    fn acquire_waits_for_a_barrier_pinned_final_destructor() {
        struct Pinned {
            entered: Option<Arc<Barrier>>,
            release: Option<Arc<Barrier>>,
        }
        impl Drop for Pinned {
            fn drop(&mut self) {
                if let Some(entered) = &self.entered {
                    entered.wait();
                }
                if let Some(release) = &self.release {
                    release.wait();
                }
            }
        }
        let registry = Arc::new(Registry::new());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let first = registry.acquire(|| Pinned {
            entered: Some(Arc::clone(&entered)),
            release: Some(Arc::clone(&release)),
        });
        let dropping = std::thread::spawn(move || drop(first));
        entered.wait();
        let (sent, received) = std::sync::mpsc::channel();
        let acquiring_registry = Arc::clone(&registry);
        let acquiring = std::thread::spawn(move || {
            acquiring_registry.acquire_observing(
                || Pinned {
                    entered: None,
                    release: None,
                },
                || {
                    let _ = sent.send(());
                },
            )
        });
        // Timeout is only a deadlock guard for this CPU coordination test, not a soak or
        // elapsed-time success criterion. A correct acquire explicitly reports waiting.
        let waited = received.recv_timeout(std::time::Duration::from_secs(3));
        release.wait();
        dropping.join().unwrap();
        let next = acquiring.join().unwrap();
        waited.expect("Concurrent acquire must observe Closing, not create a replacement runtime");
        drop(next);
        assert!(registry.acquire_existing().is_none());
    }

    #[test]
    fn concurrent_acquire_and_last_drop_never_overlap_runtime_instances() {
        struct Count {
            active: Arc<AtomicUsize>,
            created: Arc<AtomicUsize>,
        }
        impl Drop for Count {
            fn drop(&mut self) {
                assert_eq!(self.active.fetch_sub(1, Ordering::SeqCst), 1);
            }
        }
        let registry = Arc::new(Registry::new());
        let active = Arc::new(AtomicUsize::new(0));
        let created = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(8));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let registry = Arc::clone(&registry);
            let active = Arc::clone(&active);
            let created = Arc::clone(&created);
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..64 {
                    let lease = registry.acquire(|| {
                        assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
                        created.fetch_add(1, Ordering::SeqCst);
                        Count {
                            active: Arc::clone(&active),
                            created: Arc::clone(&created),
                        }
                    });
                    assert_eq!(lease.get().active.load(Ordering::SeqCst), 1);
                    assert!(lease.get().created.load(Ordering::SeqCst) > 0);
                    drop(lease);
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert!(created.load(Ordering::SeqCst) > 0);
    }
}
