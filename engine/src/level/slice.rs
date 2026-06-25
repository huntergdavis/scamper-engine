//! Slice a level into small, de-identified, full-height chunks for the slice
//! database (CAMPAIGN_PLAN: ship a remixable slice DB, not whole imported levels).
//!
//! A slice is a `width`-tile-wide vertical cut of a level — generic platforming
//! geometry (some ground, a block, a gap) with our own entity vocabulary, too
//! small to be a recognizable copy of any source level. The random stitcher
//! recombines slices into original layouts, so committing the slice DB lets
//! anyone use the random-walk without the (un-shippable) source levels.

use super::ir::{Entity, Level, TileSpan};

/// Uniform slice height — the playable strip kept from each source level (its
/// bottom `SLICE_H` rows). Standardizes the DB so stitched megas aren't dominated
/// by one tall vertical level's empty sky.
pub const SLICE_H: i32 = 16;

/// Cut `lvl` into consecutive `width`-tile slices (left to right). Each slice is a
/// standalone [`Level`] of fixed height [`SLICE_H`] (bottom-aligned to the source
/// ground; taller levels are top-cropped): x shifted to 0, no goal, warp targets
/// stripped, a generic id. Spans straddling a cut are clipped at the boundary.
pub fn slice_level(lvl: &Level, width: i32) -> Vec<Level> {
    let width = width.max(2);
    let yoff = SLICE_H - lvl.h; // map source bottom → slice bottom (may be negative)
    let in_h = |y: i32| (0..SLICE_H).contains(&y);
    let mut out = Vec::new();
    let mut x0 = 0;
    while x0 < lvl.w {
        let x1 = (x0 + width).min(lvl.w);
        let mut s = Level::new(format!("slice-{}-{x0}", lvl.id), &lvl.theme, x1 - x0, SLICE_H);
        s.spawn = (1, SLICE_H - 3);

        for t in &lvl.tiles {
            let a = t.x.max(x0);
            let b = (t.x + t.len).min(x1);
            if b > a && in_h(t.y + yoff) {
                s.tiles.push(TileSpan { x: a - x0, y: t.y + yoff, len: b - a, kind: t.kind });
            }
        }
        for e in &lvl.entities {
            if e.x >= x0 && e.x < x1 && in_h(e.y + yoff) {
                let props = if e.kind == "warp" || e.kind == "pipe" {
                    e.props.iter().filter(|(k, _)| k != "warp" && k != "to").cloned().collect()
                } else {
                    e.props.clone()
                };
                s.entities.push(Entity { kind: e.kind.clone(), x: e.x - x0, y: e.y + yoff, props });
            }
        }
        // No goal, no checkpoints — slices are anonymous chunks.
        out.push(s);
        x0 = x1;
    }
    out
}

/// A stable content fingerprint of a slice (ignoring its id), so the slicer can
/// drop exact duplicates (lots of slices are just flat ground).
pub fn slice_fingerprint(s: &Level) -> u64 {
    // FNV-1a over the geometry + entities (not the id/spawn).
    let mut h: u64 = 0xcbf29ce484222325;
    let mut byte = |b: u8, h: &mut u64| {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x100000001b3);
    };
    let mut feed = |n: i64, h: &mut u64| {
        for b in n.to_le_bytes() {
            byte(b, h);
        }
    };
    feed(s.w as i64, &mut h);
    feed(s.h as i64, &mut h);
    for t in &s.tiles {
        feed(t.x as i64, &mut h);
        feed(t.y as i64, &mut h);
        feed(t.len as i64, &mut h);
        feed(t.kind as i64, &mut h);
    }
    for e in &s.entities {
        for b in e.kind.bytes() {
            byte(b, &mut h);
        }
        feed(e.x as i64, &mut h);
        feed(e.y as i64, &mut h);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::super::ir::{Goal, TileKind};
    use super::*;

    #[test]
    fn slices_are_small_anonymous_and_clipped() {
        let mut l = Level::new("1-1", "overworld", 20, 12);
        l.goal = Some(Goal { kind: "flag".into(), x: 19, y: 2 });
        l.tiles.push(TileSpan { x: 0, y: 11, len: 20, kind: TileKind::Ground }); // full-width ground
        l.entities.push(Entity { kind: "boneling".into(), x: 13, y: 10, props: vec![] });

        let slices = slice_level(&l, 8);
        assert_eq!(slices.len(), 3, "20 / 8 → 3 slices (8,8,4)");
        assert_eq!((slices[0].w, slices[1].w, slices[2].w), (8, 8, 4));
        // Ground is clipped per slice and re-based to x=0.
        assert_eq!(slices[0].tiles[0].len, 8);
        assert_eq!(slices[0].tiles[0].x, 0);
        // No goal survives; the boneling lands in the 2nd slice at x = 13-8 = 5.
        assert!(slices.iter().all(|s| s.goal.is_none()));
        assert!(slices[1].entities.iter().any(|e| e.kind == "boneling" && e.x == 5));
        // Distinct geometry → distinct fingerprints (slice 0 vs slice 2 differ in width).
        assert_ne!(slice_fingerprint(&slices[0]), slice_fingerprint(&slices[2]));
    }
}
