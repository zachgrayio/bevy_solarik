//! Optional local persistence for wgpu's opaque device pipeline cache.
use crate::renderer::{RenderAdapterInfo, RenderDevice, wgpu_wrapper};
use bevy_ecs::resource::Resource;
use bevy_log::{info, warn};
use bevy_tasks::{IoTaskPool, Task};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAGIC: &[u8; 8] = b"BEVYPC01";
const HEADER: usize = 48;
const MAX_BYTES: usize = 64 * 1024 * 1024;
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

wgpu_wrapper! {
    #[derive(Clone)]
    pub(super) struct DriverPipelineCache(wgpu::PipelineCache);
}

/// Enables a local device pipeline cache when inserted before `RenderPlugin`.
///
/// The directory is application-owned local cache storage, never an asset or downloaded input.
/// Device/driver identity and a checksummed envelope reject stale or damaged files. Shader
/// changes are handled by the native pipeline keys. Unsupported devices continue without caching.
#[derive(Resource, Clone)]
pub struct PipelineCacheStorage {
    /// Directory for disposable, application-owned cache files.
    pub directory: PathBuf,
}

pub(super) struct PersistentCache {
    pub(super) raw: DriverPipelineCache,
    path: PathBuf,
    saved_revision: u64,
    write: Option<Task<io::Result<u64>>>,
    failed: bool,
}

impl PersistentCache {
    pub(super) fn new(
        device: &RenderDevice,
        adapter: &RenderAdapterInfo,
        storage: &PipelineCacheStorage,
    ) -> Option<Self> {
        if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
            return None;
        }
        let key = wgpu::util::pipeline_cache_key(adapter)?;
        let path = storage
            .directory
            .join(file_name(&key, &adapter.driver, &adapter.driver_info));
        let data = match read(&path) {
            Ok(data) => Some(data),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                warn!("Pipeline cache ignored: {}: {error}", path.display());
                None
            }
        };
        #[expect(
            unsafe_code,
            reason = "The renderer owns the trusted local wgpu cache boundary"
        )]
        // SAFETY: Initial bytes are exclusively a prior get_data() result in this application's
        // local device/driver-specific cache. The versioned envelope validates length and digest
        // before this call. wgpu validates its own backend/version key and falls back on changes.
        let raw = unsafe {
            device
                .wgpu_device()
                .create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("persistent application pipeline cache"),
                    data: data.as_deref(),
                    fallback: true,
                })
        };
        info!(
            "Pipeline cache {}: {} bytes from {}",
            if data.is_some() { "loaded" } else { "cold" },
            data.as_ref().map_or(0, Vec::len),
            path.display()
        );
        Some(Self {
            raw: DriverPipelineCache::new(raw),
            path,
            saved_revision: 0,
            write: None,
            failed: false,
        })
    }

    pub(super) fn update(&mut self, revision: u64, pipelines: usize, idle: bool) {
        if let Some(task) = self.write.as_mut() {
            match bevy_tasks::futures::check_ready(task) {
                Some(Ok(count)) => {
                    self.saved_revision = count;
                    self.write = None;
                }
                Some(Err(error)) => {
                    warn!("Pipeline cache save disabled for this session: {error}");
                    self.failed = true;
                    self.write = None;
                }
                None => return,
            }
        }
        if !idle || self.failed || revision == 0 || revision == self.saved_revision {
            return;
        }
        let Some(data) = self.raw.get_data() else {
            self.failed = true;
            return;
        };
        if data.len() > MAX_BYTES {
            warn!(
                "Pipeline cache exceeds the local size limit; persistence disabled for this session"
            );
            self.failed = true;
            return;
        }
        let path = self.path.clone();
        self.write = Some(IoTaskPool::get().spawn(async move {
            save(&path, &data)?;
            info!(
                "Pipeline cache saved: {} bytes for {pipelines} pipelines",
                data.len()
            );
            Ok(revision)
        }));
    }
}

impl Drop for PersistentCache {
    fn drop(&mut self) {
        // Finish an already-started atomic write during normal renderer teardown.
        if let Some(task) = self.write.take()
            && let Err(error) = bevy_tasks::block_on(task)
        {
            warn!("Pipeline cache save failed: {error}");
        }
    }
}

fn file_name(key: &str, driver: &str, driver_info: &str) -> String {
    let identity = format!("{key}\n{driver}\n{driver_info}\nbevy-wgpu-cache-v1");
    format!(
        "{key}-{}.bin",
        &blake3::hash(identity.as_bytes()).to_hex()[..16]
    )
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid pipeline cache envelope",
    )
}

fn decode(bytes: &[u8]) -> io::Result<&[u8]> {
    if bytes.len() < HEADER || bytes.len() > MAX_BYTES + HEADER || &bytes[..8] != MAGIC {
        return Err(invalid());
    }
    let length = u64::from_le_bytes(bytes[8..16].try_into().map_err(|_| invalid())?);
    let data = &bytes[HEADER..];
    if length != data.len() as u64 || blake3::hash(data).as_bytes() != &bytes[16..48] {
        return Err(invalid());
    }
    Ok(data)
}

fn read(path: &Path) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > (MAX_BYTES + HEADER) as u64 {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + HEADER + 1) as u64)
        .read_to_end(&mut bytes)?;
    decode(&bytes)?;
    bytes.drain(..HEADER);
    Ok(bytes)
}

fn save(path: &Path, data: &[u8]) -> io::Result<()> {
    if data.len() > MAX_BYTES {
        return Err(invalid());
    }
    let parent = path.parent().ok_or_else(invalid)?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(MAGIC)?;
        file.write_all(&(data.len() as u64).to_le_bytes())?;
        file.write_all(blake3::hash(data).as_bytes())?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistent_cache_rejects_corruption_truncation_and_wrong_version() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("cache.bin");
        save(&path, b"opaque driver result")?;
        let original = fs::read(&path)?;
        assert_eq!(read(&path)?, b"opaque driver result");
        let mut corrupted = original.clone();
        *corrupted.last_mut().unwrap() ^= 1;
        assert!(decode(&corrupted).is_err());
        assert!(decode(&original[..original.len() - 1]).is_err());
        assert!(decode(&original[..8]).is_err());
        corrupted = original.clone();
        corrupted[7] = b'2';
        assert!(decode(&corrupted).is_err());
        corrupted = original;
        corrupted[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode(&corrupted).is_err());
        Ok(())
    }
    #[test]
    fn persistent_cache_replacement_is_atomic_and_failed_save_preserves_previous_data()
    -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("cache.bin");
        save(&path, b"old")?;
        save(&path, b"new")?;
        assert_eq!(read(&path)?, b"new");
        assert!(save(&path.join("invalid"), b"bad").is_err());
        assert_eq!(read(&path)?, b"new");
        assert_eq!(fs::read_dir(directory.path())?.count(), 1);
        Ok(())
    }
    #[test]
    fn persistent_cache_driver_identity_separates_incompatible_devices_and_drivers() {
        let key = file_name("vulkan_4318_9860", "NVIDIA", "621.01");
        assert_eq!(key, file_name("vulkan_4318_9860", "NVIDIA", "621.01"));
        assert_ne!(key, file_name("vulkan_4318_9860", "NVIDIA", "622.00"));
        assert_ne!(key, file_name("vulkan_4318_9861", "NVIDIA", "621.01"));
    }

    #[test]
    fn persistent_cache_rejects_oversized_files_before_reading_the_payload() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("cache.bin");
        fs::File::create(&path)?.set_len((MAX_BYTES + HEADER + 1) as u64)?;
        assert_eq!(read(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        Ok(())
    }
}
