//! Header-verified NVIDIA Streamline SDK v2.14.16 C ABI (`sl_*.h`).
//!
//! Pure `#[repr(C)]` data layouts matching `NVIDIA-RTX/Streamline@v2.14.16`:
//! `sl_struct.h`, `sl_consts.h`, `sl_version.h`, `sl_core_types.h`,
//! `sl_core_api.h`, `sl_dlss_g.h`, `sl_reflex.h`, `sl_pcl.h`, `sl_helpers_vk.h`.

#![allow(
    missing_docs,
    reason = "ABI fields mirror upstream Streamline C++ headers 1:1"
)]

use std::ffi::{c_char, c_void};

pub const SL_VERSION_MAJOR: u64 = 2;
pub const SL_VERSION_MINOR: u64 = 14;
pub const SL_VERSION_PATCH: u64 = 16;
pub const K_SDK_VERSION_MAGIC: u64 = 0xfedc;
pub const K_SDK_VERSION: u64 = (SL_VERSION_MAJOR << 48)
    | (SL_VERSION_MINOR << 32)
    | (SL_VERSION_PATCH << 16)
    | K_SDK_VERSION_MAGIC;

pub const K_STRUCT_VERSION_1: usize = 1;
pub const K_STRUCT_VERSION_2: usize = 2;
pub const K_STRUCT_VERSION_3: usize = 3;
pub const K_STRUCT_VERSION_4: usize = 4;
pub const K_STRUCT_VERSION_5: usize = 5;

pub const INVALID_FLOAT: f32 = 3.402_823_5e38_f32;

pub type SlResult = u32;
pub const SL_RESULT_OK: SlResult = 0;

pub type Feature = u32;
pub const K_FEATURE_DLSS: Feature = 0;
pub const K_FEATURE_REFLEX: Feature = 3;
pub const K_FEATURE_PCL: Feature = 4;
pub const K_FEATURE_DLSS_G: Feature = 1000;
pub const K_FEATURE_DLSS_RR: Feature = 1001;

pub type BufferType = u32;
pub const K_BUFFER_TYPE_DEPTH: BufferType = 0;
pub const K_BUFFER_TYPE_MOTION_VECTORS: BufferType = 1;
pub const K_BUFFER_TYPE_HUD_LESS_COLOR: BufferType = 2;
pub const K_BUFFER_TYPE_SCALING_INPUT_COLOR: BufferType = 3;
pub const K_BUFFER_TYPE_SCALING_OUTPUT_COLOR: BufferType = 4;
pub const K_BUFFER_TYPE_ALBEDO: BufferType = 7;
pub const K_BUFFER_TYPE_SPECULAR_ALBEDO: BufferType = 8;
pub const K_BUFFER_TYPE_SPECULAR_MOTION_VECTORS: BufferType = 10;
pub const K_BUFFER_TYPE_EXPOSURE: BufferType = 13;
pub const K_BUFFER_TYPE_NORMAL_ROUGHNESS: BufferType = 14;
pub const K_BUFFER_TYPE_SPECULAR_HIT_DISTANCE: BufferType = 42;

pub const RESOURCE_LIFECYCLE_VALID_UNTIL_EVALUATE: u32 = 2;

pub const DLSS_MODE_OFF: u32 = 0;
pub const DLSS_MODE_MAX_PERFORMANCE: u32 = 1;
pub const DLSS_MODE_BALANCED: u32 = 2;
pub const DLSS_MODE_MAX_QUALITY: u32 = 3;
pub const DLSS_MODE_ULTRA_PERFORMANCE: u32 = 4;
pub const DLSS_MODE_ULTRA_QUALITY: u32 = 5;
pub const DLSS_MODE_DLAA: u32 = 6;

pub const DLSSD_NORMAL_ROUGHNESS_PACKED: u32 = 1;

pub const RENDER_API_D3D12: u32 = 1;
pub const RENDER_API_VULKAN: u32 = 2;

