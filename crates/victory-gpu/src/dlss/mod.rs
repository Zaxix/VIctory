//! DLSS Super Resolution in place of the title's own upscaling
//! (`VICTORY_DLSS`), through NVIDIA Streamline, loaded at run time from a
//! folder of the user's (`VICTORY_DLSS_DIR`; `scripts\get-dlss.ps1` fetches
//! NVIDIA's release there).
//!
//! On Series X the title upscales with a temporal pass of its own. On
//! Series S it calls AMD's FidelityFX API instead (renderer 7): an upscale
//! dispatch names the command list, the colour, depth, motion vectors,
//! exposure and reactive mask, the output, the jitter and the camera. With
//! DLSS asked, the runner patches the title to take that path at the DLSS
//! mode's scale (`apps/victory/src/output.rs`), and `victory_hle`'s
//! `fidelityfx` module puts itself in front of the title's FidelityFX: an
//! upscale dispatch comes here ([`upscale`]), and DLSS is recorded at that
//! place of the title's list, from the same textures into the same output.
//! Anything missing (the folder, the card, a texture) leaves the title's
//! own FidelityFX upscaling to run, with a line in the log saying why.
//!
//! Streamline is used with manual hooking: nothing of D3D12 or DXGI is
//! hooked; it is given our device (`slSetD3DDevice`) and records into our
//! host command lists.
//!
//! Defeat's own: VIctory-NVIDIA (`streamline.rs`, `streamline/plan.rs`) was
//! read as a reference, as was DexNom's FidelityFX work it builds on.

pub(crate) mod abi;

use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::*};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Graphics::Direct3D12::*;
use windows::core::Interface;

use crate::object::Object;

const INTERPOSER: &str = "sl.interposer.dll";
/// The id NGX is given for a title without an NVIDIA application id.
const PROJECT_ID: &CStr = c"0a5c3f1e-8d27-4b6a-9e13-defea7000001";
const ENGINE_VERSION: &CStr = c"Defeat";
/// The one viewport DLSS works on.
const VIEWPORT: u32 = 1;

/// A DLSS mode: the render size over the output size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dlaa,
    Quality,
    Balanced,
    Performance,
    UltraPerformance,
}

impl Mode {
    /// `VICTORY_DLSS`'s values; `None` for off (or not one of them).
    pub fn from_setting(value: &str) -> Option<Mode> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "dlaa" => Mode::Dlaa,
            "1" | "quality" => Mode::Quality,
            "balanced" => Mode::Balanced,
            "performance" => Mode::Performance,
            "ultraperformance" | "ultra-performance" => Mode::UltraPerformance,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Dlaa => "dlaa",
            Mode::Quality => "quality",
            Mode::Balanced => "balanced",
            Mode::Performance => "performance",
            Mode::UltraPerformance => "ultraperformance",
        }
    }

    /// DLSS's render scale for the mode.
    pub fn scale(self) -> f32 {
        match self {
            Mode::Dlaa => 1.0,
            Mode::Quality => 2.0 / 3.0,
            Mode::Balanced => 0.58,
            Mode::Performance => 0.5,
            Mode::UltraPerformance => 1.0 / 3.0,
        }
    }

    /// FidelityFX's quality mode of the same scale (the title notes it).
    pub fn ffx_mode(self) -> u8 {
        match self {
            Mode::Dlaa => 0,
            Mode::Quality => 1,
            Mode::Balanced => 2,
            Mode::Performance => 3,
            Mode::UltraPerformance => 4,
        }
    }

    fn sl_mode(self) -> u32 {
        match self {
            Mode::Dlaa => abi::DLSS_DLAA,
            Mode::Quality => abi::DLSS_MAX_QUALITY,
            Mode::Balanced => abi::DLSS_BALANCED,
            Mode::Performance => abi::DLSS_MAX_PERFORMANCE,
            Mode::UltraPerformance => abi::DLSS_ULTRA_PERFORMANCE,
        }
    }
}

/// The mode `VICTORY_DLSS` asks for; `None` when off.
pub fn asked() -> Option<Mode> {
    std::env::var("VICTORY_DLSS").ok().as_deref().and_then(Mode::from_setting)
}

/// The library's exports.
struct Sl {
    init: abi::SlInit,
    is_feature_supported: abi::SlIsFeatureSupported,
    evaluate_feature: abi::SlEvaluateFeature,
    set_constants: abi::SlSetConstants,
    get_feature_function: abi::SlGetFeatureFunction,
    get_new_frame_token: abi::SlGetNewFrameToken,
    set_d3d_device: abi::SlSetD3DDevice,
    get_feature_version: abi::SlGetFeatureVersion,
}

