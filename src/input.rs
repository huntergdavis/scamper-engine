//! Keyboard input: a virtual controller fed by the Kitty keyboard protocol
//! (press/repeat/release) with a legacy raw-byte fallback (PROJECT_PLAN.md §4.5).
//!
//! The game reads abstract actions (`left/right/up/down/jump`), never raw keys.
//! Kitty mode gives true key-release events (needed for variable-height jumps);
//! legacy mode approximates "held" via terminal autorepeat + an auto-release timeout.

use std::collections::{HashMap, HashSet};

// Normalized key codes. Letters/space use their unicode codepoint; arrows are synthetic.
pub const K_SPACE: u32 = 32;
pub const K_A: u32 = 97;
pub const K_D: u32 = 100;
pub const K_W: u32 = 119;
pub const K_S: u32 = 115;
pub const K_Z: u32 = 122;
pub const K_K: u32 = 107;
pub const K_Q: u32 = 113;
pub const K_LEFT: u32 = 1_000;
pub const K_DOWN: u32 = 1_001;
pub const K_UP: u32 = 1_002;
pub const K_RIGHT: u32 = 1_003;
pub const K_ESC: u32 = 27;

#[derive(Clone, Copy, PartialEq)]
enum Ev {
    Press,
    Repeat,
    Release,
}

pub struct Input {
    kitty: bool,
    down: HashSet<u32>,
    pressed: HashSet<u32>,  // press edges this frame
    released: HashSet<u32>, // release edges this frame
    legacy_hold: HashMap<u32, u32>, // code -> frames until auto-release (legacy mode)
    pending: Vec<u8>,
    pub quit: bool,
    pub focused: bool,
}

impl Input {
    pub fn new(kitty: bool) -> Self {
        Input {
            kitty,
            down: HashSet::new(),
            pressed: HashSet::new(),
            released: HashSet::new(),
            legacy_hold: HashMap::new(),
            pending: Vec::new(),
            quit: false,
            focused: true,
        }
    }

