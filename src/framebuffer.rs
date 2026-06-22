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
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgba) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.width as i32);
        let y1 = (y + h).min(self.height as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                self.blend(px, py, c);
            }
        }
    }

    /// Draw a 1px rectangle outline.
    pub fn stroke_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgba) {
        self.fill_rect(x, y, w, 1, c);
        self.fill_rect(x, y + h - 1, w, 1, c);
        self.fill_rect(x, y, 1, h, c);
        self.fill_rect(x + w - 1, y, 1, h, c);
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
    fn blend_half() {
        let mut fb = Framebuffer::new(2, 2);
        fb.clear(Rgba::rgb(0, 0, 0));
        fb.blend(0, 0, Rgba::new(255, 255, 255, 128));
        // ~50% of 255
        assert!(fb.px[0] >= 120 && fb.px[0] <= 135);
    }
}
