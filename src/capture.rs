//! Input capture + deterministic replay storage (RECORD_REPLAY.md).
//!
//! A [`Recording`] is everything needed to reproduce a playthrough tick-for-tick:
//! the arena's originating window size, a seed (reserved; the sim has no RNG yet),
//! and one [`InputFrame`] per sim tick — the exact arguments handed to
//! `Player::step`. Replaying those frames against the same arena through the same
//! tick-driven sim reproduces the run exactly (see [`crate::sim`]).
//!
//! On-disk formats are line-oriented text (diff-friendly, greppable, CI-readable):
//!   - captures:  `<dir>/<name>.scap`
//!   - snapshots: `<dir>/<name>.snap`  (golden `mono_text` keyframes)
//! `<dir>` defaults to `$XDG_STATE_HOME/scamper/captures` (`~/.local/state/...`).

use crate::terminal::WinSize;
use std::io;
use std::path::{Path, PathBuf};

pub const CAPTURE_EXT: &str = "scap";
pub const SNAPSHOT_EXT: &str = "snap";

/// One tick's worth of input — exactly what `Player::step` consumes. `axis_x` is
/// -1/0/1 (left/none/right); the three flags mirror the loop's per-substep state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct InputFrame {
    pub axis_x: i8,
    pub jump_pressed: bool,
    pub jump_held: bool,
    pub down_held: bool,
}

/// A full recorded run: header + per-tick inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct Recording {
    pub name: String,
    pub seed: u64,
    pub win: WinSize,
    pub frames: Vec<InputFrame>,
}

impl Recording {
    pub fn new(name: impl Into<String>, win: WinSize) -> Self {
        Recording { name: name.into(), seed: 0, win, frames: Vec::new() }
    }

    /// Serialize to the on-disk text form.
    pub fn to_text(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        let _ = writeln!(s, "scamper-capture v1");
        let _ = writeln!(s, "name {}", sanitize_header(&self.name));
        let _ = writeln!(s, "seed {}", self.seed);
        let _ = writeln!(s, "win {} {} {} {}", self.win.cols, self.win.rows, self.win.xpix, self.win.ypix);
        let _ = writeln!(s, "frames {}", self.frames.len());
        // body: one line per tick — `axis jump_pressed jump_held down_held`.
        for f in &self.frames {
            let _ = writeln!(s, "{} {} {} {}", f.axis_x, f.jump_pressed as u8, f.jump_held as u8, f.down_held as u8);
        }
        s
    }

    /// Parse the on-disk text form.
    pub fn from_text(text: &str) -> io::Result<Recording> {
        let mut lines = text.lines();
        let magic = lines.next().unwrap_or("");
        if magic.trim() != "scamper-capture v1" {
            return Err(bad(format!("not a scamper capture (got {magic:?})")));
        }
        let mut name = String::new();
        let mut seed = 0u64;
        let mut win = WinSize { cols: 0, rows: 0, xpix: 0, ypix: 0 };
        let mut declared = 0usize;
        // header lines until `frames N`
        for line in lines.by_ref() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "name" => name = rest.to_string(),
                "seed" => seed = rest.trim().parse().map_err(|_| bad("bad seed"))?,
                "win" => {
                    let v = parse_u16s(rest)?;
                    if v.len() != 4 {
                        return Err(bad("win needs 4 ints"));
                    }
                    win = WinSize { cols: v[0], rows: v[1], xpix: v[2], ypix: v[3] };
                }
                "frames" => {
                    declared = rest.trim().parse().map_err(|_| bad("bad frame count"))?;
                    break;
                }
                _ => return Err(bad(format!("unknown header key {key:?}"))),
            }
        }
        let mut frames = Vec::with_capacity(declared);
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let p: Vec<&str> = line.split_whitespace().collect();
            if p.len() != 4 {
                return Err(bad(format!("frame line needs 4 fields: {line:?}")));
            }
            frames.push(InputFrame {
                axis_x: p[0].parse().map_err(|_| bad("bad axis"))?,
                jump_pressed: parse_bit(p[1])?,
                jump_held: parse_bit(p[2])?,
                down_held: parse_bit(p[3])?,
            });
        }
        if declared != frames.len() {
            return Err(bad(format!("declared {declared} frames, found {}", frames.len())));
        }
        Ok(Recording { name, seed, win, frames })
    }
}