static SL: OnceLock<Sl> = OnceLock::new();
static MODE: OnceLock<Mode> = OnceLock::new();

/// DLSS's own functions, fetched once the device is made.
struct Dlss {
    optimal: abi::SlDlssGetOptimalSettings,
    set_options: abi::SlDlssSetOptions,
}

static DLSS: OnceLock<Dlss> = OnceLock::new();
static READY: AtomicBool = AtomicBool::new(false);
/// Given up for the run (the driver kept refusing).
static GIVEN_UP: AtomicBool = AtomicBool::new(false);

/// Upscalings recorded, refused (the title's own ran), and failed.
static COUNTS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Loads Streamline from `dir` and starts it for DLSS (before the host
/// device is made). Fails soft: the error says why, and the title's own
/// upscaling stays.
pub fn init(dir: &Path, mode: Mode) -> Result<(), String> {
    let path = std::path::absolute(dir.join(INTERPOSER)).map_err(|e| e.to_string())?;
    if !path.is_file() {
        return Err(format!("no {INTERPOSER} in {} (scripts\\get-dlss.ps1 fetches NVIDIA's files)", dir.display()));
    }
    let sl = load(&path)?;
    let sl = SL.get_or_init(|| sl);
    let plugins = wide(dir);
    let paths = [plugins.as_ptr()];
    let features = [abi::FEATURE_DLSS];
    let verbose = std::env::var_os("VICTORY_DLSS_LOG").is_some_and(|v| v == "1");
    let preferences = abi::Preferences {
        base: abi::Base::new(abi::PREFERENCES, 1),
        show_console: false,
        log_level: if verbose { abi::LOG_VERBOSE } else { abi::LOG_DEFAULT },
        paths_to_plugins: paths.as_ptr(),
        num_paths_to_plugins: 1,
        path_to_logs_and_data: std::ptr::null(),
        allocate_callback: std::ptr::null(),
        release_callback: std::ptr::null(),
        log_message_callback: Some(log_message),
        // No downloads: only what is in the folder runs.
        flags: abi::DISABLE_CL_STATE_TRACKING | abi::DISABLE_DEBUG_TEXT | abi::USE_MANUAL_HOOKING,
        features_to_load: features.as_ptr(),
        num_features_to_load: features.len() as u32,
        application_id: 0,
        engine: abi::ENGINE_CUSTOM,
        engine_version: ENGINE_VERSION.as_ptr(),
        project_id: PROJECT_ID.as_ptr(),
        render_api: abi::RENDER_API_D3D12,
    };
    // SAFETY: filled preferences; the library copies what it keeps.
    let code = unsafe { (sl.init)(&preferences, abi::SDK_VERSION) };
    if code != 0 {
        return Err(format!("slInit: {} ({code})", abi::result_name(code)));
    }
    let _ = MODE.set(mode);
    tracing::info!(library = %path.display(), mode = mode.name(), "DLSS: Streamline 2.14 started (manual hooking, no downloads)");
    Ok(())
}

fn load(path: &Path) -> Result<Sl, String> {
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
    };
    let file = wide(path);
    // Its own folder and System32 only, never the current folder.
    // SAFETY: a NUL-terminated full path.
    let module = unsafe { LoadLibraryExW(file.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if module.is_null() {
        return Err(format!("{INTERPOSER} did not load ({})", std::io::Error::last_os_error()));
    }
    let find = |name: &CStr| {
        // SAFETY: a loaded module and a NUL-terminated name.
        unsafe { GetProcAddress(module, name.as_ptr().cast()) }
            .map(|f| f as *const c_void)
            .ok_or_else(|| format!("{INTERPOSER} has no {}", name.to_string_lossy()))
    };
    macro_rules! get {
        ($name:literal, $t:ty) => {
            // SAFETY: the export of that name has the header's signature.
            unsafe { std::mem::transmute::<*const c_void, $t>(find($name)?) }
        };
    }
    // The library stays loaded for the run.
    Ok(Sl {
        init: get!(c"slInit", abi::SlInit),
        is_feature_supported: get!(c"slIsFeatureSupported", abi::SlIsFeatureSupported),
        evaluate_feature: get!(c"slEvaluateFeature", abi::SlEvaluateFeature),
        set_constants: get!(c"slSetConstants", abi::SlSetConstants),
        get_feature_function: get!(c"slGetFeatureFunction", abi::SlGetFeatureFunction),
        get_new_frame_token: get!(c"slGetNewFrameToken", abi::SlGetNewFrameToken),
        set_d3d_device: get!(c"slSetD3DDevice", abi::SlSetD3DDevice),
        get_feature_version: get!(c"slGetFeatureVersion", abi::SlGetFeatureVersion),
    })
}

unsafe extern "C" fn log_message(kind: u32, message: *const c_char) {
    if message.is_null() {
        return;
    }
    // SAFETY: a NUL-terminated message for the call.
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let text = text.trim_end();
    match kind {
        abi::LOG_ERROR => tracing::error!(target: "streamline", "{text}"),
        // (Each plugin in the folder that was not asked for says so.)
        abi::LOG_WARN if !text.contains("not requested by the host") => tracing::warn!(target: "streamline", "{text}"),
        _ => tracing::debug!(target: "streamline", "{text}"),
    }
}

/// Whether DLSS was asked for and Streamline started: the title's
/// FidelityFX calls go through `victory_hle`'s module then.
pub fn serving() -> bool {
    MODE.get().is_some() && SL.get().is_some()
}

/// The host device is made: Streamline is given it, the card is asked
/// whether it runs DLSS, and DLSS's functions are fetched.
pub(crate) fn device_made(device: &ID3D12Device) {
    let (Some(sl), Some(_)) = (SL.get(), MODE.get()) else {
        return;
    };
    // SAFETY: our native device.
    let code = unsafe { (sl.set_d3d_device)(device.as_raw()) };
    if code != 0 {
        tracing::warn!("DLSS: slSetD3DDevice: {} ({code}); the title's own upscaling stays", abi::result_name(code));
        return;
    }
    // SAFETY: a plain query.
    let luid = unsafe { device.GetAdapterLuid() };
    let info = abi::AdapterInfo {
        base: abi::Base::new(abi::ADAPTER_INFO, 1),
        device_luid: std::ptr::from_ref(&luid).cast(),
        device_luid_size: size_of_val(&luid) as u32,
        vk_physical_device: std::ptr::null_mut(),
    };
    // SAFETY: an adapter description that lives through the call.
    let code = unsafe { (sl.is_feature_supported)(abi::FEATURE_DLSS, &info) };
    if code != 0 {
        tracing::warn!("DLSS: not supported on this card: {} ({code}); the title's own upscaling stays", abi::result_name(code));
        return;
    }
    let function = |name: &CStr| -> Option<*mut c_void> {
        let mut f = std::ptr::null_mut();
        // SAFETY: a NUL-terminated name and an out pointer.
        let code = unsafe { (sl.get_feature_function)(abi::FEATURE_DLSS, name.as_ptr(), &mut f) };
        if code != 0 || f.is_null() {
            tracing::warn!("DLSS: no {} ({}, {code})", name.to_string_lossy(), abi::result_name(code));
            return None;
        }
        Some(f)
    };
    let (Some(optimal), Some(set_options)) = (function(c"slDLSSGetOptimalSettings"), function(c"slDLSSSetOptions")) else {
        return;
    };
    // SAFETY: the functions of those names have the header's signatures.
    let dlss = unsafe {
        Dlss {
            optimal: std::mem::transmute::<*mut c_void, abi::SlDlssGetOptimalSettings>(optimal),
            set_options: std::mem::transmute::<*mut c_void, abi::SlDlssSetOptions>(set_options),
        }
    };
    let _ = DLSS.set(dlss);
    let mut version = abi::FeatureVersion { base: abi::Base::new(abi::FEATURE_VERSION, 1), sl: abi::Version::default(), ngx: abi::Version::default() };
    // SAFETY: a structure to fill.
    let _ = unsafe { (sl.get_feature_version)(abi::FEATURE_DLSS, &mut version) };
    READY.store(true, Release);
    tracing::info!(
        streamline = format_args!("{}.{}.{}", version.sl.major, version.sl.minor, version.sl.build),
        ngx = format_args!("{}.{}.{}", version.ngx.major, version.ngx.minor, version.ngx.build),
        "DLSS: ready on this card; the title's upscalings are taken"
    );
}

/// The textures of an upscaling, as FidelityFX names them.
pub const COLOUR: usize = 0;
pub const DEPTH: usize = 1;
pub const MOTION: usize = 2;
pub const EXPOSURE: usize = 3;
pub const REACTIVE: usize = 4;
pub const OUTPUT: usize = 6;

// FidelityFX's upscale context flags (`FFX_UPSCALE_ENABLE_*`).
pub const FFX_HIGH_DYNAMIC_RANGE: u32 = 1 << 0;
pub const FFX_DISPLAY_RESOLUTION_MOTION_VECTORS: u32 = 1 << 1;
pub const FFX_MOTION_VECTORS_JITTER_CANCELLATION: u32 = 1 << 2;
pub const FFX_DEPTH_INVERTED: u32 = 1 << 3;
pub const FFX_AUTO_EXPOSURE: u32 = 1 << 5;

/// What an upscaling says of the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub jitter: [f32; 2],
    /// Multiplies the motion vectors into render pixels (FidelityFX's).
    pub motion_scale: [f32; 2],
    pub render: [u32; 2],
    pub output: [u32; 2],
    pub near: f32,
    pub far: f32,
    pub fov_y: f32,
    pub reset: bool,
    /// The context's creation flags (`FFX_*`).
    pub flags: u32,
}

/// One upscale dispatch of the title's: its context, its command list, its
/// textures (title resources) with their FidelityFX states, the camera.
#[derive(Clone, Copy, Debug)]
pub struct Upscale {
    pub context: usize,
    pub list: *const Object,
    pub resources: [(*const Object, u32); 7],
    pub camera: Camera,
    pub pre_exposure: f32,
}

// SAFETY: title objects, used on the title's thread that passed them.
unsafe impl Send for Upscale {}

/// Takes an upscaling: DLSS is recorded in its place in the title's list.
/// `Err` says why not (the title's own runs then).
pub fn upscale(up: &Upscale) -> Result<(), &'static str> {
    if GIVEN_UP.load(Relaxed) {
        return Err("DLSS gave up for this run (the driver refused its upscalings)");
    }
    if !READY.load(Acquire) {
        return Err("DLSS is not ready (no Streamline, no device yet, or the card does not run it)");
    }
    if [COLOUR, DEPTH, MOTION, OUTPUT].iter().any(|&i| up.resources[i].0.is_null()) {
        return Err("a texture DLSS needs (colour, depth, motion vectors, output) is missing");
    }
    if up.camera.render.contains(&0) || up.camera.output.contains(&0) {
        return Err("an empty render or output size");
    }
    let up = *up;
    let mut failed: Option<String> = None;
    let ran = crate::list::external(up.list, |host| match evaluate(host, &up) {
        Ok(()) => true,
        Err(why) => {
            failed = Some(why);
            false
        }
    });
    match (ran, failed) {
        (false, _) => Err("the title's list records nowhere (no host list)"),
        (true, Some(why)) => {
            let failures = COUNTS[2].fetch_add(1, Relaxed) + 1;
            if failures <= 8 {
                tracing::warn!("DLSS: the upscaling was not recorded: {why}; the title's own runs");
            }
            let done = COUNTS[0].load(Relaxed);
            if failures >= 60 && done == 0 {
                GIVEN_UP.store(true, Relaxed);
                tracing::warn!("DLSS: 60 upscalings failed and none was recorded: the title's own upscaling for the rest of the run");
            }
            Err("DLSS could not record it (see the line before)")
        }
        (true, None) => Ok(()),
    }
}