pub const ENGINE_TYPE_CUSTOM: u32 = 0;
pub const LOG_LEVEL_DEFAULT: u32 = 1;

pub const PREF_DISABLE_CL_STATE_TRACKING: u64 = 1 << 0;
pub const PREF_USE_MANUAL_HOOKING: u64 = 1 << 2;
pub const PREF_USE_FRAME_BASED_RESOURCE_TAGGING: u64 = 1 << 7;

pub const RESOURCE_TYPE_TEX2D: i8 = 0;
pub const RESOURCE_LIFECYCLE_VALID_UNTIL_PRESENT: u32 = 1;

pub const BOOLEAN_FALSE: u8 = 0;
pub const BOOLEAN_TRUE: u8 = 1;
pub const BOOLEAN_INVALID: u8 = 2;

pub const DLSSG_MODE_OFF: u32 = 0;
pub const DLSSG_MODE_ON: u32 = 1;

pub const REFLEX_MODE_OFF: u32 = 0;
pub const REFLEX_MODE_LOW_LATENCY: u32 = 1;

pub const PCL_MARKER_SIMULATION_START: u32 = 0;
pub const PCL_MARKER_SIMULATION_END: u32 = 1;
pub const PCL_MARKER_RENDER_SUBMIT_START: u32 = 2;
pub const PCL_MARKER_RENDER_SUBMIT_END: u32 = 3;
pub const PCL_MARKER_PRESENT_START: u32 = 4;
pub const PCL_MARKER_PRESENT_END: u32 = 5;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StructType {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BaseStructure {
    pub next: *mut BaseStructure,
    pub struct_type: StructType,
    pub struct_version: usize,
}

impl BaseStructure {
    pub const fn new(struct_type: StructType, struct_version: usize) -> Self {
        Self {
            next: std::ptr::null_mut(),
            struct_type,
            struct_version,
        }
    }
}

pub const GUID_PREFERENCES: StructType = StructType {
    data1: 0x1ca1_0965,
    data2: 0xbf8e,
    data3: 0x432b,
    data4: [0x8d, 0xa1, 0x67, 0x16, 0xd8, 0x79, 0xfb, 0x14],
};
pub const GUID_VIEWPORT_HANDLE: StructType = StructType {
    data1: 0x171b_6435,
    data2: 0x9b3c,
    data3: 0x4fc8,
    data4: [0x99, 0x94, 0xfb, 0xe5, 0x25, 0x69, 0xaa, 0xa4],
};
pub const GUID_ADAPTER_INFO: StructType = StructType {
    data1: 0x0677_315f,
    data2: 0xa746,
    data3: 0x4492,
    data4: [0x9f, 0x42, 0xcb, 0x61, 0x42, 0xc9, 0xc3, 0xd4],
};
pub const GUID_RESOURCE: StructType = StructType {
    data1: 0x3a9d_70cf,
    data2: 0x2418,
    data3: 0x4b72,
    data4: [0x83, 0x91, 0x13, 0xf8, 0x72, 0x1c, 0x72, 0x61],
};
pub const GUID_SUBRESOURCE_RANGE: StructType = StructType {
    data1: 0x8d4c_316c,
    data2: 0xd402,
    data3: 0x4524,
    data4: [0x89, 0xa7, 0x14, 0xe7, 0x9e, 0x63, 0x8e, 0x3a],
};
pub const GUID_RESOURCE_TAG: StructType = StructType {
    data1: 0x4c6a_5aad,
    data2: 0xb445,
    data3: 0x496c,
    data4: [0x87, 0xff, 0x1a, 0xf3, 0x84, 0x5b, 0xe6, 0x53],
};
pub const GUID_CONSTANTS: StructType = StructType {
    data1: 0xdcd3_5ad7,
    data2: 0x4e4a,
    data3: 0x4bad,
    data4: [0xa9, 0x0c, 0xe0, 0xc4, 0x9e, 0xb2, 0x3a, 0xfe],
};
pub const GUID_DLSSG_OPTIONS: StructType = StructType {
    data1: 0xfac5_f1cb,
    data2: 0x2dfd,
    data3: 0x4f36,
    data4: [0xa1, 0xe6, 0x3a, 0x9e, 0x86, 0x52, 0x56, 0xc5],
};
pub const GUID_DLSSG_STATE: StructType = StructType {
    data1: 0xcc8a_c8e1,
    data2: 0xa179,
    data3: 0x44f5,
    data4: [0x97, 0xfa, 0xe7, 0x41, 0x12, 0xf9, 0xbc, 0x61],
};
pub const GUID_REFLEX_OPTIONS: StructType = StructType {
    data1: 0xf03a_f81a,
    data2: 0x6d0b,
    data3: 0x4902,
    data4: [0xa6, 0x51, 0xc4, 0x96, 0x5e, 0x21, 0x54, 0x34],
};
pub const GUID_VULKAN_INFO: StructType = StructType {
    data1: 0x0eed_6fd5,
    data2: 0x82cd,
    data3: 0x43a9,
    data4: [0xbd, 0xb5, 0x47, 0xa5, 0xba, 0x2f, 0x45, 0xd6],
};

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Preferences {
    pub base: BaseStructure,
    pub show_console: bool,
    pub log_level: u32,
    pub paths_to_plugins: *const *const u16,
    pub num_paths_to_plugins: u32,
    pub path_to_logs_and_data: *const u16,
    pub allocate_callback: *const c_void,
    pub release_callback: *const c_void,
    pub log_message_callback: *const c_void,
    pub flags: u64,
    pub features_to_load: *const Feature,
    pub num_features_to_load: u32,
    pub application_id: u32,
    pub engine: u32,
    pub engine_version: *const c_char,
    pub project_id: *const c_char,
    pub render_api: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViewportHandle {
    pub base: BaseStructure,
    pub value: u32,
}

impl ViewportHandle {
    pub const fn new(value: u32) -> Self {
        Self {
            base: BaseStructure::new(GUID_VIEWPORT_HANDLE, K_STRUCT_VERSION_1),
            value,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AdapterInfo {
    pub base: BaseStructure,
    pub device_luid: *mut u8,
    pub device_luid_size_in_bytes: u32,
    pub vk_physical_device: *mut c_void,
}

impl Default for AdapterInfo {
    fn default() -> Self {
        Self {
            base: BaseStructure::new(GUID_ADAPTER_INFO, K_STRUCT_VERSION_1),
            device_luid: std::ptr::null_mut(),
            device_luid_size_in_bytes: 0,
            vk_physical_device: std::ptr::null_mut(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Extent {
    pub top: u32,
    pub left: u32,
    pub width: u32,
    pub height: u32,
}

/// Vulkan view range chained to `Resource::base.next` (Streamline 2.14.16).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SubresourceRange {
    pub base: BaseStructure,
    pub aspect_mask: u32,
    pub base_mip_level: u32,
    pub level_count: u32,
    pub base_array_layer: u32,
    pub layer_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Resource {
    pub base: BaseStructure,
    pub kind: i8,
    pub native: *mut c_void,
    pub memory: *mut c_void,
    pub view: *mut c_void,
    pub state: u32,
    pub width: u32,
    pub height: u32,
    pub native_format: u32,
    pub mip_levels: u32,
    pub array_layers: u32,
    pub gpu_virtual_address: u64,
    pub flags: u32,
    pub usage: u32,
    pub internal_flags: u16,
    pub reserved: u16,
}

impl Resource {
    pub const fn new_tex2d(
        native: *mut c_void,
        state: u32,
        width: u32,
        height: u32,
        format: u32,
    ) -> Self {
        Self {
            base: BaseStructure::new(GUID_RESOURCE, K_STRUCT_VERSION_1),
            kind: RESOURCE_TYPE_TEX2D,
            native,
            memory: std::ptr::null_mut(),
            view: std::ptr::null_mut(),
            state,
            width,
            height,
            native_format: format,
            mip_levels: 1,
            array_layers: 1,
            gpu_virtual_address: 0,
            flags: 0,
            usage: 0,
            internal_flags: 0,
            reserved: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ResourceTag {
    pub base: BaseStructure,
    pub resource: *mut Resource,
    pub buffer_type: BufferType,
    pub lifecycle: u32,
    pub extent: Extent,
}

impl ResourceTag {
    pub const fn new(resource: *mut Resource, buffer_type: BufferType, extent: Extent) -> Self {
        Self {
            base: BaseStructure::new(GUID_RESOURCE_TAG, K_STRUCT_VERSION_1),
            resource,
            buffer_type,
            lifecycle: RESOURCE_LIFECYCLE_VALID_UNTIL_PRESENT,
            extent,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Constants {
    pub base: BaseStructure,
    pub camera_view_to_clip: [[f32; 4]; 4],
    pub clip_to_camera_view: [[f32; 4]; 4],
    pub clip_to_lens_clip: [[f32; 4]; 4],
    pub clip_to_prev_clip: [[f32; 4]; 4],
    pub prev_clip_to_clip: [[f32; 4]; 4],
    pub jitter_offset: [f32; 2],
    pub mvec_scale: [f32; 2],
    pub camera_pinhole_offset: [f32; 2],
    pub camera_pos: [f32; 3],
    pub camera_up: [f32; 3],
    pub camera_right: [f32; 3],
    pub camera_fwd: [f32; 3],
    pub camera_near: f32,
    pub camera_far: f32,
    pub camera_fov: f32,
    pub camera_aspect_ratio: f32,
    pub motion_vectors_invalid_value: f32,
    pub depth_inverted: u8,
    pub camera_motion_included: u8,
    pub motion_vectors_3d: u8,
    pub reset: u8,
    pub orthographic_projection: u8,
    pub motion_vectors_dilated: u8,
    pub motion_vectors_jittered: u8,
    pub min_relative_linear_depth_object_separation: f32,
}

impl Default for Constants {
    fn default() -> Self {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        Self {
            base: BaseStructure::new(GUID_CONSTANTS, K_STRUCT_VERSION_2),
            camera_view_to_clip: identity,
            clip_to_camera_view: identity,
            clip_to_lens_clip: identity,
            clip_to_prev_clip: identity,
            prev_clip_to_clip: identity,
            jitter_offset: [0.0, 0.0],
            mvec_scale: [1.0, 1.0],
            camera_pinhole_offset: [0.0, 0.0],
            camera_pos: [0.0, 0.0, 0.0],
            camera_up: [0.0, 1.0, 0.0],
            camera_right: [1.0, 0.0, 0.0],
            camera_fwd: [0.0, 0.0, -1.0],
            camera_near: 0.1,
            camera_far: 1000.0,
            camera_fov: core::f32::consts::FRAC_PI_4,
            camera_aspect_ratio: 16.0 / 9.0,
            motion_vectors_invalid_value: INVALID_FLOAT,
            depth_inverted: BOOLEAN_TRUE,
            camera_motion_included: BOOLEAN_TRUE,
            motion_vectors_3d: BOOLEAN_FALSE,
            reset: BOOLEAN_FALSE,
            orthographic_projection: BOOLEAN_FALSE,
            motion_vectors_dilated: BOOLEAN_FALSE,
            motion_vectors_jittered: BOOLEAN_FALSE,
            min_relative_linear_depth_object_separation: 40.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DlssGOptions {
    pub base: BaseStructure,
    pub mode: u32,
    pub num_frames_to_generate: u32,
    pub flags: u32,
    pub dynamic_res_width: u32,
    pub dynamic_res_height: u32,
    pub num_back_buffers: u32,
    pub mvec_depth_width: u32,
    pub mvec_depth_height: u32,
    pub color_width: u32,
    pub color_height: u32,
    pub color_buffer_format: u32,
    pub mvec_buffer_format: u32,
    pub depth_buffer_format: u32,
    pub hud_less_buffer_format: u32,
    pub ui_buffer_format: u32,
    pub on_error_callback: *const c_void,
    pub b_reserved15: u8,
    pub queue_parallelism_mode: u32,
    pub enable_user_interface_recomposition: u8,
    pub dynamic_target_frame_rate: f32,
}

impl Default for DlssGOptions {
    fn default() -> Self {
        Self {
            base: BaseStructure::new(GUID_DLSSG_OPTIONS, K_STRUCT_VERSION_5),
            mode: DLSSG_MODE_OFF,
            num_frames_to_generate: 1,
            flags: 0,
            dynamic_res_width: 0,
            dynamic_res_height: 0,
            num_back_buffers: 0,
            mvec_depth_width: 0,
            mvec_depth_height: 0,
            color_width: 0,
            color_height: 0,
            color_buffer_format: 0,
            mvec_buffer_format: 0,
            depth_buffer_format: 0,
            hud_less_buffer_format: 0,
            ui_buffer_format: 0,
            on_error_callback: std::ptr::null(),
            b_reserved15: BOOLEAN_INVALID,
            queue_parallelism_mode: 0,
            enable_user_interface_recomposition: BOOLEAN_FALSE,
            dynamic_target_frame_rate: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DlssGState {
    pub base: BaseStructure,
    pub estimated_vram_usage_in_bytes: u64,
    pub status: u32,
    pub min_width_or_height: u32,
    pub num_frames_actually_presented: u32,
    pub num_frames_to_generate_max: u32,
    pub b_reserved4: u8,
    pub b_is_vsync_support_available: u8,
    pub inputs_processing_completion_fence: *mut c_void,
    pub last_present_inputs_processing_completion_fence_value: u64,
    pub b_is_dynamic_mfg_supported: u8,
}

impl Default for DlssGState {
    fn default() -> Self {
        Self {
            base: BaseStructure::new(GUID_DLSSG_STATE, K_STRUCT_VERSION_4),
            estimated_vram_usage_in_bytes: 0,
            status: 0,
            min_width_or_height: 0,
            num_frames_actually_presented: 0,
            num_frames_to_generate_max: 0,
            b_reserved4: 0,
            b_is_vsync_support_available: 0,
            inputs_processing_completion_fence: std::ptr::null_mut(),
            last_present_inputs_processing_completion_fence_value: 0,
            b_is_dynamic_mfg_supported: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ReflexOptions {
    pub base: BaseStructure,
    pub mode: u32,
    pub frame_limit_us: u32,
    pub use_markers_to_optimize: bool,
    pub virtual_key: u16,
    pub id_thread: u32,
}

impl Default for ReflexOptions {
    fn default() -> Self {
        Self {
            base: BaseStructure::new(GUID_REFLEX_OPTIONS, K_STRUCT_VERSION_1),
            mode: REFLEX_MODE_OFF,
            frame_limit_us: 0,
            use_markers_to_optimize: false,
            virtual_key: 0,
            id_thread: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VulkanInfo {
    pub base: BaseStructure,
    pub device: *mut c_void,
    pub instance: *mut c_void,
    pub physical_device: *mut c_void,
    pub compute_queue_index: u32,
    pub compute_queue_family: u32,
    pub graphics_queue_index: u32,
    pub graphics_queue_family: u32,
    pub optical_flow_queue_index: u32,
    pub optical_flow_queue_family: u32,
    pub use_native_optical_flow_mode: bool,
    pub compute_queue_create_flags: u32,
    pub graphics_queue_create_flags: u32,
    pub optical_flow_queue_create_flags: u32,
}

impl Default for VulkanInfo {
    fn default() -> Self {
        Self {
            base: BaseStructure::new(GUID_VULKAN_INFO, K_STRUCT_VERSION_3),
            device: std::ptr::null_mut(),
            instance: std::ptr::null_mut(),
            physical_device: std::ptr::null_mut(),
            compute_queue_index: 0,
            compute_queue_family: 0,
            graphics_queue_index: 0,
            graphics_queue_family: 0,
            optical_flow_queue_index: 0,
            optical_flow_queue_family: 0,
            use_native_optical_flow_mode: false,
            compute_queue_create_flags: 0,
            graphics_queue_create_flags: 0,
            optical_flow_queue_create_flags: 0,
        }
    }
}

pub const GUID_DLSS_OPTIONS: StructType = StructType {
    data1: 0x6ac8_26e4,
    data2: 0x4c61,
    data3: 0x4101,
    data4: [0xa9, 0x2d, 0x63, 0x8d, 0x42, 0x10, 0x57, 0xb8],
};
pub const GUID_DLSS_OPTIMAL_SETTINGS: StructType = StructType {
    data1: 0xef1d_0957,
    data2: 0xfd58,
    data3: 0x4df7,
    data4: [0xb5, 0x04, 0x8b, 0x69, 0xd8, 0xaa, 0x6b, 0x76],
};
pub const GUID_DLSSD_OPTIONS: StructType = StructType {
    data1: 0x0ad8_7504,
    data2: 0x774e,
    data3: 0x4bf3,
    data4: [0x96, 0x33, 0xa4, 0x4d, 0x1f, 0x7f, 0x9c, 0xb8],
};
pub const GUID_DLSSD_OPTIMAL_SETTINGS: StructType = StructType {
    data1: 0xfbd0_c637,
    data2: 0xa28f,
    data3: 0x41f2,
    data4: [0xbc, 0x91, 0xb4, 0x21, 0xfa, 0xee, 0x8e, 0x1e],
};

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DlssOptions {
    pub base: BaseStructure,
    pub mode: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub sharpness: f32,
    pub pre_exposure: f32,
    pub exposure_scale: f32,
    pub color_buffers_hdr: u8,
    pub indicator_invert_axis_x: u8,
    pub indicator_invert_axis_y: u8,
    pub dlaa_preset: u32,
    pub quality_preset: u32,
    pub balanced_preset: u32,
    pub performance_preset: u32,
    pub ultra_performance_preset: u32,
    pub ultra_quality_preset: u32,
    pub use_auto_exposure: u8,
    pub alpha_upscaling_enabled: u8,
}

impl Default for DlssOptions {
    fn default() -> Self {
        Self {
            base: BaseStructure::new(GUID_DLSS_OPTIONS, K_STRUCT_VERSION_3),
            mode: DLSS_MODE_OFF,
            output_width: u32::MAX,
            output_height: u32::MAX,
            sharpness: 0.0,
            pre_exposure: 1.0,
            exposure_scale: 1.0,
            color_buffers_hdr: BOOLEAN_TRUE,
            indicator_invert_axis_x: BOOLEAN_FALSE,
            indicator_invert_axis_y: BOOLEAN_FALSE,
            dlaa_preset: 0,
            quality_preset: 0,
            balanced_preset: 0,
            performance_preset: 0,
            ultra_performance_preset: 0,
            ultra_quality_preset: 0,
            use_auto_exposure: BOOLEAN_FALSE,
            alpha_upscaling_enabled: BOOLEAN_FALSE,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DlssOptimalSettings {
    pub base: BaseStructure,
    pub optimal_render_width: u32,
    pub optimal_render_height: u32,
    pub optimal_sharpness: f32,
    pub render_width_min: u32,
    pub render_height_min: u32,
    pub render_width_max: u32,
    pub render_height_max: u32,
}

impl Default for DlssOptimalSettings {
    fn default() -> Self {
        Self {
            base: BaseStructure::new(GUID_DLSS_OPTIMAL_SETTINGS, K_STRUCT_VERSION_1),
            optimal_render_width: 0,
            optimal_render_height: 0,
            optimal_sharpness: 0.0,
            render_width_min: 0,
            render_height_min: 0,
            render_width_max: 0,
            render_height_max: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DlssDOptions {
    pub base: BaseStructure,
    pub mode: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub sharpness: f32,
    pub pre_exposure: f32,
    pub exposure_scale: f32,
    pub color_buffers_hdr: u8,
    pub indicator_invert_axis_x: u8,
    pub indicator_invert_axis_y: u8,
    pub normal_roughness_mode: u32,
    pub world_to_camera_view: [[f32; 4]; 4],
    pub camera_view_to_world: [[f32; 4]; 4],
    pub alpha_upscaling_enabled: u8,
    pub dlaa_preset: u32,
    pub quality_preset: u32,
    pub balanced_preset: u32,
    pub performance_preset: u32,
    pub ultra_performance_preset: u32,
    pub ultra_quality_preset: u32,
}

impl Default for DlssDOptions {
    fn default() -> Self {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        Self {
            base: BaseStructure::new(GUID_DLSSD_OPTIONS, K_STRUCT_VERSION_3),
            mode: DLSS_MODE_OFF,
            output_width: u32::MAX,
            output_height: u32::MAX,
            sharpness: 0.0,
            pre_exposure: 1.0,
            exposure_scale: 1.0,
            color_buffers_hdr: BOOLEAN_TRUE,
            indicator_invert_axis_x: BOOLEAN_FALSE,
            indicator_invert_axis_y: BOOLEAN_FALSE,
            normal_roughness_mode: DLSSD_NORMAL_ROUGHNESS_PACKED,
            world_to_camera_view: identity,
            camera_view_to_world: identity,
            alpha_upscaling_enabled: BOOLEAN_FALSE,
            dlaa_preset: 0,
            quality_preset: 0,
            balanced_preset: 0,
            performance_preset: 0,
            ultra_performance_preset: 0,
            ultra_quality_preset: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DlssDOptimalSettings {
    pub base: BaseStructure,
    pub optimal_render_width: u32,
    pub optimal_render_height: u32,
    pub optimal_sharpness: f32,
    pub render_width_min: u32,
    pub render_height_min: u32,
    pub render_width_max: u32,
    pub render_height_max: u32,
}

impl Default for DlssDOptimalSettings {
    fn default() -> Self {
        Self {
            base: BaseStructure::new(GUID_DLSSD_OPTIMAL_SETTINGS, K_STRUCT_VERSION_1),
            optimal_render_width: 0,
            optimal_render_height: 0,
            optimal_sharpness: 0.0,
            render_width_min: 0,
            render_height_min: 0,
            render_width_max: 0,
            render_height_max: 0,
        }
    }
}

// Compile-time x86_64 MSVC/SysV layout assertions against Streamline 2.14.16 headers.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<StructType>() == 16);
    assert!(size_of::<BaseStructure>() == 32);
    assert!(size_of::<DlssOptions>() == 88);
    assert!(size_of::<DlssOptimalSettings>() == 64);
    assert!(size_of::<DlssDOptions>() == 224);
    assert!(size_of::<DlssDOptimalSettings>() == 64);
    assert!(size_of::<Preferences>() == 144);
    assert!(size_of::<ViewportHandle>() == 40);
    assert!(size_of::<AdapterInfo>() == 56);
    assert!(size_of::<SubresourceRange>() == 56);
    assert!(std::mem::offset_of!(SubresourceRange, aspect_mask) == 32);
    assert!(std::mem::offset_of!(SubresourceRange, base_mip_level) == 36);
    assert!(std::mem::offset_of!(SubresourceRange, level_count) == 40);
    assert!(std::mem::offset_of!(SubresourceRange, base_array_layer) == 44);
    assert!(std::mem::offset_of!(SubresourceRange, layer_count) == 48);
    assert!(size_of::<Resource>() == 112);
    assert!(size_of::<ResourceTag>() == 64);
    assert!(size_of::<DlssGOptions>() == 120);
    assert!(size_of::<DlssGState>() == 88);
    assert!(size_of::<ReflexOptions>() == 48);
    assert!(size_of::<VulkanInfo>() == 96);
};
