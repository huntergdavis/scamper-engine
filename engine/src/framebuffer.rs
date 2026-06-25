//! A simple RGBA8 framebuffer the engine composites into each frame.
//! Origin is top-left, +y points down (screen convention).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Rgba { r, g, b, a }
    }
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba { r, g, b, a: 255 }
    }
}

pub struct Framebuffer {
    pub width: usize,
    pub height: usize,
    /// RGBA8, row-major, length = width*height*4.
    pub px: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Framebuffer { width, height, px: vec![0; width * height * 4] }
    }

    /// Resize, reusing the allocation when possible. Contents become undefined.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.px.resize(width * height * 4, 0);
    }

    #[inline]
    pub fn clear(&mut self, c: Rgba) {
        let mut i = 0;
        while i < self.px.len() {
            self.px[i] = c.r;
            self.px[i + 1] = c.g;
            self.px[i + 2] = c.b;
            self.px[i + 3] = c.a;
            i += 4;
        }
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, c: Rgba) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let i = (y as usize * self.width + x as usize) * 4;
        self.px[i] = c.r;
        self.px[i + 1] = c.g;
        self.px[i + 2] = c.b;
        self.px[i + 3] = c.a;
    }

    /// Alpha-blend a source color over the existing pixel (src-over).
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, c: Rgba) {
        if c.a == 0 {
            return;
        }
        if c.a == 255 {
            self.set(x, y, c);
            return;
        }
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let i = (y as usize * self.width + x as usize) * 4;
        let a = c.a as u32;
        let ia = 255 - a;
        let mix = |s: u8, d: u8| -> u8 { ((s as u32 * a + d as u32 * ia) / 255) as u8 };
        self.px[i] = mix(c.r, self.px[i]);
        self.px[i + 1] = mix(c.g, self.px[i + 1]);
        self.px[i + 2] = mix(c.b, self.px[i + 2]);
        self.px[i + 3] = 255;
    }

    /// Fill an axis-aligned rectangle (integer pixel coords), alpha-blended.
    /// Saturating bounds so extreme coords clip cleanly instead of overflowing.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgba) {
        if w <= 0 || h <= 0 {
            return;
        }
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = x.saturating_add(w).min(self.width as i32);
        let y1 = y.saturating_add(h).min(self.height as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                self.blend(px, py, c);
            }
        }
    }

    /// Draw a 1px rectangle outline.
    pub fn stroke_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgba) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.fill_rect(x, y, w, 1, c);
        self.fill_rect(x, y.saturating_add(h - 1), w, 1, c);
        self.fill_rect(x, y, 1, h, c);
        self.fill_rect(x.saturating_add(w - 1), y, 1, h, c);
    }

    /// Nearest-neighbor magnify: fill this buffer from `src`, scaling up by the
    /// integer factor `scale` (each src pixel becomes a `scale`×`scale` block).
    /// Self pixel (x, y) samples src at (x/scale, y/scale). Used for the "tiny
    /// world" zoom — the environment renders small, then blows up blocky.
    pub fn upscale_from(&mut self, src: &Framebuffer, scale: usize) {
        if scale <= 1 {
            // 1× (or degenerate) — straight copy of whatever overlaps.
            let h = self.height.min(src.height);
            let w = self.width.min(src.width);
            for y in 0..h {
                let s = y * src.width * 4;
                let d = y * self.width * 4;
                self.px[d..d + w * 4].copy_from_slice(&src.px[s..s + w * 4]);
            }
            return;
        }
        let (sw, sh) = (src.width, src.height);
        for y in 0..self.height {
            let sy = (y / scale).min(sh.saturating_sub(1));
            let srow = sy * sw * 4;
            let drow = y * self.width * 4;
            for x in 0..self.width {
                let sx = (x / scale).min(sw.saturating_sub(1));
                let si = srow + sx * 4;
                let di = drow + x * 4;
                self.px[di..di + 4].copy_from_slice(&src.px[si..si + 4]);
            }
        }
    }

    /// Bresenham line (used for debug overlays like velocity vectors).
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        loop {
            self.blend(x, y, c);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_and_read() {
        let mut fb = Framebuffer::new(8, 8);
        fb.clear(Rgba::rgb(0, 0, 0));
        fb.fill_rect(2, 2, 3, 3, Rgba::rgb(255, 0, 0));
        let i = (3 * 8 + 3) * 4;
        assert_eq!(fb.px[i], 255);
        // outside the rect stays black
        assert_eq!(fb.px[0], 0);
    }

    #[test]
    fn upscale_magnifies_blocks() {
        let mut src = Framebuffer::new(2, 2);
        src.clear(Rgba::rgb(0, 0, 0));
        src.set(0, 0, Rgba::rgb(255, 0, 0)); // top-left red
        src.set(1, 1, Rgba::rgb(0, 255, 0)); // bottom-right green
        let mut dst = Framebuffer::new(4, 4);
        dst.upscale_from(&src, 2);
        // src(0,0) fills dst's top-left 2×2 block.
        assert_eq!(dst.px[0], 255, "dst(0,0) red");
        assert_eq!(dst.px[(1 * 4 + 1) * 4], 255, "dst(1,1) still red");
        // src(1,1) fills dst's bottom-right 2×2 block.
        assert_eq!(dst.px[(3 * 4 + 3) * 4 + 1], 255, "dst(3,3) green");
    }

    #[test]
    fn blend_half() {
        let mut fb = Framebuffer::new(2, 2);
        fb.clear(Rgba::rgb(0, 0, 0));
        fb.blend(0, 0, Rgba::new(255, 255, 255, 128));
        // ~50% of 255
        assert!(fb.px[0] >= 120 && fb.px[0] <= 135);
    }
}
