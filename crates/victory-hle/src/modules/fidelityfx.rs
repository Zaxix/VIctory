//! `amd_fidelityfx_loader_xbox.dll`: the title's AMD FidelityFX API, five
//! functions over chains of descriptions (each starts with a type id and
//! the next one's address). With DLSS (`victory_gpu::dlss`, `VICTORY_DLSS`)
//! the title's imports of the five are bound to the functions here
//! ([`Interpose`]); they call the title's own DLL for everything but an
//! upscale dispatch DLSS takes. Without DLSS the imports bind as before.
//!
//! The description layouts are those of the FidelityFX SDK's API (MIT), as
//! DexNom's fork read them for this title and VIctory-NVIDIA carried them;
//! this is Defeat's own interposer.
#![allow(non_snake_case)]
// Title code calls these through its import table.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use victory_gpu::dlss;
use victory_gpu::object::Object;
use victory_loader::{ImportResolver, Provenance, Resolution};
use victory_pe::ImportSymbol;

/// The DLL the title imports the API from.
pub const LOADER_DLL: &str = "amd_fidelityfx_loader_xbox.dll";

const OK: u32 = 0;
/// `FFX_API_RETURN_ERROR_PARAMETER`.
const PARAMETER: u32 = 6;

/// `FFX_API_CREATE_CONTEXT_DESC_TYPE_UPSCALE`, `..._DISPATCH_DESC_TYPE_UPSCALE`.
const CREATE_UPSCALE: u64 = 0x1_0000;
const DISPATCH_UPSCALE: u64 = 0x1_0001;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Header {
    kind: u64,
    next: *mut Header,
}

/// `ffxCreateContextDescUpscale`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CreateUpscale {
    header: Header,
    flags: u32,
    max_render: [u32; 2],
    max_output: [u32; 2],
    message: usize,
}

/// `FfxApiResource`: the resource, its description, its state.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Resource {
    resource: *mut c_void,
    description: [u32; 8],
    state: u32,
}

/// `ffxDispatchDescUpscale`: the command list; colour, depth, motion
/// vectors, exposure, reactive mask, transparency mask, output; jitter,
/// motion-vector scale, sizes, sharpening, frame time, pre-exposure,
/// reset, camera, flags (the C `bool`s as bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct DispatchUpscale {
    header: Header,
    list: *mut c_void,
    resources: [Resource; 7],
    jitter: [f32; 2],
    motion_scale: [f32; 2],
    render_size: [u32; 2],
    upscale_size: [u32; 2],
    enable_sharpening: u8,
    sharpness: f32,
    frame_time_delta: f32,
    pre_exposure: f32,
    reset: u8,
    camera_near: f32,
    camera_far: f32,
    camera_fov_angle_vertical: f32,
    view_space_to_meters_factor: f32,
    flags: u32,
}

const _: () = assert!(size_of::<CreateUpscale>() == 0x30);
const _: () = assert!(size_of::<Resource>() == 0x30);
const _: () = assert!(size_of::<DispatchUpscale>() == 0x1b0);

type Create = unsafe extern "C" fn(*mut *mut c_void, *mut Header, *const c_void) -> u32;
type Destroy = unsafe extern "C" fn(*mut *mut c_void, *const c_void) -> u32;
/// Configure, query and dispatch.
type Call = unsafe extern "C" fn(*mut *mut c_void, *mut Header) -> u32;

const NAMES: [&str; 5] = ["ffxCreateContext", "ffxDestroyContext", "ffxConfigure", "ffxQuery", "ffxDispatch"];
/// The title's own five, by [`NAMES`].
static ORIGINALS: [AtomicUsize; 5] = [const { AtomicUsize::new(0) }; 5];

/// An upscaler context the title made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Made {
    flags: u32,
    max_output: [u32; 2],
    /// Made at the title's shipping sizes, not the ones asked: its own
    /// upscaling must not run at the size asked.
    fitted: bool,
}

static CONTEXTS: Mutex<Option<HashMap<usize, Made>>> = Mutex::new(None);

