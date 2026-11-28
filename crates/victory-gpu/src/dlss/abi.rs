//! What Defeat calls of NVIDIA Streamline's C ABI for DLSS Super
//! Resolution, written down from the SDK 2.14.16 headers (`include/sl.h`,
//! `sl_core_types.h`, `sl_consts.h`, `sl_struct.h`, `sl_core_api.h`,
//! `sl_dlss.h`; MIT licensed, github.com/NVIDIA-RTX/Streamline). Nothing of
//! NVIDIA's is in this repository: the library is loaded at run time from
//! the user's folder. VIctory-NVIDIA's `streamline/abi.rs` was read beside
//! the headers as a check.
//!
//! The C++ API passes structures by reference: in the ABI those are
//! pointers. Every structure starts with `sl::BaseStructure` (next, type
//! id, version).
#![allow(dead_code)]

use std::ffi::{c_char, c_void};

/// `sl::StructType`, a GUID.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StructType {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> StructType {
    StructType { data1, data2, data3, data4 }
}

/// `sl::BaseStructure`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Base {
    pub next: *mut Base,
    pub kind: StructType,
    pub version: usize,
}

impl Base {
    pub(crate) const fn new(kind: StructType, version: usize) -> Base {
        Base { next: std::ptr::null_mut(), kind, version }
    }
}

// The structure type ids, from the headers' `SL_STRUCT_BEGIN` lines.
pub(crate) const PREFERENCES: StructType = guid(0x1ca10965, 0xbf8e, 0x432b, [0x8d, 0xa1, 0x67, 0x16, 0xd8, 0x79, 0xfb, 0x14]);
pub(crate) const RESOURCE: StructType = guid(0x3a9d70cf, 0x2418, 0x4b72, [0x83, 0x91, 0x13, 0xf8, 0x72, 0x1c, 0x72, 0x61]);
pub(crate) const RESOURCE_TAG: StructType = guid(0x4c6a5aad, 0xb445, 0x496c, [0x87, 0xff, 0x1a, 0xf3, 0x84, 0x5b, 0xe6, 0x53]);
pub(crate) const CONSTANTS: StructType = guid(0xdcd35ad7, 0x4e4a, 0x4bad, [0xa9, 0x0c, 0xe0, 0xc4, 0x9e, 0xb2, 0x3a, 0xfe]);
pub(crate) const VIEWPORT: StructType = guid(0x171b6435, 0x9b3c, 0x4fc8, [0x99, 0x94, 0xfb, 0xe5, 0x25, 0x69, 0xaa, 0xa4]);
pub(crate) const ADAPTER_INFO: StructType = guid(0x0677315f, 0xa746, 0x4492, [0x9f, 0x42, 0xcb, 0x61, 0x42, 0xc9, 0xc3, 0xd4]);
pub(crate) const FEATURE_VERSION: StructType = guid(0x6d5b51f0, 0x076b, 0x486d, [0x99, 0x95, 0x5a, 0x56, 0x10, 0x43, 0xf5, 0xc1]);
pub(crate) const DLSS_OPTIONS: StructType = guid(0x6ac826e4, 0x4c61, 0x4101, [0xa9, 0x2d, 0x63, 0x8d, 0x42, 0x10, 0x57, 0xb8]);
pub(crate) const DLSS_OPTIMAL: StructType = guid(0xef1d0957, 0xfd58, 0x4df7, [0xb5, 0x04, 0x8b, 0x69, 0xd8, 0xaa, 0x6b, 0x76]);

/// `sl::kSDKVersion` of 2.14.16 (`sl_version.h`).
pub(crate) const SDK_VERSION: u64 = (2 << 48) | (14 << 32) | (16 << 16) | 0xfedc;

/// `sl::kFeatureDLSS`.
pub(crate) const FEATURE_DLSS: u32 = 0;

// `sl::BufferType`.
pub(crate) const DEPTH: u32 = 0;
pub(crate) const MOTION_VECTORS: u32 = 1;
pub(crate) const INPUT_COLOR: u32 = 3;
pub(crate) const OUTPUT_COLOR: u32 = 4;
pub(crate) const EXPOSURE: u32 = 13;
pub(crate) const BIAS_CURRENT_COLOR_HINT: u32 = 29;

