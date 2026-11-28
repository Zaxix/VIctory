//! Copies of title memory made in place, by this process, with a way back
//! when a page is not there (`VICTORY_COPY_PLAIN=1`; off by default).
//!
//! Every copy of title memory into a host buffer goes through the kernel
//! (`ReadProcessMemory` on our own process, `crate::host::copy_mapped`):
//! the title unmaps and remaps its memory while we read it, a read in
//! place of a page that is gone is an access violation, and the kernel's
//! copy fails instead. It costs what a system call that locks and probes
//! every page costs: run 225's profile has the submitting thread 35 ms a
//! frame inside `ZwReadVirtualMemory` for about 100 MB (3 GB a second),
//! where the processor's own copy moves that much in a third of the time.
//!
//! Here the copy is one `rep movsb` in a function of ours that uses no
//! stack. An exception handler, first in the line, looks at every access
//! violation: one that is a READ and happened at exactly that instruction
//! continues at the function's other exit, which returns 0, and the
//! caller falls back to the kernel's copy as before (it says which page
//! is gone and fills in zeros). A write that faults is not ours to take:
//! the destination is host memory, which is always there; such a fault
//! goes on to the handlers after this one.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl victory_copy_guarded",
    ".globl victory_copy_guarded_at",
    ".globl victory_copy_guarded_fault",
    "victory_copy_guarded:",
    // rcx: to, rdx: from, r8: bytes. rsi and rdi are the caller's: kept in
    // r9 and r10, which are ours to use, so nothing is on the stack.
    "    mov r9, rsi",
    "    mov r10, rdi",
    "    mov rdi, rcx",
    "    mov rsi, rdx",
    "    mov rcx, r8",
    "victory_copy_guarded_at:",
    "    rep movsb",
    "    mov rsi, r9",
    "    mov rdi, r10",
    "    mov eax, 1",
    "    ret",
    "victory_copy_guarded_fault:",
    "    mov rsi, r9",
    "    mov rdi, r10",
    "    xor eax, eax",
    "    ret",
    // The same copy for stores INTO title memory ([`store`]): its own
    // instruction, so the handler can tell the two apart.
    ".globl victory_store_guarded",
    ".globl victory_store_guarded_at",
    ".globl victory_store_guarded_fault",
    "victory_store_guarded:",
    "    mov r9, rsi",
    "    mov r10, rdi",
    "    mov rdi, rcx",
    "    mov rsi, rdx",
    "    mov rcx, r8",
    "victory_store_guarded_at:",
    "    rep movsb",
    "    mov rsi, r9",
    "    mov rdi, r10",
    "    mov eax, 1",
    "    ret",
    "victory_store_guarded_fault:",
    "    mov rsi, r9",
    "    mov rdi, r10",
    "    xor eax, eax",
    "    ret",
    // The same copy from title memory INTO title memory ([`move_title`]):
    // either side may fault, and any fault there gives up.
    ".globl victory_move_guarded",
    ".globl victory_move_guarded_at",
    ".globl victory_move_guarded_fault",
    "victory_move_guarded:",
    "    mov r9, rsi",
    "    mov r10, rdi",
    "    mov rdi, rcx",
    "    mov rsi, rdx",
    "    mov rcx, r8",
    "victory_move_guarded_at:",
    "    rep movsb",
    "    mov rsi, r9",
    "    mov rdi, r10",
    "    mov eax, 1",
    "    ret",
    "victory_move_guarded_fault:",
    "    mov rsi, r9",
    "    mov rdi, r10",
    "    xor eax, eax",
    "    ret",
);

#[cfg(target_arch = "x86_64")]
unsafe extern "C" {
    fn victory_copy_guarded(to: *mut u8, from: *const u8, len: usize) -> u32;
    static victory_copy_guarded_at: u8;
    static victory_copy_guarded_fault: u8;
    fn victory_store_guarded(to: *mut u8, from: *const u8, len: usize) -> u32;
    static victory_store_guarded_at: u8;
    static victory_store_guarded_fault: u8;
    fn victory_move_guarded(to: *mut u8, from: *const u8, len: usize) -> u32;
    static victory_move_guarded_at: u8;
    static victory_move_guarded_fault: u8;
}

/// Whether copies of title memory into host memory are made in place
/// (`VICTORY_COPY_PLAIN=1`).
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = cfg!(target_arch = "x86_64") && crate::fast::on("VICTORY_COPY_PLAIN");
        if on {
            install();
            tracing::info!(
                "copies of title memory into host buffers are made in place, with the kernel's copy behind them for a \
                 page that is not there (VICTORY_COPY_PLAIN=1)"
            );
        }
        on
    })
}

/// Copies made in place, their bytes, the copies that met a page that was
/// not there (and went to the kernel's copy), and the copies not tried in
/// place because their source faulted lately ([`RECENT`]) or they were
/// under [`LEAST`] bytes, since the last line; and the fills of under
/// [`LEAST`] bytes made in place ([`copy_short`]).
static COUNT: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

