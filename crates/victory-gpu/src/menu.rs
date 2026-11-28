//! Defeat's F1 settings menu.
//!
//! F1 shows and hides it while one of our windows has the focus. Up and
//! Down pick a line, Left and Right change a setting, Enter runs the line
//! picked, Esc closes. While it is shown the title sees no keyboard
//! ([`open`], read by the input module), so the arrows and Enter do not
//! also move the game; a pad still works.
//!
//! The layer reads its settings once, at start, so a change is saved at
//! once to the settings file and most take effect at the next start. The
//! runner applies the file before anything reads a switch (it names the
//! file in `VICTORY_MENU_FILE`: `settings\defeat-settings.txt` in the data folder,
//! `%LOCALAPPDATA%\victory\data` unless `victory.toml` says otherwise);
//! what the file holds wins over the same switch set in the window the
//! game was started from. The frame-rate box changes at once.
//!
//! Drawn as the frame-rate box is (`crate::fpsbox`): text of 5 by 7 dots
//! on a dark box, made on the CPU into buffers the card copies onto the
//! swap chain's buffer, here in the middle of the picture.
//!
//! VIctory-NVIDIA has a menu of the same kind (`host/menu.rs`, F1 and the
//! same keys); this one is Defeat's own and holds Defeat's settings.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;

/// One setting: its switch, its name in the menu, its values (as written,
/// as shown; the first is what the layer does when the switch is not set),
/// and whether it changes at once.
struct Item {
    var: &'static str,
    label: &'static str,
    values: &'static [(&'static str, &'static str)],
    live: bool,
}

const ITEMS: &[Item] = &[
    Item {
        var: "VICTORY_OUTPUT",
        label: "RENDER SIZE",
        values: &[
            ("1080p", "1080P"),
            ("1080p-720p", "720P UP TO 1080P"),
            ("1440p-1080p", "1080P UP TO 1440P"),
            ("1440p", "1440P"),
            ("4k", "1440P UP TO 4K (CONSOLE)"),
        ],
        live: false,
    },
    Item { var: "VICTORY_FPS_COUNTER", label: "FPS BOX", values: &[("all", "FPS AND LOAD"), ("fps", "FPS ONLY"), ("0", "OFF")], live: true },
    Item { var: "VICTORY_FAST", label: "FASTER DEFAULTS", values: &[("1", "ON"), ("0", "OFF")], live: false },
    Item { var: "VICTORY_EARLY_START", label: "EARLY START", values: &[("1", "ON"), ("0", "OFF"), ("alt", "IN TURN")], live: false },
    Item { var: "VICTORY_TITLE_COPY_PLAIN", label: "TITLE COPIES IN PLACE", values: &[("1", "ON"), ("0", "OFF"), ("alt", "IN TURN")], live: false },
    Item { var: "VICTORY_BVH_BUILDER", label: "RAY TRACING (SLOW)", values: &[("0", "OFF"), ("1", "ON")], live: false },
];

/// The lines after the settings.
const ACTIONS: [&str; 3] = ["SAVE AND QUIT THE GAME", "RESET ALL TO DEFAULTS", "CLOSE THE MENU"];
const QUIT: usize = 0;
const RESET: usize = 1;
const CLOSE: usize = 2;

/// Glyphs across a line of the menu.
const COLUMNS: usize = 54;
/// Where a setting's value begins on its line.
const VALUE_AT: usize = 26;

/// Whether the menu is shown: the input module gives the title no keys then.
static OPEN: AtomicBool = AtomicBool::new(false);

/// Whether the F1 menu is shown (for the input module).
pub fn open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

/// What the menu shows: each setting's value (an index into its values),
/// the line picked, the note under the lines, and an action waiting for a
/// second Enter.
struct State {
    chosen: Vec<usize>,
    row: usize,
    note: String,
    confirm: Option<usize>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// The value `item`'s switch has in this run: its place among the values,
/// the first if it is not set or not one of them.
fn current(item: &Item, set: Option<&str>) -> usize {
    set.and_then(|v| item.values.iter().position(|(value, _)| value.eq_ignore_ascii_case(v.trim()))).unwrap_or(0)
}

fn state() -> std::sync::MutexGuard<'static, Option<State>> {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.is_none() {
        let chosen = ITEMS.iter().map(|item| current(item, std::env::var(item.var).ok().as_deref())).collect();
        *state = Some(State { chosen, row: 0, note: String::new(), confirm: None });
    }
    state
}

