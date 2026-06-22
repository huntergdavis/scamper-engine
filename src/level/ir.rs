//! The engine-native level format (CAMPAIGN_PLAN.md §4a).
//!
//! A [`Level`] is everything needed to place tiles, entities, the spawn, and the
//! goal on a fixed 16px tile grid. It is the ONLY level representation the runtime
//! knows about — the Godot `.tscn` importer ([`super::import`]) is an offline tool
//! that emits this and nothing else.
//!
//! On disk it's line-oriented text (`*.lvl`), like the capture files: a header,
//! then tile spans, then entities. Readable, greppable, diff-friendly, and
//! hand-authorable without a serializer dependency.

use std::collections::HashSet;
use std::io;

/// What a tile *is*. Collision/behavior semantics live in the runtime; the IR
/// just records the kind. `Ground` is the catch-all solid block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileKind {
    Ground,
    Brick,
    CoinBrick,
    Question,
    Hidden, // invisible block, materializes when bonked
    Pipe,
    Platform, // one-way / semisolid
    Hazard,   // hurts on contact but doesn't block
    Deco,     // decorative, non-solid
}

impl TileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TileKind::Ground => "ground",
            TileKind::Brick => "brick",
            TileKind::CoinBrick => "coinbrick",
            TileKind::Question => "question",
            TileKind::Hidden => "hidden",
            TileKind::Pipe => "pipe",
            TileKind::Platform => "platform",
            TileKind::Hazard => "hazard",
            TileKind::Deco => "deco",
        }
    }

    pub fn from_str(s: &str) -> Option<TileKind> {
        Some(match s {
            "ground" => TileKind::Ground,
            "brick" => TileKind::Brick,
            "coinbrick" => TileKind::CoinBrick,
            "question" => TileKind::Question,
            "hidden" => TileKind::Hidden,
            "pipe" => TileKind::Pipe,
            "platform" => TileKind::Platform,
            "hazard" => TileKind::Hazard,
            "deco" => TileKind::Deco,
            _ => return None,
        })
    }

    /// Baseline collision for preview/geometry checks. `Hidden` is non-solid until
    /// bonked; `Hazard`/`Deco` never block; `Platform` is treated solid here.
    pub fn is_solid(self) -> bool {
        matches!(
            self,
            TileKind::Ground | TileKind::Brick | TileKind::CoinBrick | TileKind::Question | TileKind::Pipe | TileKind::Platform
        )
    }

    /// One-character map glyph for the ascii preview.
    pub fn glyph(self) -> char {
        match self {
            TileKind::Ground => '#',
            TileKind::Brick => 'b',
            TileKind::CoinBrick => 'c',
            TileKind::Question => '?',
            TileKind::Hidden => 'h',
            TileKind::Pipe => '|',
            TileKind::Platform => '=',
            TileKind::Hazard => '^',
            TileKind::Deco => '.',
        }
    }
}

/// A horizontal run of identical tiles starting at (`x`,`y`), `len` wide. A single
/// tile is just `len = 1`. Runs keep big levels (long ground spans) compact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileSpan {
    pub x: i32,
    pub y: i32,
    pub len: i32,
    pub kind: TileKind,
}

/// An instanced thing at a tile position, with free-form `key=value` props
/// (e.g. a question block's `contains`, a pipe's `warp` target).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub kind: String,
    pub x: i32,
    pub y: i32,
    pub props: Vec<(String, String)>,
}

impl Entity {
    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// The level-exit marker (flagpole, castle door, …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Goal {
    pub kind: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    pub id: String,
    pub theme: String,
    pub w: i32,
    pub h: i32,
    pub spawn: (i32, i32),
    pub goal: Option<Goal>,
    pub tiles: Vec<TileSpan>,
    pub entities: Vec<Entity>,
    pub checkpoints: Vec<(i32, i32)>,
}

impl Level {
    pub fn new(id: impl Into<String>, theme: impl Into<String>, w: i32, h: i32) -> Self {
        Level {
            id: id.into(),
            theme: theme.into(),
            w,
            h,
            spawn: (0, 0),
            goal: None,
            tiles: Vec::new(),
            entities: Vec::new(),
            checkpoints: Vec::new(),
        }
    }