/// The fewest bytes copied in place. A read of a byte or a word is how the
/// layer ASKS whether a page is there (a texture's first page, a record):
/// it is expected to fail, and a failed kernel read costs a microsecond
/// where a fault and its handler cost tens (run 230: 1,000 to 1,300 faults
/// a frame, nearly all of them such askings, took back most of what the
/// copies in place had won).
const LEAST: usize = 256;

/// The 64 KiB blocks of title memory a copy in place faulted in lately:
/// their copies go straight to the kernel's, which reads round the hole.
/// By the block's number, in the place its low bits name (a later block
/// with the same low bits takes the place). Forgotten every two seconds:
/// the title maps the memory in the end.
static RECENT: [std::sync::atomic::AtomicUsize; 1024] = [const { std::sync::atomic::AtomicUsize::new(0) }; 1024];
/// When [`RECENT`] was last emptied (milliseconds of [`now_ms`]).
static EMPTIED: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// The numbers of the first and last 64 KiB block of `[from, from + len)`,
/// never 0 (0 is an empty place).
fn blocks(from: usize, len: usize) -> (usize, usize) {
    ((from >> 16) | 1 << 63, ((from + len.max(1) - 1) >> 16) | 1 << 63)
}

/// Whether a copy from there faulted lately.
fn faulted_lately(from: usize, len: usize) -> bool {
    let now = now_ms();
    let emptied = EMPTIED.load(Relaxed);
    if now.saturating_sub(emptied) >= 2_000 && EMPTIED.compare_exchange(emptied, now, Relaxed, Relaxed).is_ok() {
        for place in &RECENT {
            place.store(0, Relaxed);
        }
        return false;
    }
    let (first, last) = blocks(from, len);
    RECENT[first & 1023].load(Relaxed) == first || RECENT[last & 1023].load(Relaxed) == last
}

/// A copy from there faulted.
fn note_fault(from: usize, len: usize) {
    let (first, last) = blocks(from, len);
    RECENT[first & 1023].store(first, Relaxed);
    RECENT[last & 1023].store(last, Relaxed);
}

/// The handler that turns a read fault inside [`guarded`] into its return
/// of false; put first in the line, once.
#[cfg(target_arch = "x86_64")]
fn install() {
    use windows_sys::Win32::Foundation::{
        EXCEPTION_ACCESS_VIOLATION, EXCEPTION_GUARD_PAGE, EXCEPTION_IN_PAGE_ERROR,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
    };
    unsafe extern "system" fn on_fault(info: *mut EXCEPTION_POINTERS) -> i32 {
        // SAFETY: the system passes the exception being dispatched and the
        // context it will continue with.
        let (r, context) = unsafe { (&*(*info).ExceptionRecord, &mut *(*info).ContextRecord) };
        // A store into title memory ([`store`]): its source is host
        // memory, always there, so any fault is the destination's (a page
        // not committed, or read-only): the store gives up and the caller
        // goes to the kernel's write, which leaves such a page alone.
        if context.Rip == (&raw const victory_store_guarded_at) as u64 {
            if r.ExceptionCode == EXCEPTION_ACCESS_VIOLATION
                || r.ExceptionCode == EXCEPTION_IN_PAGE_ERROR
                || r.ExceptionCode == EXCEPTION_GUARD_PAGE
            {
                context.Rip = (&raw const victory_store_guarded_fault) as u64;
                return EXCEPTION_CONTINUE_EXECUTION;
            }
            return EXCEPTION_CONTINUE_SEARCH;
        }
        // A title copy made in place ([`move_title`]): a fault on either
        // side gives up, and the caller goes to the kernel's copy.
        if context.Rip == (&raw const victory_move_guarded_at) as u64 {
            if r.ExceptionCode == EXCEPTION_ACCESS_VIOLATION
                || r.ExceptionCode == EXCEPTION_IN_PAGE_ERROR
                || r.ExceptionCode == EXCEPTION_GUARD_PAGE
            {
                context.Rip = (&raw const victory_move_guarded_fault) as u64;
                return EXCEPTION_CONTINUE_EXECUTION;
            }
            return EXCEPTION_CONTINUE_SEARCH;
        }
        // Only at the copy's own instruction.
        if context.Rip != (&raw const victory_copy_guarded_at) as u64 {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        const READ: usize = 0;
        let read = r.NumberParameters >= 1 && r.ExceptionInformation[0] == READ;
        let gone = (r.ExceptionCode == EXCEPTION_ACCESS_VIOLATION || r.ExceptionCode == EXCEPTION_IN_PAGE_ERROR) && read;
        if gone || r.ExceptionCode == EXCEPTION_GUARD_PAGE {
            context.Rip = (&raw const victory_copy_guarded_fault) as u64;
            return EXCEPTION_CONTINUE_EXECUTION;
        }
        EXCEPTION_CONTINUE_SEARCH
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: a handler that reads the record it is given and nothing else.
    ONCE.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, Some(on_fault));
    });
}

#[cfg(not(target_arch = "x86_64"))]
fn install() {}