/// The file the menu saves to.
fn settings_file() -> Option<std::path::PathBuf> {
    if let Some(file) = std::env::var_os("VICTORY_MENU_FILE") {
        return Some(file.into());
    }
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(std::path::PathBuf::from(base).join("victory").join("data").join("settings").join("defeat-settings.txt"))
}

/// The settings file's text for `chosen` (every setting written out, so
/// that a switch set in the starting window does not come back over a
/// choice made here).
fn file_text(chosen: &[usize]) -> String {
    let mut text = String::from(
        "# Defeat's settings, written by the F1 menu and applied by the runner at every start.\n\
         # Delete this file (or use RESET ALL TO DEFAULTS) to go back to the defaults.\n",
    );
    for (item, &at) in ITEMS.iter().zip(chosen) {
        text.push_str(&format!("{}={}\n", item.var, item.values[at].0));
    }
    text
}

fn save(state: &mut State) {
    let Some(file) = settings_file() else {
        state.note = "NO PLACE TO SAVE THE SETTINGS".into();
        return;
    };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&file, file_text(&state.chosen)) {
        Ok(()) => tracing::info!(file = %file.display(), "the F1 menu saved the settings"),
        Err(e) => {
            tracing::warn!(file = %file.display(), error = %e, "the F1 menu could not save the settings");
            state.note = "THE SETTINGS COULD NOT BE SAVED".into();
        }
    }
}

/// A key pressed while the menu may be shown.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Key {
    F1,
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
}

/// What `key` does to the menu. Returns whether the game is to be closed.
fn press(state: &mut State, key: Key) -> bool {
    let lines = ITEMS.len() + ACTIONS.len();
    if key != Key::Enter {
        state.confirm = None;
    }
    match key {
        Key::F1 | Key::Esc => {
            let shown = key == Key::F1 && !open();
            OPEN.store(shown, Ordering::Relaxed);
            state.note.clear();
        }
        Key::Up => state.row = (state.row + lines - 1) % lines,
        Key::Down => state.row = (state.row + 1) % lines,
        Key::Left | Key::Right if state.row < ITEMS.len() => {
            let item = &ITEMS[state.row];
            let n = item.values.len();
            let at = &mut state.chosen[state.row];
            *at = if key == Key::Right { (*at + 1) % n } else { (*at + n - 1) % n };
            let value = item.values[*at].0;
            if item.live && item.var == "VICTORY_FPS_COUNTER" {
                crate::fpsbox::set_mode(value);
            }
            state.note = if item.live { "SAVED, CHANGED NOW".into() } else { "SAVED, USED FROM THE NEXT START".into() };
            save(state);
        }
        Key::Left | Key::Right => {}
        Key::Enter => match state.row.checked_sub(ITEMS.len()) {
            None => {}
            Some(QUIT) => {
                if state.confirm == Some(QUIT) {
                    save(state);
                    OPEN.store(false, Ordering::Relaxed);
                    return true;
                }
                state.confirm = Some(QUIT);
                state.note = "ENTER AGAIN: SAVE AND CLOSE THE GAME".into();
            }
            Some(RESET) => {
                state.chosen = vec![0; ITEMS.len()];
                crate::fpsbox::set_mode(ITEMS.iter().find(|i| i.var == "VICTORY_FPS_COUNTER").map_or("all", |i| i.values[0].0));
                save(state);
                state.note = "DEFAULTS SAVED, USED FROM THE NEXT START".into();
            }
            Some(CLOSE) => {
                OPEN.store(false, Ordering::Relaxed);
                state.note.clear();
            }
            Some(_) => {}
        },
    }
    false
}

/// The menu's lines, each [`COLUMNS`] glyphs.
fn lines(state: &State) -> Vec<String> {
    let fit = |line: String| {
        let mut out: String = line.chars().take(COLUMNS).collect();
        out.extend(std::iter::repeat_n(' ', COLUMNS - out.chars().count()));
        out
    };
    let mut out = vec![
        fit("DEFEAT SETTINGS                      F1 OR ESC CLOSES".into()),
        fit("UP/DOWN PICK   LEFT/RIGHT CHANGE   ENTER DOES IT".into()),
        fit(String::new()),
    ];
    for (i, item) in ITEMS.iter().enumerate() {
        let mark = if state.row == i { '>' } else { ' ' };
        let mut line = format!("{mark} {}", item.label);
        line.extend(std::iter::repeat_n(' ', VALUE_AT.saturating_sub(line.chars().count())));
        line.push_str(&format!("< {} >", item.values[state.chosen[i]].1));
        out.push(fit(line));
    }
    out.push(fit(String::new()));
    for (i, action) in ACTIONS.iter().enumerate() {
        let mark = if state.row == ITEMS.len() + i { '>' } else { ' ' };
        out.push(fit(format!("{mark} {action}")));
    }
    out.push(fit(String::new()));
    out.push(fit("DLSS AND FRAME GENERATION: NOT YET, THEY COME NEXT".into()));
    out.push(fit(if state.note.is_empty() { "CHANGES ARE SAVED AT ONCE".into() } else { state.note.clone() }));
    out
}