/// A set of golden keyframes: `mono_text` renders captured at specific ticks.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshots {
    pub name: String,
    pub keys: Vec<(u64, String)>, // (tick, mono_text block)
}

impl Snapshots {
    pub fn new(name: impl Into<String>) -> Self {
        Snapshots { name: name.into(), keys: Vec::new() }
    }

    pub fn to_text(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        let _ = writeln!(s, "scamper-snapshots v1");
        let _ = writeln!(s, "name {}", sanitize_header(&self.name));
        for (tick, block) in &self.keys {
            // Each block is preceded by an explicit line count so content lines
            // can contain any glyph (including a leading '@') without ambiguity.
            let lines: Vec<&str> = block.lines().collect();
            let _ = writeln!(s, "@tick {} {}", tick, lines.len());
            for l in lines {
                let _ = writeln!(s, "{l}");
            }
        }
        s
    }

    pub fn from_text(text: &str) -> io::Result<Snapshots> {
        let mut lines = text.lines();
        if lines.next().unwrap_or("").trim() != "scamper-snapshots v1" {
            return Err(bad("not a scamper snapshot file"));
        }
        let mut name = String::new();
        let mut keys = Vec::new();
        while let Some(line) = lines.next() {
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("name ") {
                name = rest.to_string();
            } else if let Some(rest) = line.strip_prefix("@tick ") {
                let mut it = rest.split_whitespace();
                let tick: u64 = it.next().and_then(|x| x.parse().ok()).ok_or_else(|| bad("bad @tick"))?;
                let n: usize = it.next().and_then(|x| x.parse().ok()).ok_or_else(|| bad("bad @tick count"))?;
                let mut block = String::new();
                for i in 0..n {
                    let l = lines.next().ok_or_else(|| bad("snapshot block truncated"))?;
                    if i > 0 {
                        block.push('\n');
                    }
                    block.push_str(l);
                }
                keys.push((tick, block));
            } else {
                return Err(bad(format!("unexpected snapshot line {line:?}")));
            }
        }
        Ok(Snapshots { name, keys })
    }

    /// Compare against freshly-rendered keyframes. Returns the ticks (and a short
    /// human diff) that differ, or empty if everything matches.
    pub fn diff(&self, fresh: &[(u64, String)]) -> Vec<String> {
        let mut out = Vec::new();
        if self.keys.len() != fresh.len() {
            out.push(format!("keyframe count differs: golden {}, replay {}", self.keys.len(), fresh.len()));
        }
        for ((gt, gb), (ft, fb)) in self.keys.iter().zip(fresh.iter()) {
            if gt != ft {
                out.push(format!("keyframe tick mismatch: golden {gt}, replay {ft}"));
            } else if gb != fb {
                out.push(format!("tick {gt}: snapshot differs\n{}", first_line_diff(gb, fb)));
            }
        }
        out
    }
}

// ---- filesystem helpers ------------------------------------------------------

/// The captures directory: `$XDG_STATE_HOME/scamper/captures`, falling back to
/// `~/.local/state/scamper/captures`. Honors `SCAMP_CAPTURE_DIR` as an override
/// (used by tests / fixtures).
pub fn captures_dir() -> PathBuf {
    if let Ok(d) = std::env::var("SCAMP_CAPTURE_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            Path::new(&home).join(".local").join("state")
        });
    base.join("scamper").join("captures")
}

pub fn capture_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{CAPTURE_EXT}"))
}
pub fn snapshot_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{SNAPSHOT_EXT}"))
}