fn contexts<T>(f: impl FnOnce(&mut HashMap<usize, Made>) -> T) -> T {
    let mut contexts = CONTEXTS.lock().unwrap_or_else(|e| e.into_inner());
    f(contexts.get_or_insert_with(HashMap::new))
}

/// Binds the title's imports of [`LOADER_DLL`] to the functions here when
/// DLSS is on, keeping what `inner` binds them to (the title's own DLL)
/// for them to call; otherwise binds as `inner` does.
pub struct Interpose<'a> {
    pub inner: &'a dyn ImportResolver,
}

impl ImportResolver for Interpose<'_> {
    fn resolve(&self, dll: &str, symbol: &ImportSymbol) -> Resolution {
        let found = self.inner.resolve(dll, symbol);
        if !dll.eq_ignore_ascii_case(LOADER_DLL) || !dlss::serving() {
            return found;
        }
        match (&found, symbol) {
            (&Resolution::Found { address, .. }, ImportSymbol::Name { name, .. }) => match interpose(name, address) {
                Some(ours) => Resolution::Found { address: ours, provenance: Provenance::Hle },
                None => found,
            },
            _ => found,
        }
    }
}

/// Keeps `original` as the title's `name` and gives ours, if we have one.
fn interpose(name: &str, original: usize) -> Option<usize> {
    let i = NAMES.iter().position(|n| *n == name)?;
    ORIGINALS[i].store(original, Relaxed);
    let ours = [
        ffxCreateContext as *const () as usize,
        ffxDestroyContext as *const () as usize,
        ffxConfigure as *const () as usize,
        ffxQuery as *const () as usize,
        ffxDispatch as *const () as usize,
    ][i];
    tracing::info!(name, "FidelityFX: the title's import goes through ours (DLSS)");
    Some(ours)
}

fn original(i: usize) -> Option<*const ()> {
    let address = ORIGINALS[i].load(Relaxed);
    (address != 0).then_some(address as *const ())
}

/// The sizes the title's shipping FidelityFX (FSR 4) takes: its fixed
/// shader tables are the Series S's, 1280x720 or 1920x1080 in, 1920x1080
/// out; other sizes fail the context, and the title, which does not look,
/// divides by zero at its first upscaling (VIctory-NVIDIA's finding).
const SHIPPING_RENDER: [[u32; 2]; 2] = [[1280, 720], [1920, 1080]];
const SHIPPING_OUTPUT: [u32; 2] = [1920, 1080];

/// The sizes to make the title's own context at, if not those asked.
fn shipping(render: [u32; 2], output: [u32; 2]) -> Option<([u32; 2], [u32; 2])> {
    (!(SHIPPING_RENDER.contains(&render) && output == SHIPPING_OUTPUT)).then_some((SHIPPING_RENDER[0], SHIPPING_OUTPUT))
}

macro_rules! once {
    ($level:ident, $($arg:tt)*) => {{
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| tracing::$level!($($arg)*));
    }};
}

extern "C" fn ffxCreateContext(context: *mut *mut c_void, desc: *mut Header, memory: *const c_void) -> u32 {
    let Some(create) = original(0) else { return PARAMETER };
    // SAFETY: the title's own function, with its arguments.
    let call = |desc: *mut Header| unsafe { std::mem::transmute::<*const (), Create>(create)(context, desc, memory) };
    // SAFETY: the title passes a chain of descriptions.
    if desc.is_null() || unsafe { (*desc).kind } != CREATE_UPSCALE {
        return call(desc);
    }
    // SAFETY: an upscaler description, by its type.
    let asked = unsafe { desc.cast::<CreateUpscale>().read() };
    // With DLSS taking the upscalings the title's own context only has to
    // exist: it is made at sizes its shipping tables take.
    let fit = shipping(asked.max_render, asked.max_output);
    let code = match fit {
        Some((render, output)) => {
            let mut fitted = asked;
            fitted.max_render = render;
            fitted.max_output = output;
            // SAFETY: the title's description, rewritten for the call and put back.
            unsafe { desc.cast::<CreateUpscale>().write(fitted) };
            let code = call(desc);
            // SAFETY: as above.
            unsafe { desc.cast::<CreateUpscale>().write(asked) };
            code
        }
        None => call(desc),
    };
    tracing::info!(
        code,
        flags = format_args!("{:#x}", asked.flags),
        max_render = ?asked.max_render,
        max_output = ?asked.max_output,
        made_at_shipping_sizes = fit.is_some(),
        "FidelityFX: the title made its upscaler"
    );
    if code == OK && !context.is_null() {
        // SAFETY: the context was made.
        let handle = unsafe { *context } as usize;
        contexts(|c| c.insert(handle, Made { flags: asked.flags, max_output: asked.max_output, fitted: fit.is_some() }));
    }
    code
}