/// An upscaling the title's own ran for.
pub fn refused() {
    COUNTS[1].fetch_add(1, Relaxed);
}

/// The FidelityFX states as D3D12 states (as the PC backend reads them);
/// `None` for ones a texture DLSS reads cannot be in.
fn host_state(ffx: u32) -> Option<u32> {
    Some(match ffx {
        0x1 | 0x80 => 0,
        0x2 => 0x8,
        0x4 => 0x40,
        0x8 => 0x80,
        0xc => 0xc0,
        0x10 => 0x800,
        0x20 => 0x400,
        0x100 => 0x4,
        0x200 => 0x10,
        _ => return None,
    })
}

/// The D3D12 state an input is given to DLSS in: the title's if a shader
/// can read it there, else shader resource.
fn input_state(ffx: u32) -> u32 {
    host_state(ffx).filter(|&s| s & 0xc0 != 0).unwrap_or(0xc0)
}

/// The output: unordered access.
const OUTPUT_STATE: u32 = 0x8;

/// Everything DLSS's calls share (one at a time).
struct Calls {
    /// The last mode and sizes, for the log.
    last: Option<(u32, [u32; 2], [u32; 2])>,
    frame: u32,
}

static CALLS: Mutex<Calls> = Mutex::new(Calls { last: None, frame: 0 });

