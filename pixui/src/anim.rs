//! Motion primitives.
//!
//! A pixel button that snaps instantly between two states feels cheap; the
//! satisfying part is the tiny overshoot on release. That needs a spring, not a
//! lerp, so pixui ships one.

/// A damped harmonic oscillator, integrated semi-implicitly.
///
/// Under-damping it slightly is the whole trick: the value shoots a little past
/// its target and settles back, which is what reads as "springy" once it is
/// quantised to two or three pixels of travel.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub pos: f32,
    pub vel: f32,
    /// How hard it is pulled towards the target.
    pub stiffness: f32,
    /// How quickly the oscillation dies out.
    pub damping: f32,
}

impl Default for Spring {
    fn default() -> Self {
        Self {
            pos: 0.0,
            vel: 0.0,
            stiffness: 900.0,
            damping: 34.0,
        }
    }
}

impl Spring {
    pub fn new(stiffness: f32, damping: f32) -> Self {
        Self {
            pos: 0.0,
            vel: 0.0,
            stiffness,
            damping,
        }
    }

    /// Advance towards `target` by `dt` seconds.
    ///
    /// A stiff spring integrated at whatever frame rate the machine happens to
    /// deliver will explode, so this always substeps at a fixed 1/480s and
    /// refuses to advance more than 1/15s at a time.
    pub fn step(&mut self, target: f32, dt: f32) {
        const STEP: f32 = 1.0 / 480.0;
        let mut remaining = dt.clamp(0.0, 1.0 / 15.0);
        while remaining > 0.0 {
            let h = remaining.min(STEP);
            let force = (target - self.pos) * self.stiffness - self.vel * self.damping;
            self.vel += force * h;
            self.pos += self.vel * h;
            remaining -= h;
        }
    }

    /// Jump straight to a value, killing any momentum.
    pub fn snap(&mut self, value: f32) {
        self.pos = value;
        self.vel = 0.0;
    }

    /// Give the spring a shove without moving the target.
    pub fn kick(&mut self, velocity: f32) {
        self.vel += velocity;
    }
}

/// Frame-rate independent exponential smoothing towards `target`.
///
/// `rate` is roughly "how many e-foldings per second" — 20 is brisk, 8 is lazy.
pub fn smooth(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let t = 1.0 - (-rate * dt.clamp(0.0, 1.0 / 15.0)).exp();
    current + (target - current) * t
}

/// Per-widget animation state, kept alive between frames by the [`crate::Ui`].
#[derive(Clone, Copy, Debug, Default)]
pub struct WidgetAnim {
    /// 0 = at rest, 1 = fully depressed. Overshoots slightly on release.
    pub press: Spring,
    /// 0..1 hover blend.
    pub hover: f32,
    /// 1 the instant a click lands, decaying to 0. Drives the highlight flash.
    pub flash: f32,
    /// 0..1 focus-ring blend.
    pub focus: f32,
    /// Generic animated value, e.g. a toggle knob sliding across its track.
    pub value: Spring,
    /// Frame index this widget was last seen, so stale entries can be dropped.
    pub(crate) touched: u64,
}
