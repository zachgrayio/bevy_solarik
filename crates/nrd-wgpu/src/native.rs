//! The only unsafe boundary: opaque NRD ownership, copying SDK-owned results and trusted SDK SPIR-V.
use super::{Error, Frame};
use std::{borrow::Cow, ffi::c_void, ptr::NonNull, slice};

mod ffi {
    use std::ffi::c_void;
    #[repr(C)]
    pub struct SnrdPipeline {
        pub spirv: *const u32,
        pub bytes: u64,
        pub textures: u32,
        pub storage_textures: u32,
        pub common_space: u32,
        pub resource_space: u32,
        pub constant_bytes: u32,
    }
    #[repr(C)]
    pub struct SnrdTexture {
        pub format: u32,
        pub downsample: u32,
    }
    #[repr(C)]
    pub struct SnrdResource {
        pub kind: u32,
        pub index: u32,
        pub storage: u32,
    }
    #[repr(C)]
    pub struct SnrdDispatch {
        pub constants: *const u8,
        pub resources: *const SnrdResource,
        pub constant_bytes: u32,
        pub resource_count: u32,
        pub pipeline: u32,
        pub width: u32,
        pub height: u32,
    }
    #[repr(C)]
    pub struct SnrdFrame {
        pub projection: [f32; 16],
        pub previous_projection: [f32; 16],
        pub world_to_view: [f32; 16],
        pub previous_world_to_view: [f32; 16],
        pub width: u32,
        pub height: u32,
        pub index: u32,
        pub reset: u32,
    }
    unsafe extern "C" {
        pub fn snrd_create() -> *mut c_void;
        pub fn snrd_destroy(instance: *mut c_void);
        pub fn snrd_pipeline_count(instance: *mut c_void) -> u32;
        pub fn snrd_pipeline(instance: *mut c_void, index: u32) -> SnrdPipeline;
        pub fn snrd_texture_count(instance: *mut c_void, permanent: u32) -> u32;
        pub fn snrd_texture(instance: *mut c_void, permanent: u32, index: u32) -> SnrdTexture;
        pub fn snrd_update(instance: *mut c_void, frame: *const SnrdFrame) -> u32;
        pub fn snrd_dispatch_count(instance: *mut c_void) -> u32;
        pub fn snrd_dispatch(instance: *mut c_void, index: u32) -> SnrdDispatch;
    }
    const _: () = assert!(
        size_of::<SnrdPipeline>() == 40
            && size_of::<SnrdDispatch>() == 40
            && size_of::<SnrdFrame>() == 272
    );
}

pub(super) struct Pipeline {
    pub spirv: Vec<u32>,
    pub textures: u32,
    pub storage_textures: u32,
    pub common_space: u32,
    pub resource_space: u32,
}
pub(super) struct Resource {
    pub kind: u32,
    pub index: u32,
    pub storage: bool,
}
pub(super) struct Dispatch {
    pub constants: Vec<u8>,
    pub resources: Vec<Resource>,
    pub pipeline: u32,
    pub width: u32,
    pub height: u32,
}

pub(super) struct Instance {
    handle: NonNull<c_void>,
    pipelines: Vec<ffi::SnrdPipeline>,
}
// SAFETY: access to this opaque instance is exclusive through &mut self. The controller has no
// thread affinity; its pointers never escape this module. Applications wrap camera contexts in Mutex.
unsafe impl Send for Instance {}

impl Instance {
    pub fn new() -> Result<Self, Error> {
        // SAFETY: the no-argument bridge allocates an owned SDK instance or returns null.
        let handle = NonNull::new(unsafe { ffi::snrd_create() }).ok_or(Error::Initialization)?;
        // SAFETY: handle is live; indices are bounded by the SDK's immutable pipeline count.
        let count = unsafe { ffi::snrd_pipeline_count(handle.as_ptr()) };
        let pipelines = (0..count)
            .map(|i| unsafe { ffi::snrd_pipeline(handle.as_ptr(), i) })
            .collect();
        Ok(Self { handle, pipelines })
    }
    pub fn constant_bytes(&self) -> u32 {
        self.pipelines.first().map_or(0, |p| p.constant_bytes)
    }
    pub fn pipeline(&self, index: u32) -> Pipeline {
        let p = &self.pipelines[index as usize];
        // SAFETY: bytecode belongs to our live instance. ShaderMake blobs need not be u32-aligned;
        // copy bytes into aligned words instead of forming an unaligned u32 slice.
        let bytes = unsafe { slice::from_raw_parts(p.spirv.cast::<u8>(), p.bytes as usize) };
        let spirv = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .collect();
        Pipeline {
            spirv,
            textures: p.textures,
            storage_textures: p.storage_textures,
            common_space: p.common_space,
            resource_space: p.resource_space,
        }
    }
    pub fn pool(&self, permanent: bool) -> Vec<(u32, u32)> {
        // SAFETY: pool metadata is immutable for the lifetime of this live instance.
        let count = unsafe { ffi::snrd_texture_count(self.handle.as_ptr(), permanent.into()) };
        (0..count)
            .map(|index| {
                // SAFETY: index is strictly below the reported pool count.
                let desc =
                    unsafe { ffi::snrd_texture(self.handle.as_ptr(), permanent.into(), index) };
                (desc.format, desc.downsample)
            })
            .collect()
    }
    pub fn update(
        &mut self,
        width: u32,
        height: u32,
        frame: &Frame,
    ) -> Result<Vec<Dispatch>, Error> {
        let frame = ffi::SnrdFrame {
            projection: frame.projection,
            previous_projection: frame.previous_projection,
            world_to_view: frame.world_to_view,
            previous_world_to_view: frame.previous_world_to_view,
            width,
            height,
            index: frame.index,
            reset: frame.reset.into(),
        };
        // SAFETY: the borrowed frame has the exact generated ABI layout and outlives this call.
        let result = unsafe { ffi::snrd_update(self.handle.as_ptr(), &frame) };
        if result != 0 {
            return Err(Error::Native(result));
        }
        // SAFETY: dispatches were published by the successful update and are copied before any next update.
        let count = unsafe { ffi::snrd_dispatch_count(self.handle.as_ptr()) };
        (0..count)
            .map(|i| {
                // SAFETY: i is within the bridge-owned dispatch array.
                let dispatch = unsafe { ffi::snrd_dispatch(self.handle.as_ptr(), i) };
                let constants = if dispatch.constant_bytes == 0 {
                    Vec::new()
                } else {
                    // SAFETY: the bridge reports this allocation's byte length; update hasn't been called again.
                    unsafe {
                        slice::from_raw_parts(dispatch.constants, dispatch.constant_bytes as usize)
                    }
                    .to_vec()
                };
                // SAFETY: the bridge-owned vector has resource_count initialized entries and is still live.
                let resources = unsafe {
                    slice::from_raw_parts(dispatch.resources, dispatch.resource_count as usize)
                }
                .iter()
                .map(|r| Resource {
                    kind: r.kind,
                    index: r.index,
                    storage: r.storage != 0,
                })
                .collect();
                if dispatch.pipeline as usize >= self.pipelines.len() {
                    return Err(Error::Initialization);
                }
                Ok(Dispatch {
                    constants,
                    resources,
                    pipeline: dispatch.pipeline,
                    width: dispatch.width,
                    height: dispatch.height,
                })
            })
            .collect()
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the unique owned instance is destroyed once, after all bridge calls have completed.
        unsafe { ffi::snrd_destroy(self.handle.as_ptr()) };
    }
}