/// `sl::ResourceType::eTex2d` (a char).
pub(crate) const TEX2D: u8 = 0;
/// `sl::ResourceLifecycle::eValidUntilEvaluate`.
pub(crate) const VALID_UNTIL_EVALUATE: u32 = 2;

// `sl::Boolean` (a char).
pub(crate) const FALSE: u8 = 0;
pub(crate) const TRUE: u8 = 1;

pub(crate) const fn boolean(value: bool) -> u8 {
    if value { TRUE } else { FALSE }
}

// `sl::PreferenceFlags`.
pub(crate) const DISABLE_CL_STATE_TRACKING: u64 = 1 << 0;
pub(crate) const DISABLE_DEBUG_TEXT: u64 = 1 << 1;
pub(crate) const USE_MANUAL_HOOKING: u64 = 1 << 2;

// `sl::LogLevel`, `sl::LogType`.
pub(crate) const LOG_DEFAULT: u32 = 1;
pub(crate) const LOG_VERBOSE: u32 = 2;
pub(crate) const LOG_WARN: u32 = 1;
pub(crate) const LOG_ERROR: u32 = 2;

/// `sl::EngineType::eCustom`, `sl::RenderAPI::eD3D12`.
pub(crate) const ENGINE_CUSTOM: u32 = 0;
pub(crate) const RENDER_API_D3D12: u32 = 1;

// `sl::DLSSMode`.
pub(crate) const DLSS_MAX_PERFORMANCE: u32 = 1;
pub(crate) const DLSS_BALANCED: u32 = 2;
pub(crate) const DLSS_MAX_QUALITY: u32 = 3;
pub(crate) const DLSS_ULTRA_PERFORMANCE: u32 = 4;
pub(crate) const DLSS_DLAA: u32 = 6;

/// `sl::Preferences` (version 1).
#[repr(C)]
pub(crate) struct Preferences {
    pub base: Base,
    pub show_console: bool,
    pub log_level: u32,
    pub paths_to_plugins: *const *const u16,
    pub num_paths_to_plugins: u32,
    pub path_to_logs_and_data: *const u16,
    pub allocate_callback: *const c_void,
    pub release_callback: *const c_void,
    pub log_message_callback: Option<LogCallback>,
    pub flags: u64,
    pub features_to_load: *const u32,
    pub num_features_to_load: u32,
    pub application_id: u32,
    pub engine: u32,
    pub engine_version: *const c_char,
    pub project_id: *const c_char,
    pub render_api: u32,
}

/// `PFun_LogMessageCallback`.
pub(crate) type LogCallback = unsafe extern "C" fn(kind: u32, message: *const c_char);

/// `sl::Resource` (version 1): for D3D12 the native resource and its state.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Resource {
    pub base: Base,
    pub kind: u8,
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
    /// A 2D texture in D3D12 state `state`.
    pub(crate) fn texture(native: *mut c_void, state: u32) -> Resource {
        Resource {
            base: Base::new(RESOURCE, 1),
            kind: TEX2D,
            native,
            memory: std::ptr::null_mut(),
            view: std::ptr::null_mut(),
            state,
            width: 0,
            height: 0,
            native_format: 0,
            mip_levels: 0,
            array_layers: 0,
            gpu_virtual_address: 0,
            flags: 0,
            usage: 0,
            internal_flags: 0,
            reserved: 0,
        }
    }
}

/// `sl::Extent`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Extent {
    pub top: u32,
    pub left: u32,
    pub width: u32,
    pub height: u32,
}

/// `sl::ResourceTag` (version 1).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ResourceTag {
    pub base: Base,
    pub resource: *mut Resource,
    pub kind: u32,
    pub lifecycle: u32,
    pub extent: Extent,
}

/// `sl::float4x4`: four rows.
pub(crate) type Float4x4 = [[f32; 4]; 4];