    /// The set of solid tile cells, expanding runs. For collision/preview/tests.
    pub fn solid_cells(&self) -> HashSet<(i32, i32)> {
        let mut s = HashSet::new();
        for t in &self.tiles {
            if t.kind.is_solid() {
                for i in 0..t.len.max(1) {
                    s.insert((t.x + i, t.y));
                }
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        let _ = writeln!(s, "scamper-level v1");
        let _ = writeln!(s, "id {}", one_line(&self.id));
        let _ = writeln!(s, "theme {}", one_line(&self.theme));
        let _ = writeln!(s, "size {} {}", self.w, self.h);
        let _ = writeln!(s, "spawn {} {}", self.spawn.0, self.spawn.1);
        if let Some(g) = &self.goal {
            let _ = writeln!(s, "goal {} {} {}", one_line(&g.kind), g.x, g.y);
        }
        for t in &self.tiles {
            if t.len == 1 {
                let _ = writeln!(s, "T {} {} {}", t.x, t.y, t.kind.as_str());
            } else {
                let _ = writeln!(s, "R {} {} {} {}", t.x, t.y, t.len, t.kind.as_str());
            }
        }
        for e in &self.entities {
            let _ = write!(s, "E {} {} {}", one_line(&e.kind), e.x, e.y);
            for (k, v) in &e.props {
                let _ = write!(s, " {}={}", one_line(k), one_line(v));
            }
            let _ = writeln!(s);
        }
        for (x, y) in &self.checkpoints {
            let _ = writeln!(s, "C {x} {y}");
        }
        s
    }

    pub fn from_text(text: &str) -> io::Result<Level> {
        let mut lines = text.lines();
        let magic = loop {
            match lines.next() {
                Some(l) if l.trim().is_empty() => continue,
                Some(l) => break l.trim(),
                None => return Err(bad("empty level file")),
            }
        };
        if magic != "scamper-level v1" {
            return Err(bad(format!("not a scamper level (got {magic:?})")));
        }

        let mut lvl = Level::new(String::new(), "overworld", 0, 0);
        for raw in lines {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.split_whitespace();
            let tag = it.next().unwrap_or("");
            match tag {
                "id" => lvl.id = rest_after(line, "id"),
                "theme" => lvl.theme = rest_after(line, "theme"),
                "size" => {
                    lvl.w = next_i32(&mut it, "size w")?;
                    lvl.h = next_i32(&mut it, "size h")?;
                }
                "spawn" => {
                    lvl.spawn = (next_i32(&mut it, "spawn x")?, next_i32(&mut it, "spawn y")?);
                }
                "goal" => {
                    let kind = it.next().ok_or_else(|| bad("goal needs a kind"))?.to_string();
                    lvl.goal = Some(Goal {
                        kind,
                        x: next_i32(&mut it, "goal x")?,
                        y: next_i32(&mut it, "goal y")?,
                    });
                }
                "T" => {
                    let x = next_i32(&mut it, "T x")?;
                    let y = next_i32(&mut it, "T y")?;
                    let kind = parse_kind(it.next())?;
                    lvl.tiles.push(TileSpan { x, y, len: 1, kind });
                }
                "R" => {
                    let x = next_i32(&mut it, "R x")?;
                    let y = next_i32(&mut it, "R y")?;
                    let len = next_i32(&mut it, "R len")?;
                    let kind = parse_kind(it.next())?;
                    lvl.tiles.push(TileSpan { x, y, len, kind });
                }
                "E" => {
                    let kind = it.next().ok_or_else(|| bad("E needs a type"))?.to_string();
                    let x = next_i32(&mut it, "E x")?;
                    let y = next_i32(&mut it, "E y")?;
                    let mut props = Vec::new();
                    for kv in it {
                        if let Some((k, v)) = kv.split_once('=') {
                            props.push((k.to_string(), v.to_string()));
                        } else {
                            return Err(bad(format!("entity prop {kv:?} is not key=value")));
                        }
                    }
                    lvl.entities.push(Entity { kind, x, y, props });
                }
                "C" => {
                    lvl.checkpoints.push((next_i32(&mut it, "C x")?, next_i32(&mut it, "C y")?));
                }
                other => return Err(bad(format!("unknown line tag {other:?}"))),
            }
        }
        Ok(lvl)
    }

    /// An ascii map of the level (solid/kind glyphs + entities/spawn/goal). Rows
    /// top-to-bottom; great for eyeballing an import. Clamped to `[w]`×`[h]`.
    pub fn ascii_preview(&self) -> String {
        let w = self.w.max(0) as usize;
        let h = self.h.max(0) as usize;
        if w == 0 || h == 0 {
            return String::new();
        }
        let mut grid = vec![vec![' '; w]; h];
        let put = |grid: &mut Vec<Vec<char>>, x: i32, y: i32, c: char| {
            if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                grid[y as usize][x as usize] = c;
            }
        };
        for t in &self.tiles {
            for i in 0..t.len.max(1) {
                put(&mut grid, t.x + i, t.y, t.kind.glyph());
            }
        }
        for e in &self.entities {
            // first letter of the entity type, uppercased (so it pops over tiles)
            let c = e.kind.chars().next().unwrap_or('e').to_ascii_uppercase();
            put(&mut grid, e.x, e.y, c);
        }
        for (x, y) in &self.checkpoints {
            put(&mut grid, *x, *y, '!');
        }
        if let Some(g) = &self.goal {
            put(&mut grid, g.x, g.y, 'G');
        }
        put(&mut grid, self.spawn.0, self.spawn.1, 'S');
        let mut s = String::with_capacity((w + 1) * h);
        for row in grid {
            s.extend(row);
            s.push('\n');
        }
        s
    }
}

// ---- helpers ----------------------------------------------------------------

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}
fn parse_kind(tok: Option<&str>) -> io::Result<TileKind> {
    let t = tok.ok_or_else(|| bad("missing tile kind"))?;
    TileKind::from_str(t).ok_or_else(|| bad(format!("unknown tile kind {t:?}")))
}
fn next_i32<'a>(it: &mut impl Iterator<Item = &'a str>, what: &str) -> io::Result<i32> {
    it.next()
        .ok_or_else(|| bad(format!("missing {what}")))?
        .parse()
        .map_err(|_| bad(format!("bad integer for {what}")))
}
/// Everything after the first token `tag ` on a line (so values may contain spaces).
fn rest_after(line: &str, tag: &str) -> String {
    line.strip_prefix(tag).map(|r| r.trim().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Level {
        let mut l = Level::new("yard-romp-1", "overworld", 48, 15);
        l.spawn = (2, 11);
        l.goal = Some(Goal { kind: "flag".into(), x: 46, y: 3 });
        l.tiles.push(TileSpan { x: 0, y: 13, len: 48, kind: TileKind::Ground });
        l.tiles.push(TileSpan { x: 8, y: 9, len: 1, kind: TileKind::Question });
        l.tiles.push(TileSpan { x: 12, y: 9, len: 3, kind: TileKind::Brick });
        l.entities.push(Entity {
            kind: "boneling".into(),
            x: 22,
            y: 12,
            props: vec![],
        });
        l.entities.push(Entity {
            kind: "pipe".into(),
            x: 28,
            y: 11,
            props: vec![("warp".into(), "yard-romp-1a@3,12".into())],
        });
        l.checkpoints.push((24, 11));
        l
    }

    #[test]
    fn level_roundtrips() {
        let l = sample();
        let back = Level::from_text(&l.to_text()).unwrap();
        assert_eq!(l, back);
    }

    #[test]
    fn rejects_foreign_file() {
        assert!(Level::from_text("nope\n").is_err());
    }

    #[test]
    fn run_expands_to_solid_cells() {
        let l = sample();
        let solids = l.solid_cells();
        assert!(solids.contains(&(0, 13)) && solids.contains(&(47, 13)));
        assert!(solids.contains(&(8, 9))); // question is solid
        assert!(!solids.contains(&(0, 0)));
    }

    #[test]
    fn tile_kind_str_roundtrip() {
        for k in [
            TileKind::Ground,
            TileKind::Brick,
            TileKind::CoinBrick,
            TileKind::Question,
            TileKind::Hidden,
            TileKind::Pipe,
            TileKind::Platform,
            TileKind::Hazard,
            TileKind::Deco,
        ] {
            assert_eq!(TileKind::from_str(k.as_str()), Some(k));
        }
        assert_eq!(TileKind::from_str("bogus"), None);
    }

    #[test]
    fn preview_marks_spawn_and_goal() {
        let p = sample().ascii_preview();
        assert!(p.contains('S'), "spawn glyph present");
        assert!(p.contains('G'), "goal glyph present");
        assert!(p.contains('#'), "ground glyph present");
        assert_eq!(p.lines().count(), 15, "one row per tile height");
    }

    #[test]
    fn comments_and_blanks_ignored() {
        let txt = "scamper-level v1\n# hi\n\nid x\nsize 4 4\nspawn 0 0\nR 0 3 4 ground\n";
        let l = Level::from_text(txt).unwrap();
        assert_eq!(l.id, "x");
        assert_eq!(l.tiles.len(), 1);
    }
}