/// Capture names present in `dir` (sorted), i.e. files ending in `.scap`.
pub fn list_captures(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some(CAPTURE_EXT) {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

pub fn save_recording(dir: &Path, rec: &Recording) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = capture_path(dir, &rec.name);
    std::fs::write(&path, rec.to_text())?;
    Ok(path)
}
pub fn load_recording(dir: &Path, name: &str) -> io::Result<Recording> {
    Recording::from_text(&std::fs::read_to_string(capture_path(dir, name))?)
}
pub fn save_snapshots(dir: &Path, snaps: &Snapshots) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = snapshot_path(dir, &snaps.name);
    std::fs::write(&path, snaps.to_text())?;
    Ok(path)
}
pub fn load_snapshots(dir: &Path, name: &str) -> io::Result<Snapshots> {
    Snapshots::from_text(&std::fs::read_to_string(snapshot_path(dir, name))?)
}

/// A capture name is safe if it's a single filename component. Spaces (and most
/// punctuation) are allowed; path separators, control characters, leading/trailing
/// whitespace, the dot-names, and over-long names are rejected so a name can't
/// traverse directories or produce a surprising file.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name == name.trim()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.chars().any(|c| c.is_control())
}

// ---- internals ---------------------------------------------------------------

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}
fn parse_bit(s: &str) -> io::Result<bool> {
    match s {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(bad(format!("expected 0/1, got {s:?}"))),
    }
}
fn parse_u16s(s: &str) -> io::Result<Vec<u16>> {
    s.split_whitespace()
        .map(|x| x.parse::<u16>().map_err(|_| bad(format!("bad int {x:?}"))))
        .collect()
}
/// Header values are single-line; strip newlines defensively.
fn sanitize_header(s: &str) -> String {
    s.replace(['\n', '\r'], "")
}
/// First differing line between two blocks, for a compact failure message.
fn first_line_diff(a: &str, b: &str) -> String {
    for (i, (la, lb)) in a.lines().zip(b.lines()).enumerate() {
        if la != lb {
            return format!("  line {i}:\n    golden: {la:?}\n    replay: {lb:?}");
        }
    }
    format!("  (line count differs: golden {}, replay {})", a.lines().count(), b.lines().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rec() -> Recording {
        let mut r = Recording::new("demo", WinSize { cols: 80, rows: 24, xpix: 800, ypix: 480 });
        r.frames.push(InputFrame { axis_x: 1, jump_pressed: true, jump_held: true, down_held: false });
        r.frames.push(InputFrame { axis_x: 0, jump_pressed: false, jump_held: true, down_held: false });
        r.frames.push(InputFrame { axis_x: -1, jump_pressed: false, jump_held: false, down_held: true });
        r
    }

    #[test]
    fn recording_roundtrips() {
        let r = sample_rec();
        let back = Recording::from_text(&r.to_text()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn rejects_foreign_file() {
        assert!(Recording::from_text("hello\n").is_err());
    }

    #[test]
    fn frame_count_must_match() {
        let mut t = sample_rec().to_text();
        t = t.replace("frames 3", "frames 9");
        assert!(Recording::from_text(&t).is_err());
    }

    #[test]
    fn snapshots_roundtrip_with_at_in_content() {
        let mut s = Snapshots::new("demo");
        // a content line that begins with '@' must survive (Munchii's nose is '@').
        s.keys.push((0, "@==o line one\nsecond .. line".to_string()));
        s.keys.push((30, "....\n.##.".to_string()));
        let back = Snapshots::from_text(&s.to_text()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn snapshot_diff_flags_mismatch() {
        let mut g = Snapshots::new("demo");
        g.keys.push((0, "abc\ndef".into()));
        let same = vec![(0u64, "abc\ndef".to_string())];
        assert!(g.diff(&same).is_empty());
        let diff = vec![(0u64, "abc\nXef".to_string())];
        assert!(!g.diff(&diff).is_empty());
    }

    #[test]
    fn name_validation() {
        assert!(valid_name("run-1.demo_2"));
        assert!(valid_name("first run record")); // spaces are fine now
        assert!(valid_name("Boss fight (take 2)!"));
        assert!(!valid_name(""));
        assert!(!valid_name("."));
        assert!(!valid_name(".."));
        assert!(!valid_name("a/b"));
        assert!(!valid_name("back\\slash"));
        assert!(!valid_name(" leading"));
        assert!(!valid_name("trailing "));
        assert!(!valid_name("new\nline"));
    }
}
