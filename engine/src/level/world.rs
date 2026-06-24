//! The playable world built from a [`Level`] (CAMPAIGN_PLAN.md §6, §10).
//!
//! Projects the IR's typed tiles + solid block-entities into a boolean-solid
//! [`TileMap`] so the existing [`crate::player::Player`] physics run unchanged,
//! and keeps the gameplay-relevant extras alongside: hazard cells (lava / deep
//! water → respawn), the goal position (level end), warps (pipes), spawn, and the
//! visual theme. Also a clamped side-scroll [`camera`].
//!
//! v1 simplification: one-way `platform` tiles are treated as fully solid (the
//! shared physics has no one-way support yet) — noted in CAMPAIGN_PLAN.md.

use crate::level::art::Theme;
use crate::level::ir::{Level, TileKind};
use crate::world::{TileMap, TILE};
use std::collections::{HashMap, HashSet};

/// A pipe/warp trigger and where it leads. `target` is `"<level-id>@tx,ty"` or
/// `"@tx,ty"` (same level); `None` means decorative (e.g. imported pipes with no
/// recorded destination).
#[derive(Clone, Debug)]
pub struct Warp {
    pub cx: i32,
    pub cy: i32,
    pub target: Option<String>,
}

/// A non-tile entity placed in the world (creature / item / power-up), at a tile
/// cell. Rendered from the sprite registry by id; behavior (walk/turn, collect,
/// pounce) comes in a later milestone.
#[derive(Clone, Debug)]
pub struct Ent {
    pub kind: String,
    pub cx: i32,
    pub cy: i32,
}

pub struct LevelWorld {
    pub map: TileMap,                // solid cells, drives Player::step
    pub hazard: HashSet<(i32, i32)>, // cells that kill on contact
    pub kinds: HashMap<(i32, i32), TileKind>, // per-cell kind, for rendering
    pub ents: Vec<Ent>,              // creatures / items, drawn from the sprite registry
    pub w: i32,
    pub h: i32,
    pub spawn: (f64, f64), // px (top-left of the player box)
    pub goal: Option<(f64, f64)>,
    pub warps: Vec<Warp>,
    pub theme: Theme,
}

impl LevelWorld {
    pub fn from_level(lvl: &Level) -> Self {
        let w = lvl.w.max(1);
        let h = lvl.h.max(1);
        let mut map = TileMap::new(w as usize, h as usize);
        let mut hazard = HashSet::new();
        let mut kinds = HashMap::new();
        let in_bounds = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h;

        for span in &lvl.tiles {
            for i in 0..span.len.max(1) {
                let (x, y) = (span.x + i, span.y);
                if !in_bounds(x, y) {
                    continue;
                }
                if span.kind.is_solid() {
                    map.set(x as usize, y as usize, true);
                }
                if span.kind == TileKind::Hazard {
                    hazard.insert((x, y));
                }
                kinds.insert((x, y), span.kind);
            }
        }

        // Interactive blocks are IR entities, not tiles, but they're solid.
        let mut warps = Vec::new();
        let mut ents = Vec::new();
        for e in &lvl.entities {
            match e.kind.as_str() {
                "question" | "brick" => {
                    if in_bounds(e.x, e.y) {
                        map.set(e.x as usize, e.y as usize, true);
                        kinds.insert((e.x, e.y), if e.kind == "question" { TileKind::Question } else { TileKind::Brick });
                    }
                }
                "warp" | "pipe" => warps.push(Warp {
                    cx: e.x,
                    cy: e.y,
                    target: e.prop("warp").or_else(|| e.prop("to")).map(|s| s.to_string()),
                }),
                // Everything else (creatures, items, power-ups) is a renderable
                // entity; the runtime draws whatever the sprite registry knows.
                _ => ents.push(Ent { kind: e.kind.clone(), cx: e.x, cy: e.y }),
            }
        }

        let spawn = resolve_spawn(&map, w, h, lvl.spawn.0, lvl.spawn.1);
        map.spawn = spawn;
        let goal = lvl.goal.as_ref().map(|g| (g.x as f64 * TILE, g.y as f64 * TILE));

        LevelWorld { map, hazard, kinds, ents, w, h, spawn, goal, warps, theme: Theme::from_str(&lvl.theme) }
    }

    /// The tile kind drawn at cell (x,y), if any (for rendering).
    pub fn kind_at(&self, x: i32, y: i32) -> Option<TileKind> {
        self.kinds.get(&(x, y)).copied()
    }

    pub fn px_w(&self) -> f64 {
        self.w as f64 * TILE
    }
    pub fn px_h(&self) -> f64 {
        self.h as f64 * TILE
    }

    /// Does the AABB [x,x+w)×[y,y+h) touch any hazard cell?
    pub fn hazard_overlap(&self, x: f64, y: f64, w: f64, h: f64) -> bool {
        if self.hazard.is_empty() {
            return false;
        }
        let eps = 1e-6;
        let tx0 = (x / TILE).floor() as i32;
        let tx1 = ((x + w - eps) / TILE).floor() as i32;
        let ty0 = (y / TILE).floor() as i32;
        let ty1 = ((y + h - eps) / TILE).floor() as i32;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if self.hazard.contains(&(tx, ty)) {
                    return true;
                }
            }
        }
        false
    }

    /// The warp whose cell the AABB's center sits in, if any.
    pub fn warp_at(&self, x: f64, y: f64, w: f64, h: f64) -> Option<&Warp> {
        let cx = ((x + w / 2.0) / TILE).floor() as i32;
        let cy = ((y + h / 2.0) / TILE).floor() as i32;
        self.warps.iter().find(|wp| wp.cx == cx && wp.cy == cy)
    }
}

