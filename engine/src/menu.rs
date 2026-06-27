//! A tiny vertical list-menu: a cursor over labeled items, rendered as a centered
//! inverse-video card (reusing [`ui::center_card`]). It's pure state — `up`/`down`/
//! `select` mutate a cursor and `render` paints it — so games map their own keys to it
//! (the arcade's game picker, Zoomies' main menu) and it unit-tests with no terminal.

use crate::ui;

/// A vertical menu: a title, a list of item labels, and a highlighted cursor.
pub struct Menu {
    pub title: String,
    pub items: Vec<String>,
    cursor: usize,
}

impl Menu {
    pub fn new(title: impl Into<String>, items: Vec<String>) -> Self {
        Menu { title: title.into(), items, cursor: 0 }
    }

    /// Move the highlight up one row, wrapping to the bottom.
    pub fn up(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.cursor = if self.cursor == 0 { self.items.len() - 1 } else { self.cursor - 1 };
    }

    /// Move the highlight down one row, wrapping to the top.
    pub fn down(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.cursor = (self.cursor + 1) % self.items.len();
    }

    /// The currently highlighted index (0 when empty).
    pub fn selected(&self) -> usize {
        self.cursor
    }

    /// Replace the item labels in place (e.g. when a "Difficulty: X" label changes),
    /// clamping the cursor so it stays in range.
    pub fn set_items(&mut self, items: Vec<String>) {
        self.items = items;
        if self.cursor >= self.items.len() {
            self.cursor = self.items.len().saturating_sub(1);
        }
    }

    /// Append the menu to `out` as a centered inverse-video card: the title, a blank
    /// spacer, then each item with the selected row marked by `▶`. `top_row` is the
    /// 1-based terminal row of the card's first line.
    pub fn render(&self, out: &mut String, cols: u16, top_row: u16) {
        let mut lines: Vec<String> = Vec::with_capacity(self.items.len() + 2);
        lines.push(self.title.clone());
        lines.push(String::new());
        for (i, it) in self.items.iter().enumerate() {
            let marker = if i == self.cursor { "▶ " } else { "  " };
            lines.push(format!("{marker}{it}"));
        }
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        ui::center_card(out, cols, top_row, &refs, true);
    }

    /// Like [`render`](Self::render) but shows at most `max_rows` items, scrolling a
    /// window to keep the cursor in view (for long lists like a level browser). A
    /// "↑/↓ more" hint marks a clipped end.
    pub fn render_windowed(&self, out: &mut String, cols: u16, top_row: u16, max_rows: usize) {
        let n = self.items.len();
        let win = max_rows.max(1);
        // Start so the cursor is centered, clamped to the list ends.
        let start = if n <= win { 0 } else { self.cursor.saturating_sub(win / 2).min(n - win) };
        let end = (start + win).min(n);
        let mut lines: Vec<String> = vec![self.title.clone(), String::new()];
        if start > 0 {
            lines.push("   ↑ more".to_string());
        }
        for i in start..end {
            let marker = if i == self.cursor { "▶ " } else { "  " };
            lines.push(format!("{marker}{}", self.items[i]));
        }
        if end < n {
            lines.push("   ↓ more".to_string());
        }
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        ui::center_card(out, cols, top_row, &refs, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> Menu {
        Menu::new("Title", vec!["a".into(), "b".into(), "c".into()])
    }

    #[test]
    fn down_wraps_to_top() {
        let mut m = menu();
        assert_eq!(m.selected(), 0);
        m.down();
        m.down();
        assert_eq!(m.selected(), 2);
        m.down(); // wrap
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn up_wraps_to_bottom() {
        let mut m = menu();
        m.up(); // from 0 wraps to last
        assert_eq!(m.selected(), 2);
        m.up();
        assert_eq!(m.selected(), 1);
    }

    #[test]
    fn set_items_clamps_cursor() {
        let mut m = menu();
        m.down();
        m.down(); // cursor = 2
        m.set_items(vec!["only".into()]);
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn empty_menu_is_inert() {
        let mut m = Menu::new("Empty", vec![]);
        m.down();
        m.up();
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn windowed_render_scrolls_to_keep_cursor_visible() {
        let items: Vec<String> = (0..20).map(|i| format!("item{i}")).collect();
        let mut m = Menu::new("Long", items);
        for _ in 0..15 {
            m.down(); // cursor = 15
        }
        let mut out = String::new();
        m.render_windowed(&mut out, 40, 2, 5);
        assert!(out.contains("item15"), "cursor item visible: {out:?}");
        assert!(out.contains("↑ more") && out.contains("↓ more"), "both ends clipped");
        assert!(!out.contains("item0"), "far items windowed out");
    }

    #[test]
    fn render_marks_selection_in_a_centered_card() {
        let mut m = menu();
        m.down(); // select "b"
        let mut out = String::new();
        m.render(&mut out, 40, 4);
        assert!(out.contains("▶ b"), "selected row marked: {out:?}");
        assert!(out.contains("  a") && out.contains("  c"), "others unmarked: {out:?}");
        assert!(out.contains("Title"), "title present: {out:?}");
    }
}
