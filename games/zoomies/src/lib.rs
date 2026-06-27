//! Zoomies — a rooftop infinite-runner on the scamper engine. You auto-run right
//! across a night-city skyline at ever-rising speed; the twist is that the four
//! graphics tiers (Kitty pixels → half-blocks → ASCII → mono) ARE your health bar.
//! Each hit drops you a tier; a hit at mono is fatal, and a fall between buildings
//! kills you outright. Score is distance run.
//!
//! This skeleton sets up the menu, difficulty, and high-score persistence; the
//! generator and gameplay land in later steps. Exposed as a library so the arcade
//! launcher can start it via [`launch`].

use scamper::input::{Input, K_DOWN, K_ESC, K_Q, K_S, K_SPACE, K_UP, K_W};
use scamper::menu::Menu;
use scamper::terminal;
use scamper::time::{now_ns, sleep_until_ns};
use scamper::ui;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

mod gen;

/// Enter key (kitty reports it as CSI 13 u). Space also selects, for legacy terminals.
const K_ENTER: u32 = 13;
/// How many scores we keep per difficulty.
const TOP_N: usize = 5;

/// Difficulty preset — scales start speed, ramp, gap sizes, obstacle density. The
/// gameplay tuning hangs off this in later steps; here it carries identity + labels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Difficulty {
    Cruise,
    Standard,
    Frantic,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Cruise, Difficulty::Standard, Difficulty::Frantic];

    /// Stable storage/CLI token.
    pub fn name(self) -> &'static str {
        match self {
            Difficulty::Cruise => "cruise",
            Difficulty::Standard => "standard",
            Difficulty::Frantic => "frantic",
        }
    }

    /// Human label for the menu.
    pub fn label(self) -> &'static str {
        match self {
            Difficulty::Cruise => "Cruise (easy)",
            Difficulty::Standard => "Standard",
            Difficulty::Frantic => "Frantic (hard)",
        }
    }

    pub fn from_str(s: &str) -> Difficulty {
        match s.trim().to_ascii_lowercase().as_str() {
            "cruise" | "easy" => Difficulty::Cruise,
            "frantic" | "hard" => Difficulty::Frantic,
            _ => Difficulty::Standard,
        }
    }

    /// Cycle to the next preset (wraps), for the menu's Difficulty row.
    pub fn next(self) -> Difficulty {
        match self {
            Difficulty::Cruise => Difficulty::Standard,
            Difficulty::Standard => Difficulty::Frantic,
            Difficulty::Frantic => Difficulty::Cruise,
        }
    }
}

/// Insert `dist` into a descending top-`n` list, returning the 0-based rank if it
/// placed (ties keep the incumbent ahead). Pure, so it unit-tests directly.
fn insert_ranked(list: &mut Vec<u32>, dist: u32, n: usize) -> Option<usize> {
    let pos = list.iter().position(|&d| dist > d).unwrap_or(list.len());
    if pos >= n {
        return None;
    }
    list.insert(pos, dist);
    list.truncate(n);
    Some(pos)
}

/// The on-disk save: the chosen difficulty plus a top-N distance table per
/// difficulty, in one tab-separated key/value file (`~/.zoomies`). Keys are
/// `difficulty` and `score.<name>` (a comma-separated descending list).
pub struct Save {
    path: PathBuf,
    kv: HashMap<String, String>,
}

impl Save {
    fn home_path() -> PathBuf {
        let dir = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        dir.join(".zoomies")
    }

    pub fn load() -> Self {
        Self::load_from(Self::home_path())
    }

    /// Load from an explicit path (used by tests).
    pub fn load_from(path: PathBuf) -> Self {
        let kv = scamper::store::load(&path);
        Save { path, kv }
    }

    pub fn difficulty(&self) -> Difficulty {
        self.kv.get("difficulty").map(|s| Difficulty::from_str(s)).unwrap_or(Difficulty::Standard)
    }

    pub fn set_difficulty(&mut self, d: Difficulty) {
        self.kv.insert("difficulty".into(), d.name().into());
        self.persist();
    }

    /// The top distances for a difficulty, descending.
    pub fn top(&self, d: Difficulty) -> Vec<u32> {
        self.kv
            .get(&format!("score.{}", d.name()))
            .map(|csv| csv.split(',').filter_map(|t| t.trim().parse::<u32>().ok()).collect())
            .unwrap_or_default()
    }

