//! Tile world + AABB collision queries (PROJECT_PLAN.md §4.7).
//! v1 uses axis-aligned solid tiles (the faithful N++ circle-vs-segment + slopes
//! is a planned refinement past the movement-completion milestone).

pub const TILE: f64 = 16.0;

pub struct TileMap {
    pub w: usize,
    pub h: usize,
    pub solid: Vec<bool>,
    pub spawn: (f64, f64),
}

impl TileMap {
    pub fn new(w: usize, h: usize) -> Self {
        TileMap { w, h, solid: vec![false; w * h], spawn: (TILE * 2.0, TILE * 2.0) }
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

    pub fn set(&mut self, tx: usize, ty: usize, v: bool) {
        if tx < self.w && ty < self.h {
            self.solid[ty * self.w + tx] = v;
        }
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