/// `sl::Constants` (version 2).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Constants {
    pub base: Base,
    pub camera_view_to_clip: Float4x4,
    pub clip_to_camera_view: Float4x4,
    pub clip_to_lens_clip: Float4x4,
    pub clip_to_prev_clip: Float4x4,
    pub prev_clip_to_clip: Float4x4,
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

/// `sl::ViewportHandle`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Viewport {
    pub base: Base,
    pub value: u32,
}

impl Viewport {
    pub(crate) const fn new(value: u32) -> Viewport {
        Viewport { base: Base::new(VIEWPORT, 1), value }
    }
}

/// `sl::FrameToken`: made by the library, only passed back to it.
#[repr(C)]
pub(crate) struct FrameToken {
    _opaque: [u8; 0],
}

/// `sl::Version`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Version {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
}

/// `sl::FeatureVersion`.
#[repr(C)]
pub(crate) struct FeatureVersion {
    pub base: Base,
    pub sl: Version,
    pub ngx: Version,
}

/// `sl::AdapterInfo`.
#[repr(C)]
pub(crate) struct AdapterInfo {
    pub base: Base,
    pub device_luid: *const u8,
    pub device_luid_size: u32,
    pub vk_physical_device: *mut c_void,
}

/// `sl::DLSSOptions` (version 3).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DlssOptions {
    pub base: Base,
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

impl DlssOptions {
    /// The header's defaults, with `mode` for an output of `output`.
    pub(crate) fn new(mode: u32, output: [u32; 2]) -> DlssOptions {
        DlssOptions {
            base: Base::new(DLSS_OPTIONS, 3),
            mode,
            output_width: output[0],
            output_height: output[1],
            sharpness: 0.0,
            pre_exposure: 1.0,
            exposure_scale: 1.0,
            color_buffers_hdr: TRUE,
            indicator_invert_axis_x: FALSE,
            indicator_invert_axis_y: FALSE,
            dlaa_preset: 0,
            quality_preset: 0,
            balanced_preset: 0,
            performance_preset: 0,
            ultra_performance_preset: 0,
            ultra_quality_preset: 0,
            use_auto_exposure: FALSE,
            alpha_upscaling_enabled: FALSE,
        }
    }
}

/// `sl::DLSSOptimalSettings` (version 1).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DlssOptimal {
    pub base: Base,
    pub optimal_render_width: u32,
    pub optimal_render_height: u32,
    pub optimal_sharpness: f32,
    pub render_width_min: u32,
    pub render_height_min: u32,
    pub render_width_max: u32,
    pub render_height_max: u32,
}