/// Resolve a spawn cell to a player-box top-left (px) that isn't embedded in
/// terrain. Imported levels put the Player node's Y on the ground *surface*, which
/// maps to a solid cell — placing the ~1-tile box's top-left there starts the
/// player inside the ground (stuck / unplayable). We lift the box straight up to
/// the first clear cell; physics then settles it onto the floor under gravity.
fn resolve_spawn(map: &TileMap, w: i32, h: i32, sx: i32, sy: i32) -> (f64, f64) {
    let sx = sx.clamp(0, w - 1);
    let mut sy = sy.clamp(0, h - 1);
    while sy > 0 && map.is_solid(sx, sy) {
        sy -= 1;
    }
    (sx as f64 * TILE, sy as f64 * TILE)
}

/// Clamped side-scroll camera: center the view on the player, but never scroll
/// past the level edges. Returns the view's top-left in pixels.
pub fn camera(player_cx: f64, player_cy: f64, view_w: f64, view_h: f64, level_w: f64, level_h: f64) -> (f64, f64) {
    let cx = (player_cx - view_w / 2.0).clamp(0.0, (level_w - view_w).max(0.0));
    let cy = (player_cy - view_h / 2.0).clamp(0.0, (level_h - view_h).max(0.0));
    (cx, cy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::ir::{Entity, Goal, TileSpan};

    fn demo() -> Level {
        let mut l = Level::new("t", "castle", 40, 12);
        l.spawn = (2, 9);
        l.goal = Some(Goal { kind: "flag".into(), x: 38, y: 2 });
        l.tiles.push(TileSpan { x: 0, y: 10, len: 40, kind: TileKind::Ground });
        l.tiles.push(TileSpan { x: 20, y: 9, len: 3, kind: TileKind::Hazard });
        l.tiles.push(TileSpan { x: 10, y: 6, len: 1, kind: TileKind::Deco }); // non-solid
        l.entities.push(Entity { kind: "question".into(), x: 6, y: 6, props: vec![] });
        l.entities.push(Entity {
            kind: "warp".into(),
            x: 14,
            y: 9,
            props: vec![("warp".into(), "t2@3,9".into())],
        });
        l
    }

    #[test]
    fn projects_solids_hazards_and_extras() {
        let w = LevelWorld::from_level(&demo());
        assert!(w.map.is_solid(0, 10) && w.map.is_solid(39, 10), "ground is solid");
        assert!(w.map.is_solid(6, 6), "question block entity is solid");
        assert_eq!(w.kind_at(6, 6), Some(TileKind::Question), "block entity renders as a question tile");
        assert_eq!(w.kind_at(0, 10), Some(TileKind::Ground));
        assert_eq!(w.kind_at(10, 6), Some(TileKind::Deco));
        assert!(!w.map.is_solid(10, 6), "deco is not solid");
        assert!(!w.map.is_solid(20, 9), "hazard cell is not solid (you fall in)");
        assert!(w.hazard.contains(&(20, 9)) && w.hazard.contains(&(22, 9)));
        assert_eq!(w.spawn, (2.0 * TILE, 9.0 * TILE));
        assert_eq!(w.goal, Some((38.0 * TILE, 2.0 * TILE)));
        assert_eq!(w.theme, Theme::Castle);
        assert_eq!(w.warps.len(), 1);
        assert_eq!(w.warps[0].target.as_deref(), Some("t2@3,9"));
    }

    #[test]
    fn hazard_and_warp_queries() {
        let w = LevelWorld::from_level(&demo());
        // player box over the hazard column
        assert!(w.hazard_overlap(20.0 * TILE, 9.0 * TILE, 12.0, 16.0));
        assert!(!w.hazard_overlap(0.0, 0.0, 12.0, 16.0));
        // center over the warp cell (14,9)
        let wp = w.warp_at(14.0 * TILE, 9.0 * TILE, 12.0, 16.0);
        assert!(wp.is_some() && wp.unwrap().target.as_deref() == Some("t2@3,9"));
    }

    #[test]
    fn spawn_is_lifted_out_of_solid_terrain() {
        let mut l = Level::new("t", "overworld", 10, 6);
        l.tiles.push(TileSpan { x: 0, y: 4, len: 10, kind: TileKind::Ground });
        l.tiles.push(TileSpan { x: 0, y: 5, len: 10, kind: TileKind::Ground });
        l.spawn = (3, 4); // on the solid ground surface — would start embedded
        let w = LevelWorld::from_level(&l);
        assert_eq!(w.spawn, (3.0 * TILE, 3.0 * TILE), "spawn lifts to the first clear cell above ground");

        // A spawn already in open air is left where it is.
        l.spawn = (3, 1);
        assert_eq!(LevelWorld::from_level(&l).spawn, (3.0 * TILE, 1.0 * TILE));
    }

    #[test]
    fn camera_centers_then_clamps_at_edges() {
        let (vw, vh, lw, lh) = (160.0, 96.0, 640.0, 96.0);
        // middle: centered on player
        assert_eq!(camera(320.0, 48.0, vw, vh, lw, lh), (320.0 - 80.0, 0.0));
        // left edge clamp
        assert_eq!(camera(10.0, 48.0, vw, vh, lw, lh).0, 0.0);
        // right edge clamp
        assert_eq!(camera(9999.0, 48.0, vw, vh, lw, lh).0, lw - vw);
    }
}