/// DLSS recorded into `host`, where the title's list is now.
fn evaluate(host: &crate::host::HostList, up: &Upscale) -> Result<(), String> {
    let (Some(sl), Some(dlss), Some(&mode)) = (SL.get(), DLSS.get(), MODE.get()) else {
        return Err("Streamline is not ready".into());
    };
    let camera = in_order(up.camera);
    let reactive = std::env::var_os("VICTORY_DLSS_REACTIVE").is_none_or(|v| v != "0");
    let used = |i: usize| {
        !up.resources[i].0.is_null() && (i != REACTIVE || reactive) && (i != EXPOSURE || camera.flags & FFX_AUTO_EXPOSURE == 0)
    };
    let mut textures: [Option<(ID3D12Resource, u32)>; 7] = Default::default();
    for i in [COLOUR, DEPTH, MOTION, EXPOSURE, REACTIVE, OUTPUT] {
        if !used(i) {
            continue;
        }
        let (resource, state) = up.resources[i];
        let resource = resource as crate::object::This;
        let Some(texture) = crate::resource::host_resource(resource).cloned() else {
            return Err(format!("texture {i} has no host copy"));
        };
        if i != OUTPUT {
            // As a compute shader's input would be: what the title wrote
            // into its memory is uploaded first, a depth twin brought in.
            if crate::texture::title_written(&texture) {
                crate::texture::upload(host, resource, &texture);
            }
            host.twin_use(resource, false, false);
        }
        let wanted = if i == OUTPUT { OUTPUT_STATE } else { input_state(state) };
        crate::list::ensure_state(host, resource, wanted);
        let legal = crate::host::legal_state(&texture, wanted);
        if legal != wanted {
            return Err(if i == OUTPUT { "the output cannot be written by a shader".into() } else { format!("input {i} cannot be read by a shader") });
        }
        textures[i] = Some((texture, legal));
    }
    let mut calls = CALLS.lock().unwrap_or_else(|e| e.into_inner());
    let output = camera.output;
    let sl_mode = mode.sl_mode();
    let mut optimal = abi::DlssOptimal::default();
    let options = abi::DlssOptions::new(sl_mode, output);
    // SAFETY: filled options and a structure to fill.
    let _ = unsafe { (dlss.optimal)(&options, &mut optimal) };
    let mut options = options;
    options.pre_exposure = if up.pre_exposure > 0.0 { up.pre_exposure } else { 1.0 };
    options.color_buffers_hdr = abi::boolean(camera.flags & FFX_HIGH_DYNAMIC_RANGE != 0);
    options.use_auto_exposure = abi::boolean(textures[EXPOSURE].is_none());
    let viewport = abi::Viewport::new(VIEWPORT);
    // SAFETY: filled options for our viewport.
    let code = unsafe { (dlss.set_options)(&viewport, &options) };
    if code != 0 {
        return Err(format!("slDLSSSetOptions: {} ({code})", abi::result_name(code)));
    }
    calls.frame = calls.frame.wrapping_add(1);
    let index = calls.frame;
    let mut token: *mut abi::FrameToken = std::ptr::null_mut();
    // SAFETY: an out pointer and the frame's number.
    let code = unsafe { (sl.get_new_frame_token)(&mut token, &index) };
    if code != 0 || token.is_null() {
        return Err(format!("slGetNewFrameToken: {} ({code})", abi::result_name(code)));
    }
    let constants = constants(&camera, multiplier("VICTORY_DLSS_JITTER"), multiplier("VICTORY_DLSS_MV"));
    // SAFETY: constants for this frame and viewport.
    let code = unsafe { (sl.set_constants)(&constants, token, &viewport) };
    if code != 0 {
        return Err(format!("slSetConstants: {} ({code})", abi::result_name(code)));
    }
    let kinds = [
        (COLOUR, abi::INPUT_COLOR, camera.render),
        (DEPTH, abi::DEPTH, camera.render),
        (MOTION, abi::MOTION_VECTORS, motion_size(&camera)),
        (EXPOSURE, abi::EXPOSURE, [1, 1]),
        (REACTIVE, abi::BIAS_CURRENT_COLOR_HINT, camera.render),
        (OUTPUT, abi::OUTPUT_COLOR, output),
    ];
    let mut resources: Vec<(abi::Resource, u32, [u32; 2])> = kinds
        .iter()
        .filter_map(|&(i, kind, size)| textures[i].as_ref().map(|(t, state)| (abi::Resource::texture(t.as_raw(), *state), kind, size)))
        .collect();
    let tags: Vec<abi::ResourceTag> = resources
        .iter_mut()
        .map(|(r, kind, size)| abi::ResourceTag {
            base: abi::Base::new(abi::RESOURCE_TAG, 1),
            resource: r,
            kind: *kind,
            lifecycle: abi::VALID_UNTIL_EVALUATE,
            extent: abi::Extent { top: 0, left: 0, width: size[0], height: size[1] },
        })
        .collect();
    let mut inputs: Vec<*const abi::Base> = vec![std::ptr::from_ref(&viewport.base)];
    inputs.extend(tags.iter().map(|t| std::ptr::from_ref(&t.base)));
    // SAFETY: an open command list of ours; inputs that live through the call.
    let code = unsafe { (sl.evaluate_feature)(abi::FEATURE_DLSS, token, inputs.as_ptr(), inputs.len() as u32, host.list.as_raw()) };
    if code != 0 {
        return Err(format!("slEvaluateFeature: {} ({code})", abi::result_name(code)));
    }
    // What DLSS wrote is on the host, not in title memory.
    crate::texture::host_written(up.resources[OUTPUT].0 as crate::object::This);
    let done = COUNTS[0].fetch_add(1, Relaxed) + 1;
    if done == 1 || calls.last != Some((sl_mode, camera.render, output)) {
        tracing::info!(
            mode = mode.name(),
            render = ?camera.render,
            output = ?output,
            optimal = ?[optimal.optimal_render_width, optimal.optimal_render_height],
            range = ?[optimal.render_width_min, optimal.render_height_min, optimal.render_width_max, optimal.render_height_max],
            hdr = camera.flags & FFX_HIGH_DYNAMIC_RANGE != 0,
            depth_inverted = camera.flags & FFX_DEPTH_INVERTED != 0,
            auto_exposure = textures[EXPOSURE].is_none(),
            reactive = textures[REACTIVE].is_some(),
            jitter = ?constants.jitter_offset,
            mvec_scale = ?constants.mvec_scale,
            near = camera.near,
            far = camera.far,
            fov = camera.fov_y,
            "DLSS: upscaling recorded in place of the title's"
        );
    }
    calls.last = Some((sl_mode, camera.render, output));
    Ok(())
}

