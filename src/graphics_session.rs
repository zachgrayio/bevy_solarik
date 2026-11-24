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
    #[cfg(all(windows, feature = "dx12_ngx_init"))]
    let session = runtime::registry().acquire(runtime::RuntimeSession::default);
    let mut app = build();
    let exit = app.run();
    drop(app);
    #[cfg(all(windows, feature = "dx12_ngx_init"))]
    drop(session);
    exit
}

#[cfg(any(test, all(windows, feature = "dx12_ngx_init")))]
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

    pub(super) fn shutdown_in_order<G, L>(
        gpu: G,
        stop_provider: impl FnOnce() -> L,
        stop_ngx: impl FnOnce(),
    ) {
        let code = stop_provider();
        stop_ngx();
        drop(gpu);
        // Provider code remains loaded through the final owned queue/device destruction.
        drop(code);
    }
}

#[cfg(all(windows, feature = "dx12_ngx_init"))]
mod runtime {
    use super::lifetime::{Registry, shutdown_in_order};
    use bevy_app::{App, Plugin};
    use bevy_render::{
        RenderApp,
        renderer::{RenderDevice, RenderQueue},
    };
    use std::{
        any::Any,
        sync::{Arc, Mutex, OnceLock},
    };

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
        modules: Vec<libloading::Library>,
    }
    impl RuntimeCode {
        fn shutdown_provider() -> Self {
            let mut modules = Vec::new();
            for name in [
                "_nvngx.dll",
                "nvngx_dlss.dll",
                "nvngx_dlssd.dll",
                "nvngx_dlssg.dll",
            ] {
                // Retain only modules already initialized by the application. This never
                // loads a new provider or permanently pins native graphics code.
                if let Ok(module) = libloading::os::windows::Library::open_already_loaded(name) {
                    modules.push(module.into());
                }
            }
            #[cfg(feature = "dlss_frame_generation")]
            let streamline = solarik_streamline_wgpu::shutdown_retaining_runtime();
            Self {
                #[cfg(feature = "dlss_frame_generation")]
                streamline,
                modules,
            }
        }
    }
    impl Drop for RuntimeCode {
        fn drop(&mut self) {
            #[cfg(feature = "dlss_frame_generation")]
            drop(self.streamline.take());
            self.modules.clear();
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
            let gpu = self.gpu.get_mut().unwrap().take();
            if gpu.is_none() && !crate::dx12_ngx_init::ngx_d3d12_ready() {
                return;
            }
            if let Some(gpu) = &gpu {
                if let Err(error) = gpu
                    .device
                    .as_ref()
                    .expect("Live device")
                    .poll(bevy_render::render_resource::PollType::wait_indefinitely())
                {
                    tracing::warn!(?error, "GPU wait failed during native runtime shutdown");
                }
            }
            let bootstrap = crate::dx12_ngx_init::prepare_shutdown();
            tracing::info!(
                "Native graphics session: provider shutdown after feature/parameter destructors"
            );
            shutdown_in_order(gpu, RuntimeCode::shutdown_provider, || drop(bootstrap));
        }
    }
    pub(super) fn registry() -> &'static Arc<Registry<RuntimeSession>> {
        static REGISTRY: OnceLock<Arc<Registry<RuntimeSession>>> = OnceLock::new();
        REGISTRY.get_or_init(|| Arc::new(Registry::new()))
    }
    pub(crate) fn retain(app: &mut App) -> bool {
        if !crate::dx12_ngx_init::ngx_d3d12_ready() {
            return false;
        }
        let Some(render) = app.get_sub_app(RenderApp) else {
            return false;
        };
        let Some(device) = render.world().get_resource::<RenderDevice>().cloned() else {
            return false;
        };
        let Some(queue) = render.world().get_resource::<RenderQueue>().cloned() else {
            return false;
        };
        let session = registry().acquire_existing().expect("D3D12 NGX applications must use bevy_solarik::run_app so native code outlives the complete App");
        session.get().retain_gpu(device, queue);
        let owner: Arc<dyn Any + Send + Sync> = session;
        bevy_anti_alias::dlss::retain_dlss_runtime(app, owner);
        true
    }
    pub(crate) struct SessionPlugin;
    impl Plugin for SessionPlugin {
        fn build(&self, _: &mut App) {}
        fn cleanup(&self, app: &mut App) {
            // Installed by the raw-init seam before DefaultPlugins. All finish hooks have
            // completed; cleanup has not yet transferred RenderApp to its rendering thread.
            if crate::dx12_ngx_init::ngx_d3d12_ready() {
                assert!(
                    retain(app),
                    "NGX session must retain its live renderer before pipelined transfer"
                );
            }
        }
    }
}
#[cfg(all(windows, feature = "dx12_ngx_init"))]
pub(crate) use runtime::{SessionPlugin, retain};

#[cfg(test)]
mod tests {
    use super::lifetime::{Registry, shutdown_in_order};
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
                    Tag("dll_unload", Arc::clone(&self.log))
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
                    expected.extend(["ngx_shutdown", "queue", "device", "dll_unload"]);
                    assert_eq!(*log.lock().unwrap(), expected);
                }
            }
        }
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