impl Default for DlssOptimal {
    fn default() -> DlssOptimal {
        DlssOptimal {
            base: Base::new(DLSS_OPTIMAL, 1),
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

// The exports of `sl.interposer.dll` (`sl_core_api.h`).
pub(crate) type SlInit = unsafe extern "C" fn(*const Preferences, u64) -> i32;
pub(crate) type SlShutdown = unsafe extern "C" fn() -> i32;
pub(crate) type SlIsFeatureSupported = unsafe extern "C" fn(u32, *const AdapterInfo) -> i32;
pub(crate) type SlEvaluateFeature = unsafe extern "C" fn(u32, *const FrameToken, *const *const Base, u32, *mut c_void) -> i32;
pub(crate) type SlFreeResources = unsafe extern "C" fn(u32, *const Viewport) -> i32;
pub(crate) type SlSetConstants = unsafe extern "C" fn(*const Constants, *const FrameToken, *const Viewport) -> i32;
pub(crate) type SlGetFeatureFunction = unsafe extern "C" fn(u32, *const c_char, *mut *mut c_void) -> i32;
pub(crate) type SlGetNewFrameToken = unsafe extern "C" fn(*mut *mut FrameToken, *const u32) -> i32;
pub(crate) type SlSetD3DDevice = unsafe extern "C" fn(*mut c_void) -> i32;
pub(crate) type SlGetFeatureVersion = unsafe extern "C" fn(u32, *mut FeatureVersion) -> i32;
// `sl_dlss.h`, through `slGetFeatureFunction`.
pub(crate) type SlDlssGetOptimalSettings = unsafe extern "C" fn(*const DlssOptions, *mut DlssOptimal) -> i32;
pub(crate) type SlDlssSetOptions = unsafe extern "C" fn(*const Viewport, *const DlssOptions) -> i32;

/// The name of an `sl::Result` (`sl_result.h`), for the log.
pub(crate) fn result_name(code: i32) -> &'static str {
    const NAMES: [&str; 43] = [
        "ok",
        "IO error",
        "driver out of date",
        "OS out of date",
        "hardware GPU scheduling off",
        "device not created",
        "no supported adapter",
        "adapter not supported",
        "no plugins",
        "Vulkan API error",
        "DXGI API error",
        "D3D API error",
        "NRD API error",
        "NVAPI error",
        "Reflex API error",
        "NGX failed",
        "JSON parsing error",
        "missing proxy",
        "missing resource state",
        "invalid integration",
        "missing input parameter",
        "not initialized",
        "compute failed",
        "init not called",
        "exception handler",
        "invalid parameter",
        "missing constants",
        "duplicated constants",
        "missing or invalid API",
        "common constants missing",
        "unsupported interface",
        "feature missing",
        "feature not supported",
        "feature missing hooks",
        "feature failed to load",
        "feature wrong priority",
        "feature missing dependency",
        "feature manager invalid state",
        "invalid state",
        "out of VRAM (warning)",
        "?",
        "?",
        "?",
    ];
    usize::try_from(code).ok().and_then(|i| NAMES.get(i)).copied().unwrap_or("?")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// The layouts the C++ compiler gives these structures on x64.
    #[test]
    fn structures_lay_out_as_the_headers() {
        assert_eq!(size_of::<StructType>(), 16);
        assert_eq!(size_of::<Base>(), 32);
        assert_eq!(offset_of!(Preferences, show_console), 32);
        assert_eq!(offset_of!(Preferences, log_level), 36);
        assert_eq!(offset_of!(Preferences, paths_to_plugins), 40);
        assert_eq!(offset_of!(Preferences, flags), 88);
        assert_eq!(offset_of!(Preferences, render_api), 136);
        assert_eq!(offset_of!(Resource, kind), 32);
        assert_eq!(offset_of!(Resource, native), 40);
        assert_eq!(offset_of!(Resource, state), 64);
        assert_eq!(offset_of!(Resource, gpu_virtual_address), 88);
        assert_eq!(size_of::<Resource>(), 112);
        assert_eq!(offset_of!(ResourceTag, resource), 32);
        assert_eq!(offset_of!(ResourceTag, extent), 48);
        assert_eq!(size_of::<ResourceTag>(), 64);
        assert_eq!(offset_of!(Constants, camera_view_to_clip), 32);
        assert_eq!(offset_of!(Constants, jitter_offset), 32 + 5 * 64);
        assert_eq!(offset_of!(Constants, camera_near), 32 + 5 * 64 + 3 * 8 + 4 * 12);
        assert_eq!(offset_of!(Constants, depth_inverted), 32 + 5 * 64 + 3 * 8 + 4 * 12 + 5 * 4);
        assert_eq!(offset_of!(Constants, min_relative_linear_depth_object_separation), 32 + 5 * 64 + 3 * 8 + 4 * 12 + 5 * 4 + 8);
        assert_eq!(offset_of!(Viewport, value), 32);
        assert_eq!(offset_of!(DlssOptions, mode), 32);
        assert_eq!(offset_of!(DlssOptions, color_buffers_hdr), 56);
        assert_eq!(offset_of!(DlssOptions, dlaa_preset), 60);
        assert_eq!(offset_of!(DlssOptions, use_auto_exposure), 84);
        assert_eq!(offset_of!(DlssOptimal, optimal_render_width), 32);
    }

    #[test]
    fn results_are_named() {
        assert_eq!(result_name(0), "ok");
        assert_eq!(result_name(15), "NGX failed");
        assert_eq!(result_name(32), "feature not supported");
        assert_eq!(result_name(-1), "?");
    }
}