/// `VICTORY_DLSS_JITTER`, `VICTORY_DLSS_MV`: `x,y` multipliers for a sign
/// or scale that turns out wrong; 1,1 unset.
fn multiplier(var: &str) -> [f32; 2] {
    std::env::var(var).ok().as_deref().map_or([1.0, 1.0], pair)
}

fn pair(value: &str) -> [f32; 2] {
    let mut parts = value.split(',').map(|p| p.trim().parse::<f32>());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Ok(x)), Some(Ok(y)), None) if x.is_finite() && y.is_finite() => [x, y],
        _ => [1.0, 1.0],
    }
}

/// The camera with its near plane first. FidelityFX takes the two either
/// way round (its upscaler takes the smaller as the near one); the title,
/// whose depth is inverted, names the far plane first. Streamline's
/// constants name each as it is.
fn in_order(camera: Camera) -> Camera {
    let mut camera = camera;
    if camera.near > camera.far && camera.far > 0.0 {
        (camera.near, camera.far) = (camera.far, camera.near);
    }
    camera
}

/// The size the motion vectors cover.
fn motion_size(camera: &Camera) -> [u32; 2] {
    if camera.flags & FFX_DISPLAY_RESOLUTION_MOTION_VECTORS != 0 { camera.output } else { camera.render }
}

