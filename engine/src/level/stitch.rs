//! Stitch many levels into one giant test level (CAMPAIGN_PLAN red-team aid).
//!
//! Concatenates level segments left-to-right, bottom-aligned, with a short ground
//! bridge across each seam so you can walk the whole thing. Finish poles (goals)
//! are dropped — the mega level is a continuous romp through every game system,
//! not something you "win". Cross-level warp targets are stripped (the
//! destinations aren't in the combined level) so pipes are inert, not broken.

use super::ir::{Entity, Level, TileKind, TileSpan};

/// Build one level from `parts`, in order. `gap` is the bridge width (tiles)
/// inserted between segments. Spawn comes from the first segment; there is no goal.
pub fn stitch(parts: &[Level], gap: i32) -> Level {
    let h = parts.iter().map(|l| l.h).max().unwrap_or(15).max(1);
    let gap = gap.max(0);
    let mut out = Level::new("megalevel", "overworld", 0, h);
    out.spawn = (2, h - 3);

    let mut xoff = 0;
    for (i, p) in parts.iter().enumerate() {
        let yoff = h - p.h; // bottom-align each segment

        for t in &p.tiles {
            out.tiles.push(TileSpan { x: t.x + xoff, y: t.y + yoff, len: t.len, kind: t.kind });
        }
        for e in &p.entities {
            // Strip warp destinations: the target levels aren't in the mega, so a
            // live warp would teleport nowhere / mis-load. Keep the pipe visual.
            let props: Vec<(String, String)> = if e.kind == "warp" || e.kind == "pipe" {
                e.props.iter().filter(|(k, _)| k != "warp" && k != "to").cloned().collect()
            } else {
                e.props.clone()
            };
            out.entities.push(Entity { kind: e.kind.clone(), x: e.x + xoff, y: e.y + yoff, props });
        }
        for c in &p.checkpoints {
            out.checkpoints.push((c.0 + xoff, c.1 + yoff));
        }
        if i == 0 {
            out.spawn = (p.spawn.0 + xoff, p.spawn.1 + yoff);
        }
        // No goal is carried — finish poles are intentionally removed.

        xoff += p.w;
        if i + 1 < parts.len() {
            // A 2-row ground bridge across the seam so the romp stays walkable.
            out.tiles.push(TileSpan { x: xoff, y: h - 1, len: gap, kind: TileKind::Ground });
            out.tiles.push(TileSpan { x: xoff, y: h - 2, len: gap, kind: TileKind::Ground });
            xoff += gap;
        }
    }
    out.w = xoff.max(1);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ir::Goal;

    fn seg(id: &str, w: i32, h: i32) -> Level {
        let mut l = Level::new(id, "overworld", w, h);
        l.spawn = (1, h - 3);
        l.goal = Some(Goal { kind: "flag".into(), x: w - 1, y: 2 });
        l.tiles.push(TileSpan { x: 0, y: h - 1, len: w, kind: TileKind::Ground });
        l.entities.push(Entity { kind: "boneling".into(), x: 3, y: h - 2, props: vec![] });
        l
    }

    #[test]
    fn stitches_segments_drops_goals_offsets_content() {
        let a = seg("a", 10, 12);
        let b = seg("b", 20, 15);
        let m = stitch(&[a, b], 4);

        // Width = 10 + 4 (bridge) + 20 = 34; height = max(12,15) = 15.
        assert_eq!(m.w, 34);
        assert_eq!(m.h, 15);
        // No finish pole survives.
        assert!(m.goal.is_none(), "the mega level has no goal");
        // Spawn comes from the first segment (bottom-aligned: a has h=12 → yoff=3).
        assert_eq!(m.spawn, (1, 12));
        // The second segment's boneling is offset past the first segment + bridge.
        assert!(m.entities.iter().any(|e| e.kind == "boneling" && e.x == 3 + 14), "segment B entities are x-offset");
        // Two segments' bonelings both present.
        assert_eq!(m.entities.iter().filter(|e| e.kind == "boneling").count(), 2);
    }

    #[test]
    fn empty_and_single() {
        assert_eq!(stitch(&[], 4).w, 1);
        let m = stitch(&[seg("a", 8, 10)], 4);
        assert_eq!(m.w, 8);
        assert!(m.goal.is_none());
    }
}