/// Starts the thread that reads F1 and the menu's keys (once).
fn listen() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let started = std::thread::Builder::new().name("victory-menu-keys".into()).spawn(|| {
            use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
            const KEYS: [(i32, Key); 7] = [
                (0x70, Key::F1),
                (0x26, Key::Up),
                (0x28, Key::Down),
                (0x25, Key::Left),
                (0x27, Key::Right),
                (0x0D, Key::Enter),
                (0x1B, Key::Esc),
            ];
            let mut held = [false; KEYS.len()];
            loop {
                std::thread::sleep(std::time::Duration::from_millis(10));
                let focused = focused();
                for (i, &(vk, key)) in KEYS.iter().enumerate() {
                    // SAFETY: a plain query.
                    let down = focused && unsafe { GetAsyncKeyState(vk) } < 0;
                    let pressed = down && !held[i];
                    held[i] = down;
                    // Only F1 while the menu is hidden: the other keys are the game's then.
                    if !pressed || (key != Key::F1 && !open()) {
                        continue;
                    }
                    let quit = {
                        let mut state = state();
                        let Some(state) = state.as_mut() else { continue };
                        press(state, key)
                    };
                    if quit {
                        close_game();
                    }
                }
            }
        });
        if let Err(e) = started {
            tracing::warn!(error = %e, "no thread for the F1 menu's keys: no menu");
        } else {
            tracing::info!("F1 opens Defeat's settings menu");
        }
    });
}

/// Whether the window in front is one of this process's.
fn focused() -> bool {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let mut process = 0;
    // SAFETY: plain queries; no window gives process 0.
    unsafe {
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut process));
        process == GetCurrentProcessId()
    }
}

/// Closes the game the way the window's close button does (the window
/// procedure ends the title as the console would).
fn close_game() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, PostMessageW, SC_CLOSE, WM_SYSCOMMAND};
    tracing::info!("the F1 menu closes the game");
    // SAFETY: a message to our own window, in front (`focused`).
    unsafe {
        let window = GetForegroundWindow();
        let _ = PostMessageW(Some(window), WM_SYSCOMMAND, WPARAM(SC_CLOSE as usize), LPARAM(0));
    }
}

/// A buffer the card copies the menu from.
struct Made {
    buffer: ID3D12Resource,
    data: *mut u8,
    width: u32,
    height: u32,
    pitch: u32,
}

// SAFETY: a free-threaded D3D12 object, its mapping (written under
// `BOARD`'s lock only) and plain numbers.
unsafe impl Send for Made {}

/// Buffers the menu is drawn into, in turn: one is written again only
/// three texts after it was last recorded, so a present still in the air
/// reads what it was recorded with.
const RING: usize = 4;

#[derive(Default)]
struct Board {
    made_for: Option<(i32, u32)>,
    shown: Option<Vec<String>>,
    ring: Vec<Made>,
    at: usize,
}

static BOARD: Mutex<Option<Board>> = Mutex::new(None);