/// The frame's constants. The title names no camera matrices: the
/// projection is made of its field of view, aspect and depth range; the
/// camera's motion is in the motion vectors, so the clip-to-previous-clip
/// matrices stay the identity.
fn constants(camera: &Camera, jitter: [f32; 2], mvec: [f32; 2]) -> abi::Constants {
    let aspect = camera.render[0].max(1) as f32 / camera.render[1].max(1) as f32;
    let inverted = camera.flags & FFX_DEPTH_INVERTED != 0;
    let view_to_clip = projection(camera.near, camera.far, camera.fov_y, aspect, inverted);
    let size = motion_size(camera);
    abi::Constants {
        base: abi::Base::new(abi::CONSTANTS, 2),
        camera_view_to_clip: view_to_clip,
        clip_to_camera_view: invert(&view_to_clip).unwrap_or(IDENTITY),
        clip_to_lens_clip: IDENTITY,
        clip_to_prev_clip: IDENTITY,
        prev_clip_to_clip: IDENTITY,
        jitter_offset: [camera.jitter[0] * jitter[0], camera.jitter[1] * jitter[1]],
        // Streamline wants motion in the vectors' own size's units; it
        // scales by that size before DLSS gets FidelityFX's scale back.
        mvec_scale: [0, 1].map(|i| camera.motion_scale[i] * mvec[i] / size[i].max(1) as f32),
        camera_pinhole_offset: [0.0, 0.0],
        camera_pos: [0.0, 0.0, 0.0],
        camera_up: [0.0, 1.0, 0.0],
        camera_right: [1.0, 0.0, 0.0],
        camera_fwd: [0.0, 0.0, 1.0],
        camera_near: camera.near,
        camera_far: camera.far,
        camera_fov: camera.fov_y,
        camera_aspect_ratio: aspect,
        motion_vectors_invalid_value: 0.0,
        depth_inverted: abi::boolean(inverted),
        camera_motion_included: abi::TRUE,
        motion_vectors_3d: abi::FALSE,
        reset: abi::boolean(camera.reset),
        orthographic_projection: abi::FALSE,
        motion_vectors_dilated: abi::FALSE,
        motion_vectors_jittered: abi::boolean(camera.flags & FFX_MOTION_VECTORS_JITTER_CANCELLATION != 0),
        min_relative_linear_depth_object_separation: 40.0,
    }
}