    /// Drain all available stdin bytes (non-blocking) and update state. Call once per frame.
    pub fn poll(&mut self) {
        self.pressed.clear();
        self.released.clear();

        let mut buf = [0u8; 1024];
        loop {
            let n = unsafe {
                libc::read(
                    libc::STDIN_FILENO,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n > 0 {
                self.pending.extend_from_slice(&buf[..n as usize]);
                if (n as usize) < buf.len() {
                    break;
                }
            } else {
                break;
            }
        }
        self.parse();

        // Legacy auto-release: decay holds; refreshed by repeated bytes in parse().
        if !self.kitty {
            let mut expired = Vec::new();
            for (k, t) in self.legacy_hold.iter_mut() {
                if *t == 0 {
                    expired.push(*k);
                } else {
                    *t -= 1;
                }
            }
            for k in expired {
                self.legacy_hold.remove(&k);
                if self.down.remove(&k) {
                    self.released.insert(k);
                }
            }
        }
    }

    fn emit(&mut self, code: u32, ev: Ev) {
        match ev {
            Ev::Press => {
                if self.down.insert(code) {
                    self.pressed.insert(code);
                }
            }
            Ev::Repeat => {
                self.down.insert(code);
            }
            Ev::Release => {
                if self.down.remove(&code) {
                    self.released.insert(code);
                }
            }
        }
        if code == K_Q || code == K_ESC {
            if ev != Ev::Release {
                self.quit = true;
            }
        }
    }

    fn legacy_byte(&mut self, code: u32) {
        // Treat each raw byte as press/refresh with a ~7-frame auto-release window.
        const HOLD_FRAMES: u32 = 7;
        let was = self.down.contains(&code);
        self.legacy_hold.insert(code, HOLD_FRAMES);
        if !was {
            self.emit(code, Ev::Press);
        }
    }

    fn parse(&mut self) {
        let data = std::mem::take(&mut self.pending);
        let mut i = 0;
        let n = data.len();
        while i < n {
            let b = data[i];
            if b == 0x1b {
                // ESC: could be a CSI/SS3 sequence, an OSC/APC reply, or a lone Esc.
                if i + 1 >= n {
                    // Incomplete: a lone trailing ESC. Stash it and decide next frame
                    // (could be the Escape key, or the start of a split sequence).
                    self.pending.push(0x1b);
                    break;
                }
                match data[i + 1] {
                    b'[' | b'O' => {
                        if let Some(len) = self.parse_csi(&data[i..]) {
                            i += len;
                        } else {
                            // incomplete CSI: stash remainder
                            self.pending.extend_from_slice(&data[i..]);
                            break;
                        }
                    }
                    b']' | b'_' | b'P' => {
                        // OSC / APC (e.g. Kitty `_G` graphics replies) / DCS: discard to ST or BEL.
                        if let Some(len) = skip_string_terminated(&data[i..]) {
                            i += len;
                        } else {
                            self.pending.extend_from_slice(&data[i..]);
                            break;
                        }
                    }
                    _ => {
                        // Lone ESC → treat as Escape key.
                        self.emit(K_ESC, Ev::Press);
                        i += 1;
                    }
                }
            } else if b == 0x03 {
                self.quit = true; // Ctrl-C (raw mode delivers it as a byte)
                i += 1;
            } else if self.kitty {
                // In kitty mode with "report all keys as escape codes", plain bytes are rare;
                // ignore stray control bytes, map newline-ish nothing.
                i += 1;
            } else {
                // Legacy mode: raw byte keys.
                let code = b as u32;
                match code {
                    K_A | K_D | K_W | K_S | K_SPACE | K_Z | K_K => self.legacy_byte(code),
                    K_Q => self.quit = true,
                    _ => {}
                }
                i += 1;
            }
        }
    }

    /// Parse a CSI/SS3 sequence at `s[0..]` (starting with ESC). Returns consumed length,
    /// or None if incomplete.
    fn parse_csi(&mut self, s: &[u8]) -> Option<usize> {
        // s[0]=ESC, s[1]='[' or 'O'
        let mut j = 2;
        // Collect parameter/intermediate bytes until a final byte 0x40..=0x7e.
        while j < s.len() {
            let c = s[j];
            if (0x40..=0x7e).contains(&c) {
                // final byte at j
                let params = &s[2..j];
                self.handle_csi(params, c);
                return Some(j + 1);
            }
            j += 1;
        }
        None // incomplete
    }

    fn handle_csi(&mut self, params: &[u8], final_byte: u8) {
        // params like "97;1:3" → fields by ';', subfields by ':'.
        // For Kitty `u`: field0 = keycode (first subfield), field1 = modifiers:event.
        // For arrows (A/B/C/D): may carry "1;mods:event".
        let text = std::str::from_utf8(params).unwrap_or("");
        let fields: Vec<&str> = text.split(';').collect();

        let first_sub = |f: &str| -> u32 {
            f.split(':').next().and_then(|x| x.parse().ok()).unwrap_or(0)
        };

        // event type from field1's 2nd subfield (default press)
        let ev = {
            let mut e = Ev::Press;
            if fields.len() >= 2 {
                if let Some(evs) = fields[1].split(':').nth(1) {
                    match evs.parse::<u32>().unwrap_or(1) {
                        2 => e = Ev::Repeat,
                        3 => e = Ev::Release,
                        _ => e = Ev::Press,
                    }
                }
            }
            e
        };

        let code = match final_byte {
            b'u' => {
                let kc = if fields.is_empty() { 0 } else { first_sub(fields[0]) };
                // map uppercase letters to lowercase code
                match kc {
                    65..=90 => kc + 32, // A-Z -> a-z
                    _ => kc,
                }
            }
            b'A' => K_UP,
            b'B' => K_DOWN,
            b'C' => K_RIGHT,
            b'D' => K_LEFT,
            _ => return, // other finals (e.g. 'c','R','~') ignored
        };
        if code != 0 {
            self.emit(code, ev);
        }
    }

    // ---- Action queries ----
    pub fn held(&self, code: u32) -> bool {
        self.down.contains(&code)
    }
    pub fn pressed(&self, code: u32) -> bool {
        self.pressed.contains(&code)
    }

    /// Horizontal input axis in [-1, 1].
    pub fn axis_x(&self) -> f64 {
        let l = self.held(K_A) || self.held(K_LEFT);
        let r = self.held(K_D) || self.held(K_RIGHT);
        (r as i32 - l as i32) as f64
    }
    pub fn down_held(&self) -> bool {
        self.held(K_S) || self.held(K_DOWN)
    }
    pub fn jump_held(&self) -> bool {
        self.held(K_SPACE) || self.held(K_Z) || self.held(K_K) || self.held(K_W) || self.held(K_UP)
    }
    pub fn jump_pressed(&self) -> bool {
        self.pressed(K_SPACE)
            || self.pressed(K_Z)
            || self.pressed(K_K)
            || self.pressed(K_W)
            || self.pressed(K_UP)
    }
}

/// Skip an OSC/APC/DCS string: from ESC to the String Terminator (ESC \) or BEL.
fn skip_string_terminated(s: &[u8]) -> Option<usize> {
    let mut j = 2;
    while j < s.len() {
        if s[j] == 0x07 {
            return Some(j + 1); // BEL
        }
        if s[j] == 0x1b && j + 1 < s.len() && s[j + 1] == b'\\' {
            return Some(j + 2); // ST
        }
        if s[j] == 0x1b && j + 1 >= s.len() {
            return None; // maybe ST split across reads
        }
        j += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(inp: &mut Input, bytes: &[u8]) {
        inp.pressed.clear();
        inp.released.clear();
        inp.pending.extend_from_slice(bytes);
        inp.parse();
    }

    #[test]
    fn kitty_press_release() {
        let mut inp = Input::new(true);
        feed(&mut inp, b"\x1b[100u"); // 'd' press
        assert!(inp.held(K_D));
        assert!(inp.pressed(K_D));
        assert!((inp.axis_x() - 1.0).abs() < 1e-9);
        feed(&mut inp, b"\x1b[100;1:3u"); // 'd' release
        assert!(!inp.held(K_D));
        assert_eq!(inp.axis_x(), 0.0);
    }

    #[test]
    fn kitty_space_jump_and_release() {
        let mut inp = Input::new(true);
        feed(&mut inp, b"\x1b[32u");
        assert!(inp.jump_pressed() && inp.jump_held());
        feed(&mut inp, b"\x1b[32;1:3u");
        assert!(!inp.jump_held());
    }

    #[test]
    fn arrows() {
        let mut inp = Input::new(true);
        feed(&mut inp, b"\x1b[C"); // right arrow (legacy form)
        assert!(inp.held(K_RIGHT));
        feed(&mut inp, b"\x1b[1;1:3C"); // right release
        assert!(!inp.held(K_RIGHT));
    }

    #[test]
    fn discards_graphics_reply() {
        let mut inp = Input::new(true);
        // a kitty graphics OK reply should be swallowed, not parsed as keys
        feed(&mut inp, b"\x1b_Gi=1;OK\x1b\\\x1b[100u");
        assert!(inp.held(K_D));
        assert!(!inp.quit);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut inp = Input::new(true);
        feed(&mut inp, &[0x03]);
        assert!(inp.quit);
    }
}