/// Copies `len` bytes from `from` to `to` in place; false if a page of the
/// source was not there (then some of `to` may have been written).
///
/// `to` must be writable memory of `len` bytes that is ours (host memory);
/// `from` may be anything.
#[cfg(target_arch = "x86_64")]
fn guarded(to: *mut u8, from: *const u8, len: usize) -> bool {
    install();
    // SAFETY: the destination is the caller's to write; a source page that
    // is not there ends the copy with 0 instead of a fault ([`install`]).
    unsafe { victory_copy_guarded(to, from, len) != 0 }
}

#[cfg(not(target_arch = "x86_64"))]
fn guarded(_: *mut u8, _: *const u8, _: usize) -> bool {
    false
}

/// With [`on`]: copies title memory at `from` into host memory at `to`;
/// false if it is off or a source page was not there (the caller then
/// copies through the kernel, as before).
pub(crate) fn copy(to: *mut u8, from: *const u8, len: usize) -> bool {
    if !on() {
        return false;
    }
    if len < LEAST {
        COUNT[4].fetch_add(1, Relaxed);
        return false;
    }
    if faulted_lately(from as usize, len) {
        COUNT[3].fetch_add(1, Relaxed);
        return false;
    }
    let whole = guarded(to, from, len);
    if whole {
        COUNT[0].fetch_add(1, Relaxed);
        COUNT[1].fetch_add(len as u64, Relaxed);
    } else {
        COUNT[2].fetch_add(1, Relaxed);
        note_fault(from as usize, len);
    }
    whole
}

/// With [`on`]: copies `len` bytes, fewer than [`LEAST`], of title memory
/// at `from` into host memory at `to`, for a caller that is NOT asking
/// whether the page is there but expects it to be (a list's fill of upload
/// memory: constants the title recorded a draw with). False if it is off,
/// if the source faulted lately, or if a source page was not there; the
/// caller then copies through the kernel, as before.
///
/// Why: the floor of [`copy`] is there for the askings, which fail. The
/// fills do not: 8,000 to 12,000 of them a frame went to the kernel's copy
/// (run 279's lines, "of under 256 bytes"), about 400 ns each where the
/// copy in place of 16 to 128 bytes is about 30 ns (timed on this
/// processor, 2026-11-25, two million copies each).
pub(crate) fn copy_short(to: *mut u8, from: *const u8, len: usize) -> bool {
    if !on() {
        return false;
    }
    if faulted_lately(from as usize, len) {
        COUNT[3].fetch_add(1, Relaxed);
        return false;
    }
    let whole = guarded(to, from, len);
    if whole {
        COUNT[5].fetch_add(1, Relaxed);
    } else {
        COUNT[2].fetch_add(1, Relaxed);
        note_fault(from as usize, len);
    }
    whole
}

/// Whether what shaders wrote is stored into title memory in place
/// (`VICTORY_WRITE_PLAIN`; on with the faster defaults, `=0` off, `alt`
/// twenty seconds each way in turn), with the kernel's write behind it for
/// a page that is not there or not writable.
///
/// Run 14 on the owner's PC: the card copies' merges spent 139 ms a frame
/// (all threads) writing back, and 60 % of their stretches went the old
/// way: one `NtWriteVirtualMemory` for each run of words the card changed,
/// a system call that locks and probes the page every time, where the
/// processor's own store of those words takes nanoseconds.
pub(crate) fn store_on() -> bool {
    /// 0 off, 1 on, 2 in turn.
    static MODE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    let mode = *MODE.get_or_init(|| {
        let mode = match std::env::var("VICTORY_WRITE_PLAIN").as_deref() {
            Ok("alt") => 2,
            Ok("0") | Ok("") => 0,
            Ok(_) => 1,
            Err(_) => u8::from(crate::fast::fast()),
        };
        if cfg!(not(target_arch = "x86_64")) {
            return 0;
        }
        if mode != 0 {
            install();
        }
        tracing::info!(
            "what shaders wrote is stored into title memory {} (VICTORY_WRITE_PLAIN: 0 through the kernel, alt in turn)",
            ["through the kernel's write, as before", "in place, the kernel's write behind it for a page that is not there", "in place and through the kernel in turn, twenty seconds each"][usize::from(mode)]
        );
        mode
    });
    match mode {
        0 => false,
        1 => true,
        _ => {
            static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
            let on = (EPOCH.get_or_init(std::time::Instant::now).elapsed().as_secs() / 20).is_multiple_of(2);
            static LAST: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);
            if LAST.swap(u8::from(on), Relaxed) != u8::from(on) {
                tracing::info!(
                    "what shaders wrote is stored into title memory from now (VICTORY_WRITE_PLAIN=alt): {}",
                    if on { "IN PLACE" } else { "through the kernel" }
                );
            }
            on
        }
    }
}

/// Since the last line: stores made in place, their bytes, and those that
/// met a page that was not there or not writable (then written by the
/// kernel); stores not tried because their page faulted lately.
static STORES: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