const IDENTITY: abi::Float4x4 = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

/// A left-handed perspective projection, row vectors: depth 0 at `near`
/// and 1 at `far`, or the other way with `inverted`; a far plane past 1e30
/// or not past `near` is at infinity. Anything degenerate: the identity.
fn projection(near: f32, far: f32, fov_y: f32, aspect: f32, inverted: bool) -> abi::Float4x4 {
    if !(near > 0.0 && fov_y > 0.0 && fov_y < std::f32::consts::PI && aspect > 0.0) {
        return IDENTITY;
    }
    let y = 1.0 / (fov_y * 0.5).tan();
    let x = y / aspect;
    let infinite = !far.is_finite() || far > 1e30 || far <= near;
    let (a, b) = match (infinite, inverted) {
        (false, false) => (far / (far - near), -near * far / (far - near)),
        (false, true) => (near / (near - far), far * near / (far - near)),
        (true, false) => (1.0, -near),
        (true, true) => (0.0, near),
    };
    [[x, 0.0, 0.0, 0.0], [0.0, y, 0.0, 0.0], [0.0, 0.0, a, 1.0], [0.0, 0.0, b, 0.0]]
}

/// A 4x4 inverse by Gauss-Jordan elimination; `None` if singular.
fn invert(m: &abi::Float4x4) -> Option<abi::Float4x4> {
    let mut a: [[f64; 8]; 4] = std::array::from_fn(|r| std::array::from_fn(|c| if c < 4 { f64::from(m[r][c]) } else if c - 4 == r { 1.0 } else { 0.0 }));
    for col in 0..4 {
        let pivot = (col..4).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        let p = a[col][col];
        for v in &mut a[col] {
            *v /= p;
        }
        let pivot_row = a[col];
        for (r, row) in a.iter_mut().enumerate() {
            let f = row[col];
            if r != col && f != 0.0 {
                for (v, p) in row.iter_mut().zip(pivot_row) {
                    *v -= f * p;
                }
            }
        }
    }
    Some(std::array::from_fn(|r| std::array::from_fn(|c| a[r][c + 4] as f32)))
}

/// The line every few seconds: upscalings by DLSS, by the title's own.
pub(crate) fn line(frames: f64) {
    if !serving() {
        return;
    }
    let [done, refused, failed] = [0, 1, 2].map(|i| COUNTS[i].swap(0, Relaxed));
    tracing::info!(
        "DLSS per frame: {:.2} upscalings by DLSS, {:.2} by the title's own ({} failed in these frames){}",
        done as f64 / frames,
        refused as f64 / frames,
        failed,
        if GIVEN_UP.load(Relaxed) { "; DLSS gave up for this run" } else { "" }
    );
}

