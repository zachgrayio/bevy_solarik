# Local device pipeline cache

**Purpose** — reuse Vulkan pipeline compilation across application processes.
**Files** — vendor/bevy_render/src/render_resource/{persistent_cache,pipeline_cache}.rs;
vendor/bevy_render/src/settings.rs.
vendor/bevy_shader/src/shader_cache.rs stabilizes WESL module assembly for repeatable native keys.
**Data** — optional main-world PipelineCacheStorage resource supplies an application-owned
local cache directory. RenderResources recreates the device cache during normal initialization
and device recovery. Unsupported features/backends retain ordinary pipeline creation.

**Flow** — insert PipelineCacheStorage before RenderPlugin initialization. The renderer derives
the file name from wgpu's adapter cache key, driver name/version and envelope version. A cache
miss creates an empty native cache. All render/compute/mesh pipeline descriptors share that cache.
When pipeline compilation becomes idle, snapshot the native data and atomically persist it on
IoTaskPool. At most one write is in flight. A completed-pipeline revision triggers later saves,
including recompiles of existing IDs after shader changes. Renderer teardown joins an active write.

**Invariants** — data is a prior wgpu get_data() result in trusted application-local storage,
never authored assets or downloaded blobs. Before native access, validate the envelope version,
exact payload length, 64 MiB bound and BLAKE3 digest. wgpu additionally checks backend/device/version
compatibility and falls back when native cache data becomes stale. Shader/pipeline changes use
wgpu's normal native keys; cached code cannot substitute for a different shader.
WESL's resolved/mangled import modules are linked main-first and then by module path. Unordered
import-graph discovery cannot reorder declarations between processes and defeat native cache hits.
Read or write failures warn and preserve ordinary compilation. A failed write disables further
attempts for that process and leaves the last complete file intact. No readiness gate is bypassed.
First launch and cache misses still compile; subsequent processes can reuse successful results.

**Extending** — keep directory policy in the application. Bounded codec/file tests need no GPU.
Cold/warm behavior and rendered output require the application's normal renderer/capture path.
