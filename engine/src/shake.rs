//! Screen-shake "trauma" accumulator — a small camera tremble on impacts.
//!
//! Bump it when something hits (a stomp, a hurt, a boss bonk); each frame read a
//! `(dx, dy)` camera offset that trembles and fades. Trauma decays linearly and
//! the offset scales with `trauma²`, which reads as a sharp jolt that settles
//! quickly — the standard "juice" curve. The tremble is a cheap hash of the frame
//! counter (no RNG), so replays and golden snapshots stay deterministic.
//!
//! ```
//! use scamper::shake::Shake;
//! let mut s = Shake::new();
//! assert_eq!(s.offset(0, 6.0), (0.0, 0.0)); // calm: no shake
//! s.bump(1.0);
//! let (dx, dy) = s.offset(1, 6.0);
//! assert!(dx.abs() <= 6.0 && dy.abs() <= 6.0 && (dx != 0.0 || dy != 0.0));
//! ```

/// A decaying shake source. Cheap to copy/store; one per camera.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shake {
    trauma: f64,
}

impl Shake {
    pub fn new() -> Self {
        Shake { trauma: 0.0 }
    }

    /// Add `amount` of trauma (≈0.3 a small tap, 1.0 a big hit). Saturates at 1.
    pub fn bump(&mut self, amount: f64) {
        self.trauma = (self.trauma + amount).clamp(0.0, 1.0);
    }

    /// True while there's shake left to apply (lets callers force a redraw).
    pub fn active(&self) -> bool {
        self.trauma > 0.0
    }

    /// Advance one frame: decay the trauma and return the camera offset in px,
    /// peaking at `max_px` at full trauma. `frame` drives the deterministic
    /// tremble (pass a monotonically increasing counter).
    pub fn offset(&mut self, frame: u64, max_px: f64) -> (f64, f64) {
        let amt = self.trauma * self.trauma; // sharper falloff than linear
        self.trauma = (self.trauma - 0.05).max(0.0); // ~20 frames to settle from full
        if amt <= 0.0 {
            return (0.0, 0.0);
        }
        // Cheap deterministic pseudo-noise in [-0.5, 0.5] from the frame counter.
        let n = |salt: u64| -> f64 {
            let h = frame.wrapping_add(salt).wrapping_mul(0x9E3779B97F4A7C15);
            ((h >> 33) & 0xffff) as f64 / 65535.0 - 0.5
        };
        (n(1) * 2.0 * max_px * amt, n(7) * 2.0 * max_px * amt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shake_fades_and_bounds() {
        let mut s = Shake::new();
        s.bump(1.0);
        assert!(s.active());
        let (dx, dy) = s.offset(3, 8.0);
        assert!(dx.abs() <= 8.0 && dy.abs() <= 8.0, "offset within max_px");
        // It settles within a couple dozen frames.
        for f in 0..40 {
            s.offset(f, 8.0);
        }
        assert!(!s.active(), "shake settles to nothing");
        assert_eq!(s.offset(99, 8.0), (0.0, 0.0));
    }

    #[test]
    fn deterministic_for_same_frame() {
        let (mut a, mut b) = (Shake::new(), Shake::new());
        a.bump(0.8);
        b.bump(0.8);
        assert_eq!(a.offset(5, 6.0), b.offset(5, 6.0), "same trauma + frame → same offset");
    }
}
