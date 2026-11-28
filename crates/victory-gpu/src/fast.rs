//! The faster set of defaults (`VICTORY_FAST`; on unless `=0`).
//!
//! Switches the gta6 base's tester measured as gains on an RTX 4070
//! (README, "Frame rate, second night" and after: 4.5-5 frames a second
//! in the first rooms with the old defaults, 6-7 and later 9.6-10.3 with
//! these among others), that the owner's runs 1-9 never had because they
//! were off by default:
//!
//! - `VICTORY_ASYNC_SUBMIT`: submissions prepared and sent by a thread of
//!   ours, not the title's render thread;
//! - `VICTORY_QUEUE2=1`: fetches of what shaders wrote go to a copy
//!   queue (the copy-back thread's wait 125 -> 7 ms a frame there);
//! - `VICTORY_COPY_PLAIN`: copies of title memory in place, the kernel's
//!   copy behind them for a missing page (submit thread's kernel time
//!   44 -> 20 ms a frame);
//! - `VICTORY_JOIN_PARTS`: parts of one submission go to the host as one
//!   (about 6 %);
//! - `VICTORY_BACK_OWN_VIEW`: what shaders wrote goes back through a view
//!   of our own, so the copy that wrote it is not read in again (about
//!   9 %; 0 stores astray in 1.36 million checked);
//! - `VICTORY_CARD_REHASH_RUNS`: one hash pass per run of touching pieces
//!   (run 9 had up to 445,000 passes a frame);
//! - `VICTORY_WIDE_MERGE`: what shaders wrote to copies in the card's
//!   memory merged on several threads (now also on the copy-back thread,
//!   where run 9 spent about 270 ms a frame);
//! - `VICTORY_LEVELS_ON_DEMAND`: a texture takes the card's memory only for
//!   the levels the title holds (run 11 on the owner's PC, alone against
//!   run 10: 3.0 GB of the card kept free, textures turned down for room
//!   73,500 -> 58 times, frames of over 300 draws 225 -> 195 ms);
//! - `VICTORY_RECHECK_SOON`: a texture picked by index found changed is
//!   looked at again the next frame, not four frames later (on by default
//!   in the AMD sibling repository);
//! - `VICTORY_SETTLE_ROOM`: flights get twice the room before "lists in
//!   the air" settles them (the base: 11.5 ms less of the preparing
//!   thread's work a frame; run 12 here waited 14-29 ms a frame there);
//! - `VICTORY_LOOKUP_CACHE`: each recording thread remembers what its asks
//!   for a buffer's host copy found (the base, run 293: about 14 ms a
//!   frame; the pull request has it on by default);
//! - `VICTORY_PARALLEL_REFRESH`: a submission's texture checks run side by
//!   side on the helper threads (the base, two launches: 8.7 and 9.4 ms
//!   less of the preparing thread's work a frame);
//! - `VICTORY_CARD_SYNC_BATCH`: the list that brings copies in the card's
//!   memory up to date records all its copies, one state change, then all
//!   its hash passes, not copy by copy (fork pull request #1227, on by
//!   default there: the card's send-and-hash lists 24-28 -> 5.3-5.7 ms a
//!   frame);
//! - `VICTORY_REFRESH_PAGES`: a refresh of watched title memory takes the
//!   whole 64 KiB pages it touches (run 12 on the owner's PC: frames of
//!   over 300 draws 195 -> 171 ms median, copies 104 -> 89 MB a frame);
//! - `VICTORY_WRITE_PLAIN`: what shaders wrote is stored into title memory
//!   in place, the kernel's write behind it for a page that is not there
//!   (run 14: 139 ms a frame of the card copies' merges went to writing
//!   back, most stretches one `NtWriteVirtualMemory` per run of words;
//!   untried on the owner's PC before run 15, which has it `alt`);
//! - `VICTORY_GUARD_MARKS`: a write told before it lands (`watch::writing`,
//!   2,000 a frame: write-backs and storage) looks at lock-free marks of
//!   the guarded blocks and takes the guards' lock only if one is there
//!   (run 16: 49 ms a frame of waiting for that lock there, all threads);
//! - `VICTORY_TITLE_COPY_PLAIN`: the title's own copies of its memory made
//!   in place, large ones split over the helper threads, the kernel's copy
//!   behind them (run 19: 27 title copies, 24 ms of a heavy frame's
//!   preparing, all through the kernel on one thread);
//! - `VICTORY_EARLY_START`: a joined part goes to the host at once while
//!   the card has nothing of ours to run (run 19: the card idle 50-55 ms of
//!   a 140 ms frame while the submitting thread prepared).
//!
//! The runner's `VICTORY_OUTPUT` follows `VICTORY_FAST` too: a 2560x1440
//! output with the scene at 1920x1080, not the console's 3840x2160 with the
//! scene at 2560x1440 (`apps/victory/src/output.rs`).
//!
//! `SETTLE_ROOM`, `LOOKUP_CACHE`, `PARALLEL_REFRESH`, `CARD_SYNC_BATCH`,
//! `WRITE_PLAIN`, `TITLE_COPY_PLAIN` and `EARLY_START` also take `alt`
//! (twenty seconds each way, in turn).
//!
//! Each still takes its own `=0` (or `=1`); `VICTORY_FAST=0` puts every
//! one of them back to off, as in runs 1-9, in one go.