    /// Record a run's distance; returns the 0-based rank if it made the table.
    pub fn record(&mut self, d: Difficulty, dist: u32) -> Option<usize> {
        let mut list = self.top(d);
        let rank = insert_ranked(&mut list, dist, TOP_N);
        if rank.is_some() {
            let csv = list.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",");
            self.kv.insert(format!("score.{}", d.name()), csv);
            self.persist();
        }
        rank
    }

    fn persist(&self) {
        scamper::store::save(&self.path, &self.kv);
    }
}

/// Standalone entry: parse `--difficulty <name>` (persisting it), then show the menu.
pub fn run_cli(args: Vec<String>) {
    let debug = args.iter().any(|a| a == "--debug");
    let log_path = std::env::var("SCAMP_LOG").unwrap_or_else(|_| "zoomies.log".into());
    scamper::dbg::init(debug, &log_path);
    scamper::dbg::install_panic_logger();

    if let Some(d) = flag_value(&args, "--difficulty") {
        let mut save = Save::load();
        save.set_difficulty(Difficulty::from_str(&d));
    }
    launch();
}

/// The first value after `flag` in `args`, if present.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

/// Launch the Zoomies menu — the entry the arcade calls. Owns its own terminal guard.
pub fn launch() {
    let _guard = match terminal::TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("zoomies needs an interactive terminal (Kitty/Ghostty/foot). ({e})");
            return;
        }
    };
    let kitty_kbd = terminal::probe_kitty_keyboard();
    let mut input = Input::new(kitty_kbd);
    menu_loop(&mut input);
}

#[derive(Clone, Copy)]
enum Item {
    Run,
    Difficulty,
    Scores,
    Help,
    Back,
}
const ITEMS: [Item; 5] = [Item::Run, Item::Difficulty, Item::Scores, Item::Help, Item::Back];

fn menu_labels(diff: Difficulty) -> Vec<String> {
    ITEMS
        .iter()
        .map(|it| match it {
            Item::Run => "Run".to_string(),
            Item::Difficulty => format!("Difficulty: {}", diff.label()),
            Item::Scores => "High Scores".to_string(),
            Item::Help => "Help".to_string(),
            Item::Back => "Back".to_string(),
        })
        .collect()
}

fn menu_loop(input: &mut Input) {
    let mut save = Save::load();
    let mut diff = save.difficulty();
    let mut menu = Menu::new("⚡  Z O O M I E S  ⚡", menu_labels(diff));
    let mut out: Vec<u8> = Vec::new();

    loop {
        let ws = terminal::query_winsize();
        render_menu(&mut out, &menu, ws.cols, ws.rows);

        if terminal::quit_requested() {
            return;
        }
        input.poll();
        if input.quit {
            return;
        }
        if input.pressed(K_UP) || input.pressed(K_W) {
            menu.up();
        }
        if input.pressed(K_DOWN) || input.pressed(K_S) {
            menu.down();
        }
        let select = input.pressed(K_ENTER) || input.pressed(K_SPACE);
        let back = input.pressed(K_Q) || input.pressed(K_ESC);
        if back {
            return;
        }
        if select {
            match ITEMS[menu.selected()] {
                Item::Run => show_card(&mut out, input, &["Zoomies", "", "the run lands in a later step —", "rooftops, rising speed, fidelity = health.", "", "press any key"]),
                Item::Difficulty => {
                    diff = diff.next();
                    save.set_difficulty(diff);
                    menu.set_items(menu_labels(diff));
                }
                Item::Scores => show_scores(&mut out, input, &save),
                Item::Help => show_help(&mut out, input),
                Item::Back => return,
            }
        }
        sleep_until_ns(now_ns() + 16_000_000, 1_000_000);
    }
}

/// Paint the menu as a centered card, clearing the screen first.
fn render_menu(out: &mut Vec<u8>, menu: &Menu, cols: u16, rows: u16) {
    out.clear();
    out.extend_from_slice(b"\x1b[2J");
    let mut s = String::new();
    let top = (rows as i32 / 2 - 4).max(1) as u16;
    menu.render(&mut s, cols, top);
    out.extend_from_slice(s.as_bytes());
    flush(out);
}

