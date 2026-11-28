//! The settings Defeat's F1 menu saves (`victory_gpu`'s `menu`), applied at
//! the start of a run: each `VICTORY_...=value` line of the file sets that
//! switch (an empty value unsets it), over what the window the game was
//! started from set. The menu is told where the file is
//! (`VICTORY_MENU_FILE`).

use std::path::Path;

/// The `(switch, value)` lines of a settings file's text; other lines
/// (comments, blank ones, anything not a `VICTORY_` switch) are left out.
fn parse(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            let valid = key.starts_with("VICTORY_") && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            // The menu's own file name is the runner's to say.
            (valid && key != "VICTORY_MENU_FILE").then(|| (key.to_owned(), value.trim().to_owned()))
        })
        .collect()
}

/// Applies `file` if it is there, and names it for the menu.
pub(crate) fn apply(file: &Path) {
    // SAFETY: called first thing in a run, before any other thread starts
    // or any switch is read.
    unsafe { std::env::set_var("VICTORY_MENU_FILE", file) };
    let Ok(text) = std::fs::read_to_string(file) else {
        println!("settings       none saved by the F1 menu yet ({})", file.display());
        return;
    };
    let lines = parse(&text);
    for (key, value) in &lines {
        // SAFETY: as above.
        unsafe {
            if value.is_empty() {
                std::env::remove_var(key);
            } else {
                std::env::set_var(key, value);
            }
        }
    }
    let shown: Vec<String> = lines.iter().map(|(k, v)| format!("{k}={v}")).collect();
    println!(
        "settings       from the F1 menu ({}): {}",
        file.display(),
        if shown.is_empty() { "none".to_owned() } else { shown.join(" ") }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_switch_lines_are_taken() {
        let text = "# a note\n\nVICTORY_OUTPUT=1080p\n  VICTORY_FAST = 0 \nPATH=C:\\x\nVICTORY_MENU_FILE=elsewhere\nVICTORY_lower=1\nVICTORY_BVH_BUILDER=\nnot a line\n";
        assert_eq!(
            parse(text),
            [
                ("VICTORY_OUTPUT".to_owned(), "1080p".to_owned()),
                ("VICTORY_FAST".to_owned(), "0".to_owned()),
                ("VICTORY_BVH_BUILDER".to_owned(), String::new()),
            ]
        );
    }
}