/// The 64 KiB blocks a store in place faulted in lately, as [`RECENT`]
/// for reads (a page the title has unmapped stays so a while).
static STORE_RECENT: [std::sync::atomic::AtomicUsize; 1024] = [const { std::sync::atomic::AtomicUsize::new(0) }; 1024];
static STORE_EMPTIED: AtomicU64 = AtomicU64::new(0);

/// With [`store_on`]: stores `len` bytes of host memory at `from` into
/// title memory at `to`, in place; false if it is off, or a page of `to`
/// was not there or not writable (some of it may have been written: the
/// caller writes it all again through the kernel, as before).
///
/// `from` must be `len` readable bytes of host memory. The caller has
/// opened the write-watch guards of `to` (`crate::watch::writing`); a
/// guard still closed faults here and the kernel's write takes it.
pub(crate) fn store(to: *mut u8, from: *const u8, len: usize) -> bool {
    if len == 0 || !store_on() {
        return false;
    }
    let now = now_ms();
    let emptied = STORE_EMPTIED.load(Relaxed);
    if now.saturating_sub(emptied) >= 2_000 && STORE_EMPTIED.compare_exchange(emptied, now, Relaxed, Relaxed).is_ok() {
        for place in &STORE_RECENT {
            place.store(0, Relaxed);
        }
    }
    let (first, last) = blocks(to as usize, len);
    if STORE_RECENT[first & 1023].load(Relaxed) == first || STORE_RECENT[last & 1023].load(Relaxed) == last {
        STORES[3].fetch_add(1, Relaxed);
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    // SAFETY: the source is the caller's host memory; a destination page
    // that is not there or not writable ends the store with 0 instead of a
    // fault ([`install`]).
    let whole = unsafe { victory_store_guarded(to, from, len) != 0 };
    #[cfg(not(target_arch = "x86_64"))]
    let whole = false;
    if whole {
        STORES[0].fetch_add(1, Relaxed);
        STORES[1].fetch_add(len as u64, Relaxed);
    } else {
        STORES[2].fetch_add(1, Relaxed);
        STORE_RECENT[first & 1023].store(first, Relaxed);
        STORE_RECENT[last & 1023].store(last, Relaxed);
    }
    whole
}

/// [`store`] for the runs `(offset, bytes)` of `from` going to the same
/// offsets from `to` (a merge's runs of words, all in one 64 KiB page):
/// the switch and the recent faults asked once, counted once. False if it
/// is off or a page faulted (then some runs may have been written: the
/// caller writes them all again through the kernel).
pub(crate) fn store_runs(to: usize, from: &[u8], runs: &[(usize, usize)]) -> bool {
    let (Some(&(first, _)), Some(&(last, last_len))) = (runs.first(), runs.last()) else {
        return true;
    };
    if !store_on() {
        return false;
    }
    let (lo, hi) = blocks(to + first, last + last_len - first);
    if STORE_RECENT[lo & 1023].load(Relaxed) == lo || STORE_RECENT[hi & 1023].load(Relaxed) == hi {
        STORES[3].fetch_add(runs.len() as u64, Relaxed);
        return false;
    }
    let mut bytes = 0u64;
    for &(off, len) in runs {
        debug_assert!(off + len <= from.len());
        #[cfg(target_arch = "x86_64")]
        // SAFETY: the source is the caller's host memory (inside `from`);
        // a destination page that is not there or not writable ends the
        // store with 0 instead of a fault ([`install`]).
        let whole = unsafe { victory_store_guarded((to + off) as *mut u8, from.as_ptr().add(off), len) != 0 };
        #[cfg(not(target_arch = "x86_64"))]
        let whole = false;
        if !whole {
            STORES[2].fetch_add(1, Relaxed);
            STORE_RECENT[lo & 1023].store(lo, Relaxed);
            STORE_RECENT[hi & 1023].store(hi, Relaxed);
            return false;
        }
        bytes += len as u64;
    }
    STORES[0].fetch_add(runs.len() as u64, Relaxed);
    STORES[1].fetch_add(bytes, Relaxed);
    true
}

/// With the frame line: what [`store`] did per frame over `frames`.
pub(crate) fn store_line(frames: f64) {
    let [stores, bytes, gone, lately] = [0, 1, 2, 3].map(|i| STORES[i].swap(0, Relaxed) as f64 / frames);
    if stores + gone + lately == 0.0 {
        return;
    }
    tracing::info!(
        "what shaders wrote, stored into title memory in place per frame (VICTORY_WRITE_PLAIN): {stores:.0}, {:.1} MB; \
         {gone:.1} met a page that was not there or not writable and went through the kernel's write; {lately:.0} not \
         tried, their page having faulted in the last two seconds",
        bytes / 1e6
    );
}

/// The fewest bytes [`copy`] takes: what is shorter is [`copy_short`]'s,
/// for callers that may use it.
pub(crate) const fn least() -> usize {
    LEAST
}

/// With the frame line: what [`copy`] did per frame over `frames`.
/// Whether the title's own copies of its memory (a list's buffer copy,
/// run at submission) are made in place, on several threads when large
/// (`VICTORY_TITLE_COPY_PLAIN`; on with the faster defaults, `=0` off,
/// `alt` twenty seconds each way in turn).
///
/// Run 19: the 27 title copies of a heavy frame took 24 ms of the
/// submitting thread, which the card waited for. They went through the
/// kernel's copy (`ReadProcessMemory` on our own process, about 3 GB a
/// second), one thread, while the copy in place moves several times that,
/// and a large one is split over the helper threads. A page that is not
/// there or not writable on either side ends the copy in place, and the
/// kernel's copy does it all again, as before.
pub(crate) fn title_moves_on() -> bool {
    /// 0 off, 1 on, 2 in turn.
    static MODE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    let mode = *MODE.get_or_init(|| {
        let mode = match std::env::var("VICTORY_TITLE_COPY_PLAIN").as_deref() {
            Ok("alt") => 2,
            Ok("0") | Ok("") => 0,
            Ok(_) => 1,
            Err(_) => u8::from(crate::fast::fast()),
        };
        if cfg!(not(target_arch = "x86_64")) {
            return 0;
        }
        if mode != 0 {
            install();
        }
        tracing::info!(
            "the title's copies of its memory are made {} (VICTORY_TITLE_COPY_PLAIN: 0 through the kernel, alt in turn)",
            [
                "through the kernel's copy, as before",
                "in place, large ones on several threads, the kernel's copy behind them",
                "in place and through the kernel in turn, twenty seconds each"
            ][usize::from(mode)]
        );
        mode
    });
    match mode {
        0 => false,
        1 => true,
        _ => {
            static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
            let on = (EPOCH.get_or_init(std::time::Instant::now).elapsed().as_secs() / 20).is_multiple_of(2);
            static LAST: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);
            if LAST.swap(u8::from(on), Relaxed) != u8::from(on) {
                tracing::info!(
                    "the title's copies of its memory are made from now (VICTORY_TITLE_COPY_PLAIN=alt): {}",
                    if on { "IN PLACE" } else { "through the kernel" }
                );
            }
            on
        }
    }
}