extern "C" fn ffxDestroyContext(context: *mut *mut c_void, memory: *const c_void) -> u32 {
    let Some(destroy) = original(1) else { return PARAMETER };
    if !context.is_null() {
        // SAFETY: the title passes its context.
        let handle = unsafe { *context } as usize;
        contexts(|c| c.remove(&handle));
    }
    // SAFETY: the title's own function, with its arguments.
    unsafe { std::mem::transmute::<*const (), Destroy>(destroy)(context, memory) }
}

extern "C" fn ffxConfigure(context: *mut *mut c_void, desc: *mut Header) -> u32 {
    let Some(configure) = original(2) else { return PARAMETER };
    // SAFETY: the title's own function, with its arguments.
    unsafe { std::mem::transmute::<*const (), Call>(configure)(context, desc) }
}

extern "C" fn ffxQuery(context: *mut *mut c_void, desc: *mut Header) -> u32 {
    let Some(query) = original(3) else { return PARAMETER };
    // SAFETY: the title's own function, with its arguments.
    unsafe { std::mem::transmute::<*const (), Call>(query)(context, desc) }
}

extern "C" fn ffxDispatch(context: *mut *mut c_void, desc: *mut Header) -> u32 {
    let Some(dispatch) = original(4) else { return PARAMETER };
    // SAFETY: the title's own function, with its arguments.
    let own = || unsafe { std::mem::transmute::<*const (), Call>(dispatch)(context, desc) };
    if context.is_null() || desc.is_null() {
        return own();
    }
    // SAFETY: the title passes a description and its context.
    let (kind, handle) = unsafe { ((*desc).kind, *context as usize) };
    if kind != DISPATCH_UPSCALE {
        return own();
    }
    let Some(made) = contexts(|c| c.get(&handle).copied()) else {
        once!(warn, "FidelityFX: an upscale dispatch on a context not seen made; the title's own runs");
        return own();
    };
    // SAFETY: an upscaling, by its type.
    let d = unsafe { desc.cast::<DispatchUpscale>().read() };
    once!(
        info,
        render = ?d.render_size,
        output = ?d.upscale_size,
        flags = format_args!("{:#x}", made.flags),
        jitter = ?d.jitter,
        motion_scale = ?d.motion_scale,
        near = d.camera_near,
        far = d.camera_far,
        fov = d.camera_fov_angle_vertical,
        pre_exposure = d.pre_exposure,
        resources = ?d.resources.iter().map(|r| (!r.resource.is_null(), r.description[1], r.description[2], r.description[3], r.state)).collect::<Vec<_>>(),
        "FidelityFX: the title's first upscale dispatch (each texture: there, format, width, height, state)"
    );
    match dlss::upscale(&upscale(&d, &made, handle)) {
        Ok(()) => OK,
        Err(why) => {
            refused(why);
            dlss::refused();
            if made.fitted {
                // Its own context is made for 1080p: run at another size it
                // would read past its tables. No upscaling this frame (the
                // output keeps the last one) rather than that.
                once!(warn, "FidelityFX: DLSS did not take an upscaling on a context made at the shipping sizes; none ran");
                OK
            } else {
                own()
            }
        }
    }
}

/// Each reason the title's own upscaling ran, once.
fn refused(why: &'static str) {
    static SEEN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.contains(&why) {
        seen.push(why);
        tracing::warn!("FidelityFX: DLSS did not take the upscaling: {why}");
    }
}

