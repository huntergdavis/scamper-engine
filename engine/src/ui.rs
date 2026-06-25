//! Tiny terminal-UI helpers shared by games — overlay cards drawn as ANSI on top
//! of the rendered scene (title / pause / intro / results banners), so each game
//! doesn't hand-roll cursor math and inverse-video padding. They append ANSI to a
//! `String` (the games build their overlay text into one before writing it out).

use std::fmt::Write;

/// Append a centered, inverse-video text card to `out`. The block is horizontally
/// centered in a `cols`-wide terminal with its first line at `top_row` (1-based,
/// terminal cells); every line is padded to the widest so the inverse box is
/// flush. `bold` adds the bold attribute. Lines must be plain text (measured by
/// char count for centering — no embedded escapes).
pub fn center_card(out: &mut String, cols: u16, top_row: u16, lines: &[&str], bold: bool) {
    let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let col = (cols as usize).saturating_sub(w) / 2 + 1;
    let attr = if bold { "1;7" } else { "7" };
    for (i, line) in lines.iter().enumerate() {
        let row = top_row as usize + i;
        let _ = write!(out, "\x1b[{row};{col}H\x1b[{attr}m{line:^w$}\x1b[0m");
    }
}

/// Append a full-width status/banner line (inverse video) on `row`, clearing the
/// row first. Used for the pause bar and similar single-row overlays.
pub fn status_line(out: &mut String, row: u16, text: &str) {
    let _ = write!(out, "\x1b[{row};1H\x1b[2K\x1b[7m{text}\x1b[0m");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_card_centers_and_pads() {
        let mut out = String::new();
        center_card(&mut out, 20, 5, &["hi", "longer"], true);
        // Both lines padded to width 6 ("hi" → "  hi  "), centered in 20 cols → col 8.
        assert!(out.contains("\x1b[5;8H\x1b[1;7m  hi  \x1b[0m"), "line 1: {out:?}");
        assert!(out.contains("\x1b[6;8H\x1b[1;7mlonger\x1b[0m"), "line 2: {out:?}");
    }

    #[test]
    fn status_line_clears_and_inverts() {
        let mut out = String::new();
        status_line(&mut out, 25, "PAUSED");
        assert_eq!(out, "\x1b[25;1H\x1b[2K\x1b[7mPAUSED\x1b[0m");
    }
}