/// Since the last line: title copies made in place, their bytes, those
/// split over threads, those that met a page that was not there (then
/// copied by the kernel), and those not tried (overlapping, or a fault
/// there lately).
static MOVES: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

/// From this many bytes a title copy is split over the helper threads.
const MOVE_WIDE: usize = 2 << 20;
/// About this many bytes to each piece of a split copy.
const MOVE_PIECE: usize = 1 << 20;

/// One guarded copy, title memory to title memory.
#[cfg(target_arch = "x86_64")]
fn move_guarded(to: *mut u8, from: *const u8, len: usize) -> bool {
    // SAFETY: a page that is not there, or not writable, on either side
    // ends the copy with 0 instead of a fault ([`install`]); the caller
    // has opened the write-watch guards of `to`.
    unsafe { victory_move_guarded(to, from, len) != 0 }
}

#[cfg(not(target_arch = "x86_64"))]
fn move_guarded(_: *mut u8, _: *const u8, _: usize) -> bool {
    false
}

/// With [`title_moves_on`]: copies `len` bytes of title memory at `from`
/// to title memory at `to` in place, a large copy in pieces on the helper
/// threads; false if it is off, the two overlap, either side faulted
/// lately, or a page was not there or not writable (then some of `to` may
/// have been written: the caller copies it all again through the kernel).
///
/// The caller has opened the write-watch guards of `to`
/// (`crate::watch::writing`) and tells the journal of the write after.
pub(crate) fn move_title(to: *mut u8, from: *const u8, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    if !title_moves_on() {
        return false;
    }
    let (t, f) = (to as usize, from as usize);
    if t < f.saturating_add(len) && f < t.saturating_add(len) {
        MOVES[4].fetch_add(1, Relaxed);
        return false;
    }
    if faulted_lately(f, len) || faulted_lately(t, len) {
        MOVES[4].fetch_add(1, Relaxed);
        return false;
    }
    let whole = if len >= MOVE_WIDE {
        MOVES[2].fetch_add(1, Relaxed);
        let failed = std::sync::atomic::AtomicBool::new(false);
        let failed_ref = &failed;
        let jobs: Vec<Box<dyn FnOnce() + Send + '_>> = (0..len)
            .step_by(MOVE_PIECE)
            .map(|at| {
                let n = MOVE_PIECE.min(len - at);
                let (to, from) = (t + at, f + at);
                Box::new(move || {
                    if !failed_ref.load(Relaxed) && !move_guarded(to as *mut u8, from as *const u8, n) {
                        failed_ref.store(true, Relaxed);
                    }
                }) as Box<dyn FnOnce() + Send + '_>
            })
            .collect();
        crate::shadow::wide_scoped(crate::shadow::ahead_workers(), jobs);
        !failed.into_inner()
    } else {
        move_guarded(to, from, len)
    };
    if whole {
        MOVES[0].fetch_add(1, Relaxed);
        MOVES[1].fetch_add(len as u64, Relaxed);
    } else {
        MOVES[3].fetch_add(1, Relaxed);
        note_fault(f, len);
        note_fault(t, len);
    }
    whole
}