pub(super) fn shader(
    device: &wgpu::Device,
    source: &[u32],
    storage_count: u32,
) -> wgpu::ShaderModule {
    // wgpu 30 Vulkan compacts descriptor bindings. Give it dense layouts and rewrite only
    // SDK resource Binding decorations (u3.. and t20..); group 1 is already dense (s0,s1,b2).
    let mut spirv = source.to_vec();
    let mut sets = std::collections::HashMap::new();
    let mut cursor = 5;
    while cursor < spirv.len() {
        let count = (spirv[cursor] >> 16) as usize;
        assert!(
            count > 0 && cursor + count <= spirv.len(),
            "Complete SDK SPIR-V instruction"
        );
        if spirv[cursor] & 0xffff == 71 && count == 4 && spirv[cursor + 2] == 34 {
            sets.insert(spirv[cursor + 1], spirv[cursor + 3]);
        }
        cursor += count;
    }
    // SPIR-V OpExecutionMode LocalSize supplies the exact source-built kernel dimensions.
    let mut cursor = 5;
    let mut workgroup_size = None;
    while cursor < spirv.len() {
        let count = (spirv[cursor] >> 16) as usize;
        assert!(
            count > 0 && cursor + count <= spirv.len(),
            "Complete SDK SPIR-V instruction"
        );
        if spirv[cursor] & 0xffff == 16 && count == 6 && spirv[cursor + 2] == 17 {
            workgroup_size = Some([spirv[cursor + 3], spirv[cursor + 4], spirv[cursor + 5]]);
        }
        if spirv[cursor] & 0xffff == 71
            && count == 4
            && spirv[cursor + 2] == 33
            && sets.get(&spirv[cursor + 1]) == Some(&0)
        {
            let binding = &mut spirv[cursor + 3];
            *binding = if *binding >= 20 {
                storage_count + *binding - 20
            } else {
                assert!(
                    *binding >= 3 && *binding < 3 + storage_count,
                    "SDK UAV binding"
                );
                *binding - 3
            };
        }
        cursor += count;
    }
    let entry_points = [wgpu::PassthroughShaderEntryPoint {
        name: Cow::Borrowed("main"),
        workgroup_size: workgroup_size
            .expect("NRD kernel declares LocalSize")
            .into(),
    }];
    // SAFETY: these are the pinned SDK's source-built Vulkan 1.2 shaders. Their descriptors,
    // pool dimensions and dispatch sizes are supplied by the same SDK instance, not user input.
    unsafe {
        device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
            label: Some("NRD 4.18 RELAX SPIR-V"),
            spirv: Some(Cow::Borrowed(&spirv)),
            entry_points: Cow::Borrowed(&entry_points),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_sdk_publishes_real_relax_shaders_and_owned_dispatches() {
        let mut instance = Instance::new().unwrap();
        assert!(instance.pipelines.len() > 5);
        let pipeline = instance.pipeline(0);
        assert_eq!(pipeline.spirv[0], 0x07230203);
        assert_eq!((pipeline.resource_space, pipeline.common_space), (0, 1));
        let perspective = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.1, 0.0,
        ];
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let frame = Frame {
            projection: perspective,
            previous_projection: perspective,
            world_to_view: identity,
            previous_world_to_view: identity,
            index: 0,
            reset: true,
        };
        let dispatches = instance.update(64, 48, &frame).unwrap();
        assert!(dispatches.len() > 5);
        assert!(
            dispatches
                .iter()
                .all(|d| d.width > 0 && d.height > 0 && d.resources.iter().all(|r| r.kind <= 8))
        );
    }
}