use std::sync::OnceLock;

/// The switches [`on`] gives a default to, for the start-up line.
const SWITCHES: [&str; 19] = [
    "VICTORY_ASYNC_SUBMIT",
    "VICTORY_QUEUE2",
    "VICTORY_COPY_PLAIN",
    "VICTORY_JOIN_PARTS",
    "VICTORY_BACK_OWN_VIEW",
    "VICTORY_CARD_REHASH_RUNS",
    "VICTORY_WIDE_MERGE",
    "VICTORY_RECHECK_SOON",
    "VICTORY_LEVELS_ON_DEMAND",
    "VICTORY_SETTLE_ROOM",
    "VICTORY_LOOKUP_CACHE",
    "VICTORY_PARALLEL_REFRESH",
    "VICTORY_CARD_SYNC_BATCH",
    "VICTORY_REFRESH_PAGES",
    "VICTORY_WRITE_PLAIN",
    "VICTORY_GUARD_MARKS",
    "VICTORY_TITLE_COPY_PLAIN",
    "VICTORY_EARLY_START",
    "VICTORY_OUTPUT",
];

/// Whether the faster defaults hold (`VICTORY_FAST`, unset or anything but
/// `0`).
pub(crate) fn fast() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("VICTORY_FAST").is_none_or(|v| v != "0");
        let set: Vec<String> = SWITCHES
            .iter()
            .filter_map(|s| std::env::var(s).ok().map(|v| format!("{s}={v}")))
            .collect();
        tracing::info!(
            "faster defaults (VICTORY_FAST): {}; set by hand: {}",
            if on {
                "ON (=0 puts them all back to off)"
            } else {
                "off, as in runs 1-9 (and the switches added since)"
            },
            if set.is_empty() {
                "none".to_owned()
            } else {
                set.join(" ")
            }
        );
        on
    })
}

/// A plain on/off switch of the set: its own value when set (`0` or empty
/// off, anything else on), else [`fast`].
pub(crate) fn on(var: &str) -> bool {
    decide(
        std::env::var_os(var).as_deref().and_then(|v| v.to_str()),
        fast,
    )
}

fn decide(value: Option<&str>, fast: impl FnOnce() -> bool) -> bool {
    match value {
        Some(v) => !v.is_empty() && v != "0",
        None => fast(),
    }
}

#[cfg(test)]
mod tests {
    use super::decide;

    #[test]
    fn a_switch_set_by_hand_wins_over_the_set() {
        assert!(decide(Some("1"), || false));
        assert!(!decide(Some("0"), || true));
        assert!(!decide(Some(""), || true));
        assert!(decide(None, || true));
        assert!(!decide(None, || false));
    }
}
