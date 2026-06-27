//! The scamper arcade — a thin launcher. Its menu lists the sample games; pick one
//! and it runs that game's own loop (its own menus, help, settings) as-is. The shell
//! only does game selection.
//!
//! Terminal-guard handoff: each game enters its own `TerminalGuard`, so the arcade
//! must not hold one while a game runs. Each menu pass takes a guard for the duration
//! of the menu, drops it before launching the chosen game, then re-enters on return.

use scamper::input::{Input, K_DOWN, K_ENTER, K_ESC, K_Q, K_S, K_SPACE, K_UP, K_W};
use scamper::menu::Menu;
use scamper::terminal;
use scamper::time::{now_ns, sleep_until_ns};
use std::io::Write;

#[derive(Clone, Copy)]
enum Game {
    Munchii,
    Zoomies,
}

/// Menu rows, in order. The trailing Quit row has no game.
const ROWS: [(&str, Option<Game>); 3] =
    [("Super Munchii", Some(Game::Munchii)), ("Zoomies", Some(Game::Zoomies)), ("Quit", None)];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let debug = args.iter().any(|a| a == "--debug");
    scamper::dbg::init(debug, &std::env::var("SCAMP_LOG").unwrap_or_else(|_| "arcade.log".into()));
    scamper::dbg::install_panic_logger();

    // `--game <name>` jumps straight into a game, skipping the menu.
    if let Some(name) = flag_value(&args, "--game") {
        match name.to_ascii_lowercase().as_str() {
            "supermunchii" | "munchii" => supermunchii::launch(),
            "zoomies" => zoomies::launch(),
            other => eprintln!("arcade: unknown game {other:?} (try: supermunchii, zoomies)"),
        }
        return;
    }
    menu_loop();
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn run_game(g: Game) {
    match g {
        Game::Munchii => supermunchii::launch(),
        Game::Zoomies => zoomies::launch(),
    }
}

fn menu_loop() {
    let labels: Vec<String> = ROWS.iter().map(|(l, _)| l.to_string()).collect();
    let mut menu = Menu::new("◆  S C A M P E R   A R C A D E  ◆", labels);

    loop {
        // A guard for this menu pass only — dropped before we launch a game so the
        // game can enter its own.
        let choice = {
            let _guard = match terminal::TerminalGuard::enter() {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("arcade needs an interactive terminal (Kitty/Ghostty/foot). ({e})");
                    return;
                }
            };
            let kitty_kbd = terminal::probe_kitty_keyboard();
            let mut input = Input::new(kitty_kbd);
            run_menu(&mut input, &mut menu)
        }; // guard drops here

        match choice {
            Some(g) => run_game(g),
            None => return, // Quit (or Esc / Ctrl-C)
        }
    }
}

/// Render + poll until the user selects a row. `Some(game)` to launch, `None` to quit
/// (Quit row, Esc/Q, or Ctrl-C).
fn run_menu(input: &mut Input, menu: &mut Menu) -> Option<Game> {
    let mut out: Vec<u8> = Vec::new();
    loop {
        let ws = terminal::query_winsize();
        out.clear();
        out.extend_from_slice(b"\x1b[2J");
        let mut s = String::new();
        let top = (ws.rows as i32 / 2 - 3).max(1) as u16;
        menu.render(&mut s, ws.cols, top);
        // A hint line under the card.
        let hint = "↑/↓ move · Enter select · q quit";
        let col = ((ws.cols as i32 - hint.chars().count() as i32) / 2).max(0) + 1;
        s.push_str(&format!("\x1b[{};{}H{}", (top as i32 + 9).max(1), col, hint));
        out.extend_from_slice(s.as_bytes());
        {
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.flush();
        }

        if terminal::quit_requested() {
            return None;
        }
        input.poll();
        if input.quit || input.pressed(K_Q) || input.pressed(K_ESC) {
            return None;
        }
        if input.pressed(K_UP) || input.pressed(K_W) {
            menu.up();
        }
        if input.pressed(K_DOWN) || input.pressed(K_S) {
            menu.down();
        }
        if input.pressed(K_ENTER) || input.pressed(K_SPACE) {
            return ROWS[menu.selected()].1;
        }
        sleep_until_ns(now_ns() + 16_000_000, 1_000_000);
    }
}
