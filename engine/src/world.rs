//! Tile world + AABB collision queries (PROJECT_PLAN.md §4.7).
//! v1 uses axis-aligned solid tiles (the faithful N++ circle-vs-segment + slopes
//! is a planned refinement past the movement-completion milestone).

pub const TILE: f64 = 16.0;

pub struct TileMap {
    pub w: usize,
    pub h: usize,
    pub solid: Vec<bool>,
    /// One-way (semisolid) cells: solid to feet landing from above, pass-through
    /// from below or the sides. Drives platform tiles.
    pub oneway: Vec<bool>,
    pub has_oneway: bool, // fast-path: skip one-way checks when a map has none
    pub spawn: (f64, f64),
}

impl TileMap {
    pub fn new(w: usize, h: usize) -> Self {
        TileMap {
            w,
            h,
            solid: vec![false; w * h],
            oneway: vec![false; w * h],
            has_oneway: false,
            spawn: (TILE * 2.0, TILE * 2.0),
        }
    }

    #[inline]
    pub fn is_solid(&self, tx: i32, ty: i32) -> bool {
        if tx < 0 || ty < 0 || tx as usize >= self.w || ty as usize >= self.h {
            // Out of bounds: treat side/top as open, but below the map as solid-free
            // (player falls into a pit and respawns). Left/right/top walls are open too;
            // the level itself provides borders.
            return false;
        }
        self.solid[ty as usize * self.w + tx as usize]
    }

    #[inline]
    pub fn is_oneway(&self, tx: i32, ty: i32) -> bool {
        if tx < 0 || ty < 0 || tx as usize >= self.w || ty as usize >= self.h {
            return false;
        }
        self.oneway[ty as usize * self.w + tx as usize]
    }

    pub fn set(&mut self, tx: usize, ty: usize, v: bool) {
        if tx < self.w && ty < self.h {
            self.solid[ty * self.w + tx] = v;
        }
    }

    pub fn set_oneway(&mut self, tx: usize, ty: usize, v: bool) {
        if tx < self.w && ty < self.h {
            self.oneway[ty * self.w + tx] = v;
            self.has_oneway |= v;
        }
    }

    /// A descending AABB lands on a one-way platform when its feet cross the
    /// platform's top edge this step. `prev_bottom`/`new_bottom` are the AABB
    /// bottom before/after the vertical move. Upward/horizontal motion passes
    /// through (semisolid); feet already below the top don't re-catch.
    pub fn lands_on_oneway(&self, x: f64, w: f64, prev_bottom: f64, new_bottom: f64) -> bool {
        if !self.has_oneway || new_bottom <= prev_bottom {
            return false;
        }
        let eps = 1e-6;
        let tx0 = (x / TILE).floor() as i32;
        let tx1 = ((x + w - eps) / TILE).floor() as i32;
        let ty = (new_bottom / TILE).floor() as i32;
        let top = ty as f64 * TILE;
        if prev_bottom <= top + 1.0 {
            for tx in tx0..=tx1 {
                if self.is_oneway(tx, ty) {
                    return true;
                }
            }
        }
        false
    }

    /// Is the AABB resting on top of a one-way platform (so it counts as grounded)?
    /// True only when the feet sit at/just above the platform's top — not mid-pass.
    pub fn on_oneway(&self, x: f64, w: f64, feet_y: f64) -> bool {
        if !self.has_oneway {
            return false;
        }
        let eps = 1e-6;
        let tx0 = (x / TILE).floor() as i32;
        let tx1 = ((x + w - eps) / TILE).floor() as i32;
        let ty = ((feet_y + 1.0) / TILE).floor() as i32; // tile just below the feet
        let top = ty as f64 * TILE;
        if feet_y <= top + 2.0 {
            for tx in tx0..=tx1 {
                if self.is_oneway(tx, ty) {
                    return true;
                }
            }
        }
        false
    }

    pub fn px_w(&self) -> f64 {
        self.w as f64 * TILE
    }
    pub fn px_h(&self) -> f64 {
        self.h as f64 * TILE
    }

    /// Does the AABB [x, x+w) x [y, y+h) overlap any solid tile?
    pub fn overlaps(&self, x: f64, y: f64, w: f64, h: f64) -> bool {
        let eps = 1e-6;
        let tx0 = (x / TILE).floor() as i32;
        let tx1 = ((x + w - eps) / TILE).floor() as i32;
        let ty0 = (y / TILE).floor() as i32;
        let ty1 = ((y + h - eps) / TILE).floor() as i32;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if self.is_solid(tx, ty) {
                    return true;
                }
            }
        }
        false
    }

    /// Build a map from ASCII rows: '#' solid, '@' spawn, others empty.
    pub fn from_ascii(rows: &[&str]) -> Self {
        let h = rows.len();
        let w = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut m = TileMap::new(w, h);
        for (ty, row) in rows.iter().enumerate() {
            for (tx, ch) in row.chars().enumerate() {
                match ch {
                    '#' => m.set(tx, ty, true),
                    '=' => m.set_oneway(tx, ty, true), // one-way (semisolid) platform
                    '@' => m.spawn = (tx as f64 * TILE, ty as f64 * TILE),
                    _ => {}
                }
            }
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_basic() {
        let m = TileMap::from_ascii(&["....", "####"]);
        // tile row 1 (y in [16,32)) is solid
        assert!(m.overlaps(0.0, 20.0, 8.0, 8.0));
        assert!(!m.overlaps(0.0, 0.0, 8.0, 8.0));
    }

    #[test]
    fn spawn_parsed() {
        let m = TileMap::from_ascii(&["..@.", "####"]);
        assert_eq!(m.spawn, (2.0 * TILE, 0.0));
    }
}
