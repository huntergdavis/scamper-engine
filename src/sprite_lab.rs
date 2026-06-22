//! sprite-lab — a standalone tool (built on the Scamper engine) for viewing
//! sprite animations in the terminal. As the engine's demo games gain sprites,
//! register their animation sets here and the lab can play them all back.
//!
//! Controls: space/Tab → next animation, s → next sprite set, q/Esc → quit.

use scamper::input::{Input, K_ESC, K_Q, K_S, K_SPACE, K_TAB};
use scamper::munchii::{Anim, ALL as MUNCHII};
use scamper::terminal;
use scamper::time::{now_ns, sleep_until_ns, NS_PER_SEC};
use std::io::Write;

/// A named collection of animations (one character's sprite sheet).
struct SpriteSet {
    name: &'static str,
    anims: &'static [Anim],
}

/// Everything the lab can show. Append sets here as the engine grows.
const SETS: &[SpriteSet] = &[SpriteSet { name: "munchii", anims: MUNCHII }];

fn main() {
    let guard = match terminal::TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("sprite-lab needs an interactive terminal: {e}");
            return;
        }
    };
    let kitty_kbd = terminal::probe_kitty_keyboard();
    let mut input = Input::new(kitty_kbd);

    let mut si = 0usize; // sprite set
    let mut ai = 0usize; // animation
    let mut fi = 0usize; // frame
    let mut last = now_ns();
    let mut buf = String::new();
    use std::fmt::Write as _;

    loop {
        if terminal::quit_requested() || input.quit {
            break;
        }
        input.poll();
        if input.pressed(K_Q) || input.pressed(K_ESC) {
            break;
        }
        if input.pressed(K_S) && SETS.len() > 1 {
            si = (si + 1) % SETS.len();
            ai = 0;
            fi = 0;
            last = now_ns();
        }
        if input.pressed(K_SPACE) || input.pressed(K_TAB) {
            ai = (ai + 1) % SETS[si].anims.len();
            fi = 0;
            last = now_ns();
        }

        let set = &SETS[si];
        let anim = &set.anims[ai];
        let now = now_ns();
        if anim.frames.len() > 1 && now - last >= NS_PER_SEC / anim.fps.max(1) as u64 {
            fi = (fi + 1) % anim.frames.len();
            last = now;
        }

        buf.clear();
        buf.push_str("\x1b[H\x1b[2J");
        let _ = write!(
            buf,
            "\x1b[2;4H\x1b[1mSPRITE LAB\x1b[0m  ::  {} / {}   ({} frames @ {} fps)   anim {}/{}",
            set.name,
            anim.name,
            anim.frames.len(),
            anim.fps,
            ai + 1,
            set.anims.len()
        );
        for (i, line) in anim.frames[fi].iter().enumerate() {
            let _ = write!(buf, "\x1b[{};6H{}", 5 + i, line);
        }
        let _ = write!(
            buf,
            "\x1b[13;4Hspace / Tab: next anim     s: next sprite     q: quit"
        );
        {
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(buf.as_bytes());
            let _ = o.flush();
        }
        sleep_until_ns(now_ns() + NS_PER_SEC / 60, 1_000_000);
    }
    drop(guard);
    eprintln!("sprite-lab: bye.");
}