/// The line of [`move_title`]'s counts.
pub(crate) fn moves_line(frames: f64) {
    if !title_moves_on() {
        return;
    }
    let [moves, bytes, split, gone, untried] = [0, 1, 2, 3, 4].map(|i| MOVES[i].swap(0, Relaxed) as f64 / frames);
    tracing::info!(
        "the title's copies of its memory made in place per frame (VICTORY_TITLE_COPY_PLAIN): {moves:.1}, {:.1} MB, \
         {split:.1} of them split over the helper threads; {gone:.1} met a page that was not there or not writable and \
         went through the kernel's copy; {untried:.1} not tried (overlapping, or a fault there lately)",
        bytes / 1e6
    );
}

pub(crate) fn line(frames: f64) {
    if !on() {
        return;
    }
    let [copies, bytes, gone, lately, small, fills] = [0, 1, 2, 3, 4, 5].map(|i| COUNT[i].swap(0, Relaxed) as f64 / frames);
    tracing::info!(
        "copies of title memory made in place per frame (VICTORY_COPY_PLAIN=1): {copies:.0}, {:.1} MB; {gone:.1} met a page \
         that was not there and went through the kernel's copy; left to the kernel's copy untried: {lately:.0} whose source \
         faulted in the last two seconds, {small:.0} of under {LEAST} bytes; fills of under {LEAST} bytes made in place \
         (VICTORY_FILLS_PLAIN): {fills:.0}",
        bytes / 1e6
    );
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_READWRITE, VirtualAlloc, VirtualFree,
    };

    const PAGE: usize = 4096;

    #[test]
    fn memory_that_is_there_is_copied() {
        let from: Vec<u8> = (0..100_000u32).map(|i| (i * 7 + 3) as u8).collect();
        let mut to = vec![0u8; from.len()];
        assert!(guarded(to.as_mut_ptr(), from.as_ptr(), from.len()));
        assert_eq!(to, from);
        // Nothing to copy is a copy.
        assert!(guarded(to.as_mut_ptr(), from.as_ptr(), 0));
    }

    /// A fill's few bytes: copied whole where they are there, up to the last
    /// byte of a page; a copy that runs into a hole by one byte ends with
    /// false and the thread goes on.
    #[test]
    fn a_short_copy_is_whole_or_ends_without_a_fault() {
        // SAFETY: memory of the test's own.
        let base = unsafe { VirtualAlloc(std::ptr::null(), 2 * PAGE, MEM_RESERVE, PAGE_NOACCESS) } as *mut u8;
        assert!(!base.is_null());
        // SAFETY: the first page of that reservation.
        assert!(!unsafe { VirtualAlloc(base.cast(), PAGE, MEM_COMMIT, PAGE_READWRITE) }.is_null());
        // SAFETY: committed and writable.
        let page = unsafe { std::slice::from_raw_parts_mut(base, PAGE) };
        for (i, b) in page.iter_mut().enumerate() {
            *b = (i * 13 + 5) as u8;
        }
        for len in [1usize, 16, 64, 255] {
            let mut to = vec![0u8; len];
            // The page's last bytes.
            assert!(guarded(to.as_mut_ptr(), base.wrapping_add(PAGE - len), len), "{len}");
            assert_eq!(to, page[PAGE - len..], "{len}");
            // One byte into the hole.
            let mut to = vec![0u8; len];
            assert!(!guarded(to.as_mut_ptr(), base.wrapping_add(PAGE - len + 1), len), "{len}");
            assert_eq!(to[..len - 1], page[PAGE - len + 1..], "{len}: the bytes before the hole");
        }
        // SAFETY: the test's reservation.
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    /// Three pages reserved, the first committed: a copy that stays in the
    /// first page is whole; one that runs into the second ends with false,
    /// the bytes before the hole copied, and the thread goes on.
    #[test]
    fn a_page_that_is_not_there_ends_the_copy_without_a_fault() {
        // SAFETY: memory of the test's own.
        let base = unsafe { VirtualAlloc(std::ptr::null(), 3 * PAGE, MEM_RESERVE, PAGE_NOACCESS) } as *mut u8;
        assert!(!base.is_null());
        // SAFETY: the first page of that reservation.
        assert!(!unsafe { VirtualAlloc(base.cast(), PAGE, MEM_COMMIT, PAGE_READWRITE) }.is_null());
        // SAFETY: committed and writable.
        unsafe { std::slice::from_raw_parts_mut(base, PAGE) }.fill(0xAB);
        let mut to = vec![0u8; 2 * PAGE];
        assert!(guarded(to.as_mut_ptr(), base, PAGE));
        assert!(to[..PAGE].iter().all(|&b| b == 0xAB));
        to.fill(0);
        assert!(!guarded(to.as_mut_ptr(), base.wrapping_add(PAGE / 2), PAGE));
        assert!(to[..PAGE / 2].iter().all(|&b| b == 0xAB), "the bytes before the hole");
        assert!(to[PAGE / 2..].iter().all(|&b| b == 0), "nothing after it");
        // A source that is not there at all.
        assert!(!guarded(to.as_mut_ptr(), base.wrapping_add(PAGE), PAGE));
        // Committed but closed to reads.
        // SAFETY: the third page of the reservation.
        assert!(!unsafe { VirtualAlloc(base.wrapping_add(2 * PAGE).cast(), PAGE, MEM_COMMIT, PAGE_NOACCESS) }.is_null());
        assert!(!guarded(to.as_mut_ptr(), base.wrapping_add(2 * PAGE), 16));
        // And the registers the caller owns came back: this still works.
        assert!(guarded(to.as_mut_ptr(), base, PAGE));
        assert!(to[..PAGE].iter().all(|&b| b == 0xAB));
        // SAFETY: the test's reservation.
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    /// A source that faulted is remembered by its 64 KiB blocks, and only
    /// those.
    #[test]
    fn a_source_that_faulted_is_remembered_by_its_blocks() {
        // (Addresses no other test uses; the table is shared.)
        let at = 0x7654_3210_0000usize;
        assert!(!faulted_lately(at, 4096));
        note_fault(at + 100, 4096);
        assert!(faulted_lately(at, 16));
        assert!(faulted_lately(at + 0xffff, 1));
        // A copy that ends in the block, and one that begins in it.
        assert!(faulted_lately(at - 4096, 8192));
        // The blocks beside it are not remembered.
        assert!(!faulted_lately(at + 0x10000, 4096));
        assert!(!faulted_lately(at - 0x10000, 4096));
        assert_eq!(blocks(0, 0), (1 << 63, 1 << 63));
    }

    /// Many threads at once, half of them faulting: each gets its own answer.
    /// A small fixed-seed generator for the store tests.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Memory reserved and committed read-write, `pages` of 4 KiB. The
    /// blocks stores faulted in are forgotten first: a test's memory may
    /// sit where an earlier test's faulted.
    fn committed(pages: usize) -> *mut u8 {
        for place in &STORE_RECENT {
            place.store(0, Relaxed);
        }
        // SAFETY: memory of the test's own.
        let base = unsafe { VirtualAlloc(std::ptr::null(), pages * PAGE, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
        assert!(!base.is_null());
        base.cast()
    }

    /// Runs of words over `len` bytes, sorted and apart, as a merge makes
    /// them: `count` at most, each 4 to 64 bytes, from `seed`.
    fn runs(len: usize, count: usize, seed: u64) -> Vec<(usize, usize)> {
        let mut rng = Rng(seed);
        let mut starts: Vec<usize> = (0..count).map(|_| rng.below(len / 4) * 4).collect();
        starts.sort_unstable();
        starts.dedup();
        let mut out: Vec<(usize, usize)> = Vec::new();
        for s in starts {
            if out.last().is_some_and(|&(a, n)| s < a + n + 4) {
                continue;
            }
            let n = (4 * (1 + rng.below(16))).min(len - s);
            out.push((s, n));
        }
        out
    }

    /// The bytes `runs` of `from` put over `before`, as the kernel's write of
    /// each run would.
    fn reference(before: &[u8], from: &[u8], runs: &[(usize, usize)]) -> Vec<u8> {
        let mut out = before.to_vec();
        for &(off, n) in runs {
            out[off..off + n].copy_from_slice(&from[off..off + n]);
        }
        out
    }

    /// Runs over 64 KiB pages and 4 KiB boundaries, from many to one: what
    /// lands is what the runs say, byte for byte, and nothing between them.
    #[test]
    fn runs_stored_in_place_are_the_runs_and_nothing_else() {
        if !store_on() {
            return;
        }
        let len = 3 * 64 * 1024 + 8 * PAGE;
        for (seed, count) in [(1u64, 1usize), (2, 17), (3, 4_000), (4, 60_000)] {
            let base = committed(len / PAGE);
            // SAFETY: committed above.
            let title = unsafe { std::slice::from_raw_parts_mut(base, len) };
            let mut rng = Rng(seed * 977);
            title.iter_mut().for_each(|b| *b = rng.next() as u8);
            let before = title.to_vec();
            let from: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            let runs = runs(len, count, seed);
            assert!(store_runs(base as usize, &from, &runs), "seed {seed}");
            assert!(title == reference(&before, &from, &runs).as_slice(), "seed {seed}: {} runs", runs.len());
            // SAFETY: the test's reservation.
            unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
        }
    }

    /// A page in the middle that cannot be written (read-only, or not
    /// committed): the stores give up without a fault, the page keeps its
    /// bytes, and the kernel's write the caller falls back to finishes
    /// every run on the pages that are there, as before the stores in place.
    #[test]
    fn a_page_that_cannot_be_written_ends_the_stores_and_the_kernel_finishes() {
        use windows_sys::Win32::System::Memory::{MEM_DECOMMIT, PAGE_READONLY, VirtualProtect};
        if !store_on() {
            return;
        }
        for read_only in [true, false] {
            let pages = 6;
            let base = committed(pages);
            let len = pages * PAGE;
            // SAFETY: committed above.
            let title = unsafe { std::slice::from_raw_parts_mut(base, len) };
            title.fill(0x11);
            let hole = 3 * PAGE..4 * PAGE;
            // SAFETY: a page of the test's own reservation.
            unsafe {
                if read_only {
                    let mut old = 0;
                    assert_ne!(VirtualProtect(base.add(hole.start).cast(), PAGE, PAGE_READONLY, &mut old), 0);
                } else {
                    assert_ne!(VirtualFree(base.add(hole.start).cast(), PAGE, MEM_DECOMMIT), 0);
                }
            }
            let from = vec![0xEEu8; len];
            let runs = runs(len, 2_000, 7 + u64::from(read_only));
            assert!(runs.iter().any(|&(a, n)| a < hole.end && a + n > hole.start), "a run in the hole");
            assert!(!store_runs(base as usize, &from, &runs), "read_only {read_only}");
            // The caller's fall-back: the kernel's write, run by run.
            for &(off, n) in &runs {
                crate::host::write_mapped(base.wrapping_add(off), from[off..].as_ptr(), n);
            }
            for (k, &(off, n)) in runs.iter().enumerate() {
                for i in off..off + n {
                    // (The hole is the kernel's to leave or write, as before.)
                    if !hole.contains(&i) {
                        // SAFETY: committed and readable.
                        assert_eq!(unsafe { *base.add(i) }, 0xEE, "run {k}: byte {i} written");
                    }
                }
            }
            // The block that faulted is left to the kernel a while: no
            // second fault for it.
            assert!(!store_runs(base as usize, &from, &runs[..1]));
            // SAFETY: the test's reservation.
            unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
        }
    }

    /// Two views of one section show the same bytes: runs stored through
    /// one are there through the other.
    #[test]
    fn runs_stored_through_one_view_show_through_another() {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, UnmapViewOfFile,
        };
        if !store_on() {
            return;
        }
        let len = 2 * 64 * 1024;
        for place in &STORE_RECENT {
            place.store(0, Relaxed);
        }
        // SAFETY: a section and two views of the test's own, let go below.
        unsafe {
            let section =
                CreateFileMappingW(INVALID_HANDLE_VALUE, std::ptr::null(), PAGE_READWRITE, 0, len as u32, std::ptr::null());
            assert!(!section.is_null());
            let a = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, len).Value as *mut u8;
            let b = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, len).Value as *mut u8;
            assert!(!a.is_null() && !b.is_null() && a != b);
            let from: Vec<u8> = (0..len).map(|i| (i * 31 + 7) as u8).collect();
            let runs = runs(len, 5_000, 11);
            assert!(store_runs(a as usize, &from, &runs));
            let shown = std::slice::from_raw_parts(b, len);
            assert!(shown == reference(&vec![0u8; len], &from, &runs).as_slice());
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: a.cast() });
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: b.cast() });
            CloseHandle(section);
        }
    }

    /// Eight threads store their own runs into the same pages at once (as
    /// the merging threads do, each its own copy's stretch): every thread's
    /// runs land, none undoes another's.
    #[test]
    fn threads_store_runs_into_the_same_pages_side_by_side() {
        if !store_on() {
            return;
        }
        let len = 4 * 64 * 1024;
        let base = committed(len / PAGE) as usize;
        let all = runs(len, 40_000, 21);
        let threads: Vec<_> = (0..8usize)
            .map(|t| {
                let mine: Vec<(usize, usize)> = all.iter().copied().skip(t).step_by(8).collect();
                std::thread::spawn(move || {
                    let from = vec![t as u8 + 1; len];
                    for _ in 0..50 {
                        assert!(store_runs(base, &from, &mine));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        // SAFETY: committed above.
        let title = unsafe { std::slice::from_raw_parts(base as *const u8, len) };
        for (k, &(off, n)) in all.iter().enumerate() {
            assert!(title[off..off + n].iter().all(|&b| b == (k % 8) as u8 + 1), "run {k}");
        }
        // SAFETY: the test's reservation.
        unsafe { VirtualFree(base as *mut _, 0, MEM_RELEASE) };
    }

    #[test]
    fn threads_fault_and_copy_side_by_side() {
        // SAFETY: memory of the test's own.
        let base = unsafe { VirtualAlloc(std::ptr::null(), 2 * PAGE, MEM_RESERVE, PAGE_NOACCESS) } as usize;
        assert_ne!(base, 0);
        // SAFETY: the first page of that reservation.
        assert!(!unsafe { VirtualAlloc(base as *const _, PAGE, MEM_COMMIT, PAGE_READWRITE) }.is_null());
        let threads: Vec<_> = (0..8)
            .map(|t| {
                std::thread::spawn(move || {
                    let mut to = vec![0u8; PAGE];
                    for _ in 0..2_000 {
                        let from = (base + if t % 2 == 0 { 0 } else { PAGE }) as *const u8;
                        assert_eq!(guarded(to.as_mut_ptr(), from, PAGE), t % 2 == 0);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        // SAFETY: the test's reservation.
        unsafe { VirtualFree(base as *mut _, 0, MEM_RELEASE) };
    }
}