/// Draw a centered text card and block until any key (or Esc/Q) is pressed.
fn show_card(out: &mut Vec<u8>, input: &mut Input, lines: &[&str]) {
    let ws = terminal::query_winsize();
    out.clear();
    out.extend_from_slice(b"\x1b[2J");
    let mut s = String::new();
    let top = (ws.rows as i32 / 2 - lines.len() as i32 / 2).max(1) as u16;
    ui::center_card(&mut s, ws.cols, top, lines, true);
    out.extend_from_slice(s.as_bytes());
    flush(out);

    // Swallow the select key still held from opening, then wait for a fresh press.
    loop {
        if terminal::quit_requested() {
            return;
        }
        input.poll();
        if input.quit || input.any_pressed() {
            return;
        }
        sleep_until_ns(now_ns() + 16_000_000, 1_000_000);
    }
}

fn show_help(out: &mut Vec<u8>, input: &mut Input) {
    show_card(
        out,
        input,
        &[
            "Z O O M I E S  —  how to play",
            "",
            "You auto-run right; the screen never waits.",
            "Jump:  Space  /  ↑      Fast-fall:  ↓",
            "",
            "Each hit drops your graphics one tier:",
            "Kitty → half-blocks → ASCII → mono.",
            "A hit at mono ends the run.",
            "Fall between buildings = instant death.",
            "",
            "Score = distance run. Pick difficulty in the menu.",
            "",
            "press any key",
        ],
    );
}

fn show_scores(out: &mut Vec<u8>, input: &mut Input, save: &Save) {
    let mut lines: Vec<String> = vec!["High Scores  (distance)".to_string(), String::new()];
    for d in Difficulty::ALL {
        let top = save.top(d);
        let best = top.first().copied().unwrap_or(0);
        lines.push(format!("{:<16} {:>6} m", d.label(), best));
    }
    lines.push(String::new());
    lines.push("press any key".to_string());
    let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    show_card(out, input, &refs);
}

fn flush(out: &[u8]) {
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(out);
    let _ = o.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_ranked_keeps_top_n_descending() {
        let mut l = vec![];
        assert_eq!(insert_ranked(&mut l, 100, 3), Some(0));
        assert_eq!(insert_ranked(&mut l, 50, 3), Some(1));
        assert_eq!(insert_ranked(&mut l, 200, 3), Some(0));
        assert_eq!(l, vec![200, 100, 50]);
        // Full list: a low score doesn't place; a high one bumps the tail.
        assert_eq!(insert_ranked(&mut l, 10, 3), None);
        assert_eq!(insert_ranked(&mut l, 150, 3), Some(1));
        assert_eq!(l, vec![200, 150, 100]);
    }

    #[test]
    fn insert_ranked_ties_keep_incumbent_ahead() {
        let mut l = vec![100];
        assert_eq!(insert_ranked(&mut l, 100, 5), Some(1)); // equal -> placed after
        assert_eq!(l, vec![100, 100]);
    }

    #[test]
    fn difficulty_round_trips_and_cycles() {
        for d in Difficulty::ALL {
            assert_eq!(Difficulty::from_str(d.name()), d);
        }
        assert_eq!(Difficulty::from_str("easy"), Difficulty::Cruise);
        assert_eq!(Difficulty::from_str("nonsense"), Difficulty::Standard);
        // next() cycles through all three.
        let mut d = Difficulty::Cruise;
        let mut seen = vec![d];
        for _ in 0..3 {
            d = d.next();
            seen.push(d);
        }
        assert_eq!(seen[3], Difficulty::Cruise, "wraps after three steps");
    }

    #[test]
    fn save_round_trips_difficulty_and_scores() {
        let mut path = std::env::temp_dir();
        path.push("zoomies_save_test.kv");
        let _ = std::fs::remove_file(&path);

        let mut s = Save::load_from(path.clone());
        assert_eq!(s.difficulty(), Difficulty::Standard, "default");
        s.set_difficulty(Difficulty::Frantic);
        assert_eq!(s.record(Difficulty::Frantic, 500), Some(0));
        assert_eq!(s.record(Difficulty::Frantic, 300), Some(1));

        // Reload from disk: settings + scores survived.
        let s2 = Save::load_from(path.clone());
        assert_eq!(s2.difficulty(), Difficulty::Frantic);
        assert_eq!(s2.top(Difficulty::Frantic), vec![500, 300]);
        assert!(s2.top(Difficulty::Cruise).is_empty());

        let _ = std::fs::remove_file(&path);
    }
}
