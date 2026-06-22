//! Minimal 2D vector math. f64 for sub-pixel precision and same-binary
//! deterministic replay (see PROJECT_PLAN.md §4.3).

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

#[inline]
pub fn vec2(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    #[inline]
    pub fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }

    #[inline]
    pub fn len_sq(self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    #[inline]
    pub fn len(self) -> f64 {
        // sqrt is IEEE correctly-rounded → deterministic on a given binary.
        self.len_sq().sqrt()
    }

    #[inline]
    pub fn normalized(self) -> Vec2 {
        let l = self.len();
        if l > 1e-12 {
            Vec2 { x: self.x / l, y: self.y / l }
        } else {
            Vec2::ZERO
        }
    }

    #[inline]
    pub fn dot(self, o: Vec2) -> f64 {
        self.x * o.x + self.y * o.y
    }

    #[inline]
    pub fn lerp(self, o: Vec2, t: f64) -> Vec2 {
        Vec2 {
            x: self.x + (o.x - self.x) * t,
            y: self.y + (o.y - self.y) * t,
        }
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    #[inline]
    fn add(self, o: Vec2) -> Vec2 {
        Vec2 { x: self.x + o.x, y: self.y + o.y }
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    #[inline]
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2 { x: self.x - o.x, y: self.y - o.y }
    }
}
impl Mul<f64> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn mul(self, s: f64) -> Vec2 {
        Vec2 { x: self.x * s, y: self.y * s }
    }
}
impl Div<f64> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn div(self, s: f64) -> Vec2 {
        Vec2 { x: self.x / s, y: self.y / s }
    }
}
impl Neg for Vec2 {
    type Output = Vec2;
    #[inline]
    fn neg(self) -> Vec2 {
        Vec2 { x: -self.x, y: -self.y }
    }
}
impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, o: Vec2) {
        self.x += o.x;
        self.y += o.y;
    }
}
impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, o: Vec2) {
        self.x -= o.x;
        self.y -= o.y;
    }
}

#[inline]
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        let a = vec2(3.0, 4.0);
        assert_eq!(a.len(), 5.0);
        assert_eq!((a + vec2(1.0, 1.0)), vec2(4.0, 5.0));
        assert_eq!(a * 2.0, vec2(6.0, 8.0));
        let n = a.normalized();
        assert!((n.len() - 1.0).abs() < 1e-9);
    }
}