/// Where NVIDIA's files are looked for without `VICTORY_DLSS_DIR`.
pub fn default_dir(data: &Path) -> PathBuf {
    data.join("streamline")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_read_and_scale() {
        assert_eq!(Mode::from_setting("Quality"), Some(Mode::Quality));
        assert_eq!(Mode::from_setting("1"), Some(Mode::Quality));
        assert_eq!(Mode::from_setting("off"), None);
        assert_eq!(Mode::from_setting("0"), None);
        assert_eq!(Mode::from_setting("ultra-performance"), Some(Mode::UltraPerformance));
        // At a 1080p output, quality draws 1280x720.
        assert_eq!((1920.0 * Mode::Quality.scale()) as u32, 1280);
        assert_eq!((1080.0 * Mode::Quality.scale()) as u32, 720);
        for mode in [Mode::Dlaa, Mode::Quality, Mode::Balanced, Mode::Performance, Mode::UltraPerformance] {
            assert_eq!(Mode::from_setting(mode.name()), Some(mode));
        }
    }

    #[test]
    fn states_read_as_d3d12_states() {
        assert_eq!(input_state(0xc), 0xc0);
        assert_eq!(input_state(0x4), 0x40);
        // A copy source or the common state is not one a shader reads in.
        assert_eq!(input_state(0x10), 0xc0);
        assert_eq!(input_state(0x1), 0xc0);
        assert_eq!(host_state(0x2), Some(0x8));
        assert_eq!(host_state(0x40), None);
    }

    fn camera() -> Camera {
        Camera {
            jitter: [0.25, -0.5],
            motion_scale: [1280.0, 720.0],
            render: [1280, 720],
            output: [1920, 1080],
            near: 10000.0,
            far: 0.15,
            fov_y: 1.0,
            reset: true,
            flags: FFX_DEPTH_INVERTED | FFX_HIGH_DYNAMIC_RANGE,
        }
    }

    #[test]
    fn constants_follow_the_upscaling() {
        let c = in_order(camera());
        assert_eq!((c.near, c.far), (0.15, 10000.0));
        let k = constants(&c, [1.0, 1.0], [1.0, 1.0]);
        assert_eq!(k.jitter_offset, [0.25, -0.5]);
        assert_eq!(k.mvec_scale, [1.0, 1.0]);
        assert_eq!((k.depth_inverted, k.reset), (abi::TRUE, abi::TRUE));
        assert!((k.camera_aspect_ratio - 16.0 / 9.0).abs() < 1e-6);
        let k = constants(&c, [-1.0, 1.0], [2.0, 2.0]);
        assert_eq!(k.jitter_offset, [-0.25, -0.5]);
        assert_eq!(k.mvec_scale, [2.0, 2.0]);
        assert_eq!(pair("1, -1"), [1.0, -1.0]);
        assert_eq!(pair("nonsense"), [1.0, 1.0]);
    }

    /// A point at the near plane lands at depth 0 (1 inverted), one at
    /// the far plane at 1 (0 inverted); the inverse undoes the projection.
    #[test]
    fn projections_place_near_and_far() {
        let depth = |m: &abi::Float4x4, z: f32| {
            let (zc, wc) = (z * m[2][2] + m[3][2], z * m[2][3] + m[3][3]);
            zc / wc
        };
        let m = projection(0.15, 10000.0, 1.0, 16.0 / 9.0, false);
        assert!(depth(&m, 0.15).abs() < 1e-4 && (depth(&m, 10000.0) - 1.0).abs() < 1e-4);
        let m = projection(0.15, 10000.0, 1.0, 16.0 / 9.0, true);
        assert!((depth(&m, 0.15) - 1.0).abs() < 1e-4 && depth(&m, 10000.0).abs() < 1e-4);
        let inv = invert(&m).unwrap();
        for (r, row) in m.iter().enumerate() {
            for c in 0..4 {
                let v: f32 = row.iter().zip(&inv).map(|(x, inv_row)| x * inv_row[c]).sum();
                assert!((v - if r == c { 1.0 } else { 0.0 }).abs() < 1e-3, "{r},{c}: {v}");
            }
        }
        assert_eq!(projection(0.0, 1.0, 1.0, 1.0, false), IDENTITY);
    }
}