/// An upscaling in `victory_gpu::dlss`'s terms; with no output size named,
/// the context's largest (as FidelityFX reads it).
fn upscale(d: &DispatchUpscale, made: &Made, context: usize) -> dlss::Upscale {
    let output = if d.upscale_size == [0, 0] { made.max_output } else { d.upscale_size };
    dlss::Upscale {
        context,
        list: d.list.cast::<Object>().cast_const(),
        resources: d.resources.map(|r| (r.resource.cast::<Object>().cast_const(), r.state)),
        camera: dlss::Camera {
            jitter: d.jitter,
            motion_scale: d.motion_scale,
            render: d.render_size,
            output,
            near: d.camera_near,
            far: d.camera_far,
            fov_y: d.camera_fov_angle_vertical,
            reset: d.reset != 0,
            flags: made.flags,
        },
        pre_exposure: d.pre_exposure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    #[test]
    fn descriptions_lay_out_as_the_sdk() {
        assert_eq!(offset_of!(CreateUpscale, flags), 0x10);
        assert_eq!(offset_of!(CreateUpscale, max_output), 0x1c);
        assert_eq!(offset_of!(Resource, state), 0x28);
        assert_eq!(offset_of!(DispatchUpscale, list), 0x10);
        assert_eq!(offset_of!(DispatchUpscale, resources), 0x18);
        assert_eq!(offset_of!(DispatchUpscale, jitter), 0x168);
        assert_eq!(offset_of!(DispatchUpscale, render_size), 0x178);
        assert_eq!(offset_of!(DispatchUpscale, upscale_size), 0x180);
        assert_eq!(offset_of!(DispatchUpscale, frame_time_delta), 0x190);
        assert_eq!(offset_of!(DispatchUpscale, reset), 0x198);
        assert_eq!(offset_of!(DispatchUpscale, camera_near), 0x19c);
        assert_eq!(offset_of!(DispatchUpscale, flags), 0x1ac);
    }

    #[test]
    fn the_shipping_sizes_are_kept_and_others_fitted() {
        assert_eq!(shipping([1280, 720], [1920, 1080]), None);
        assert_eq!(shipping([1920, 1080], [1920, 1080]), None);
        assert_eq!(shipping([1706, 960], [2560, 1440]), Some(([1280, 720], [1920, 1080])));
        assert_eq!(shipping([960, 540], [1920, 1080]), Some(([1280, 720], [1920, 1080])));
    }

    #[test]
    fn a_dispatch_reads_as_its_upscaling() {
        let r = |at: usize, state: u32| Resource { resource: at as *mut c_void, description: [1, 10, 1280, 720, 1, 1, 0, 0], state };
        let d = DispatchUpscale {
            header: Header { kind: DISPATCH_UPSCALE, next: std::ptr::null_mut() },
            list: 0x1000 as *mut c_void,
            resources: [r(0x10, 0xc), r(0x20, 0xc), r(0x30, 0xc), r(0, 0), r(0x50, 0xc), r(0, 0), r(0x70, 0x2)],
            jitter: [0.25, -0.125],
            motion_scale: [1280.0, 720.0],
            render_size: [1280, 720],
            upscale_size: [0, 0],
            enable_sharpening: 0,
            sharpness: 0.0,
            frame_time_delta: 33.3,
            pre_exposure: 1.5,
            reset: 1,
            camera_near: 10000.0,
            camera_far: 0.15,
            camera_fov_angle_vertical: 0.9,
            view_space_to_meters_factor: 1.0,
            flags: 0,
        };
        let up = upscale(&d, &Made { flags: 0x29, max_output: [1920, 1080], fitted: false }, 0xabc);
        assert_eq!(up.context, 0xabc);
        assert_eq!(up.list as usize, 0x1000);
        assert_eq!(up.resources[dlss::COLOUR].0 as usize, 0x10);
        assert!(up.resources[dlss::EXPOSURE].0.is_null());
        assert_eq!(up.resources[dlss::OUTPUT], (0x70 as *const Object, 0x2));
        assert_eq!(up.camera.output, [1920, 1080], "no output size named: the context's largest");
        assert!(up.camera.reset);
        assert_eq!(up.camera.flags, 0x29);
    }
}