/// Copies the menu, if shown, onto `back` (`width` by `height` pixels of
/// `format`, a copy's destination in `list`), in the middle. Starts the
/// key thread at the first call.
pub(crate) fn draw(host: &crate::host::Host, list: &ID3D12GraphicsCommandList, back: &ID3D12Resource, (width, height, format): (u32, u32, DXGI_FORMAT)) {
    listen();
    if !open() {
        return;
    }
    let Some(bytes) = crate::fpsbox::pixel_bytes(format) else {
        return;
    };
    let text = {
        let state = state();
        let Some(state) = state.as_ref() else { return };
        lines(state)
    };
    // A glyph about a sixtieth of the frame's height: 3 dots a pixel at 1080 lines.
    let mut scale = (height / 360).clamp(1, 12);
    while scale > 1 && (COLUMNS as u32 * 6 + 4) * scale > width {
        scale -= 1;
    }
    let mut board = BOARD.lock().unwrap_or_else(|e| e.into_inner());
    let board = board.get_or_insert_with(Board::default);
    let made_for = Some((format.0, scale));
    if board.made_for != made_for {
        // A present still in the air may read the old buffers: kept.
        std::mem::forget(std::mem::take(&mut board.ring));
        *board = Board { made_for, ..Default::default() };
    }
    if board.shown.as_ref() != Some(&text) {
        let (w, h, lit) = crate::fpsbox::bitmap(&text.join("\n"), scale);
        let pitch = (w as usize * bytes).next_multiple_of(D3D12_TEXTURE_DATA_PITCH_ALIGNMENT as usize);
        let at = if board.ring.len() < RING {
            let Some((buffer, data)) = host.shared_buffer((pitch * h as usize) as u64) else {
                return;
            };
            board.ring.push(Made { buffer, data, width: w, height: h, pitch: pitch as u32 });
            board.ring.len() - 1
        } else {
            (board.at + 1) % RING
        };
        let m = &board.ring[at];
        if (m.width, m.height, m.pitch) != (w, h, pitch as u32) {
            return;
        }
        // SAFETY: the buffer's own mapped bytes, `pitch * h` of them; the
        // card last read them at least `RING - 1` texts ago.
        let out = unsafe { std::slice::from_raw_parts_mut(m.data, pitch * h as usize) };
        for y in 0..h as usize {
            for x in 0..w as usize {
                let to = y * pitch + x * bytes;
                crate::fpsbox::pixel(format, lit[y * w as usize + x], &mut out[to..to + bytes]);
            }
        }
        board.at = at;
        board.shown = Some(text);
    }
    let Some(m) = board.ring.get(board.at) else {
        return;
    };
    if m.width > width || m.height > height {
        return;
    }
    let (x, y) = ((width - m.width) / 2, (height - m.height) / 2);
    // SAFETY: an open list; the back buffer is a copy's destination in it
    // and the menu lies inside it; our buffer holds `pitch * height` bytes
    // and is kept for as long as the process runs.
    unsafe {
        let from = D3D12_TEXTURE_COPY_LOCATION {
            pResource: std::mem::transmute_copy(&m.buffer),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                    Offset: 0,
                    Footprint: D3D12_SUBRESOURCE_FOOTPRINT { Format: format, Width: m.width, Height: m.height, Depth: 1, RowPitch: m.pitch },
                },
            },
        };
        let to = D3D12_TEXTURE_COPY_LOCATION {
            pResource: std::mem::transmute_copy(back),
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
        };
        list.CopyTextureRegion(&to, x, y, 0, &from, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> State {
        State { chosen: vec![0; ITEMS.len()], row: 0, note: String::new(), confirm: None }
    }

    #[test]
    fn every_line_fits_and_every_glyph_is_drawn() {
        let mut state = fresh();
        for row in 0..ITEMS.len() + ACTIONS.len() {
            state.row = row;
            for i in 0..ITEMS.len() {
                for v in 0..ITEMS[i].values.len() {
                    state.chosen[i] = v;
                    for line in lines(&state) {
                        assert_eq!(line.chars().count(), COLUMNS, "{line:?}");
                        for c in line.chars().filter(|&c| c != ' ') {
                            assert!(crate::fpsbox::glyph(c).iter().any(|&r| r != 0), "no glyph for {c:?} in {line:?}");
                        }
                    }
                }
                state.chosen[i] = 0;
            }
        }
        // Every label leaves room for its value.
        assert!(ITEMS.iter().all(|i| i.label.len() + 2 < VALUE_AT));
    }

    #[test]
    fn keys_move_change_and_quit_after_a_second_enter() {
        let mut state = fresh();
        // Up from the top goes round to the last line.
        assert!(!press(&mut state, Key::Up));
        assert_eq!(state.row, ITEMS.len() + ACTIONS.len() - 1);
        assert!(!press(&mut state, Key::Down));
        assert_eq!(state.row, 0);
        // (Left and Right save the file: not pressed here, to leave the
        // user's settings alone.)
        state.row = ITEMS.len() + QUIT;
        assert!(!press(&mut state, Key::Enter), "the first Enter only asks");
        assert_eq!(state.confirm, Some(QUIT));
        assert!(!press(&mut state, Key::Down), "another key forgets the ask");
        assert_eq!(state.confirm, None);
    }

    #[test]
    fn the_file_names_every_setting_with_a_value_the_switch_takes() {
        let chosen: Vec<usize> = ITEMS.iter().map(|i| i.values.len() - 1).collect();
        let text = file_text(&chosen);
        for (item, &at) in ITEMS.iter().zip(&chosen) {
            assert!(text.contains(&format!("\n{}={}\n", item.var, item.values[at].0)), "{}", item.var);
            // Read back, each value is the one written.
            assert_eq!(current(item, Some(item.values[at].0)), at);
        }
        assert_eq!(current(&ITEMS[0], Some("nonsense")), 0);
    }
}
