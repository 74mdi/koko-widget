//! The time-based morph engine for the compact ↔ expanded transformation.
//!
//! There is exactly one animation state. Every retarget starts from the current
//! interpolated value, so rapid user input or back-to-back track changes cannot
//! stack transitions or make the widget jump.

use std::time::{Duration, Instant};

pub const EXPAND_DURATION_MS: u64 = 420;
pub const COLLAPSE_DURATION_MS: u64 = 340;

/// A cubic Bézier easing curve from `(0,0)` to `(1,1)`.
#[derive(Debug, Clone, Copy)]
pub struct CubicBezier {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

impl CubicBezier {
    pub const EXPAND: Self = Self {
        x1: 0.16,
        y1: 1.0,
        x2: 0.3,
        y2: 1.0,
    };
    pub const COLLAPSE: Self = Self {
        x1: 0.4,
        y1: 0.0,
        x2: 0.2,
        y2: 1.0,
    };

    fn sample_x(&self, t: f64) -> f64 {
        let mt = 1.0 - t;
        3.0 * mt * mt * t * self.x1 + 3.0 * mt * t * t * self.x2 + t * t * t
    }

    fn sample_y(&self, t: f64) -> f64 {
        let mt = 1.0 - t;
        3.0 * mt * mt * t * self.y1 + 3.0 * mt * t * t * self.y2 + t * t * t
    }

    /// Evaluate the curve for an input progress in `0..=1`.
    pub fn ease(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }

        // Solve x(t) = x with a small number of Newton iterations, falling back
        // to bisection if the derivative is degenerate.
        let mut t = x;
        for _ in 0..8 {
            let error = self.sample_x(t) - x;
            if error.abs() < 1e-7 {
                return self.sample_y(t);
            }
            let mt = 1.0 - t;
            let derivative = 3.0 * mt * mt * self.x1
                + 6.0 * mt * t * (self.x2 - self.x1)
                + 3.0 * t * t * (1.0 - self.x2);
            if derivative.abs() < 1e-6 {
                break;
            }
            t = (t - error / derivative).clamp(0.0, 1.0);
        }

        let mut low = 0.0;
        let mut high = 1.0;
        for _ in 0..32 {
            let mid = (low + high) * 0.5;
            if self.sample_x(mid) < x {
                low = mid;
            } else {
                high = mid;
            }
        }
        self.sample_y((low + high) * 0.5)
    }
}

/// One uninterrupted morph from its current position toward an expanded or
/// compact resting state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Compact,
    Expanding,
    Expanded,
    Collapsing,
}

#[derive(Debug, Clone)]
pub struct Morph {
    value: f32,
    from: f32,
    to: f32,
    started_at: Instant,
    duration: Duration,
    phase: Phase,
}

impl Default for Morph {
    fn default() -> Self {
        Self {
            value: 0.0,
            from: 0.0,
            to: 0.0,
            started_at: Instant::now(),
            duration: Duration::ZERO,
            phase: Phase::Compact,
        }
    }
}

impl Morph {
    pub fn new_compact() -> Self {
        Self::default()
    }

    pub fn new_expanded() -> Self {
        Self {
            value: 1.0,
            from: 1.0,
            to: 1.0,
            started_at: Instant::now(),
            duration: Duration::ZERO,
            phase: Phase::Expanded,
        }
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn is_active(&self) -> bool {
        matches!(self.phase, Phase::Expanding | Phase::Collapsing)
    }

    pub fn is_expanded(&self) -> bool {
        matches!(self.phase, Phase::Expanded)
    }

    pub fn is_compact(&self) -> bool {
        matches!(self.phase, Phase::Compact)
    }

    /// Begin (or reverse) a transition. Re-targeting while an animation is in
    /// flight always starts from the current visual value.
    pub fn retarget(&mut self, expanded: bool, now: Instant, speed: f32, reduced: bool) {
        let target = if expanded { 1.0 } else { 0.0 };
        let already_there = (self.value - target).abs() < f32::EPSILON;
        if self.is_active() && (self.to - target).abs() < f32::EPSILON {
            return;
        }
        if already_there && !self.is_active() {
            self.phase = if expanded {
                Phase::Expanded
            } else {
                Phase::Compact
            };
            self.value = target;
            self.from = target;
            self.to = target;
            self.duration = Duration::ZERO;
            return;
        }

        self.phase = if expanded {
            Phase::Expanding
        } else {
            Phase::Collapsing
        };
        self.from = self.value;
        self.to = target;
        self.started_at = now;

        if reduced || speed <= 0.0 {
            self.value = target;
            self.from = target;
            self.to = target;
            self.duration = Duration::ZERO;
            self.phase = if expanded {
                Phase::Expanded
            } else {
                Phase::Compact
            };
            return;
        }

        let base = if expanded {
            EXPAND_DURATION_MS
        } else {
            COLLAPSE_DURATION_MS
        } as f32;
        let distance = (target - self.from).abs();
        let scaled = base * (0.55 + 0.45 * distance) / speed.clamp(0.1, 8.0);
        self.duration = Duration::from_secs_f32(scaled / 1000.0);
    }

    /// Update the interpolated value from the monotonic clock.
    pub fn tick(&mut self, now: Instant) -> f32 {
        if !self.is_active() {
            return self.value;
        }
        if self.duration.is_zero() {
            self.finish();
            return self.value;
        }
        let elapsed = now.saturating_duration_since(self.started_at);
        let linear = (elapsed.as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0);
        let curve = if self.phase == Phase::Expanding {
            CubicBezier::EXPAND
        } else {
            CubicBezier::COLLAPSE
        };
        let eased = curve.ease(linear) as f32;
        self.value = self.from + (self.to - self.from) * eased;
        if linear >= 1.0 {
            self.finish();
        }
        self.value
    }

    fn finish(&mut self) {
        self.value = self.to;
        self.phase = if self.to >= 1.0 {
            Phase::Expanded
        } else {
            Phase::Compact
        };
    }

    /// Force a finished state without animating. Used when reduced motion is on.
    pub fn snap(&mut self, expanded: bool) {
        self.value = if expanded { 1.0 } else { 0.0 };
        self.from = self.value;
        self.to = self.value;
        self.phase = if expanded {
            Phase::Expanded
        } else {
            Phase::Compact
        };
        self.duration = Duration::ZERO;
    }
}

/// A small, cancellable deadline used for auto-collapse and hover intent.
#[derive(Debug, Clone, Copy, Default)]
pub struct Deadline {
    at: Option<Instant>,
}

impl Deadline {
    pub fn schedule(&mut self, now: Instant, delay: Duration) {
        self.at = Some(now + delay);
    }

    pub fn cancel(&mut self) {
        self.at = None;
    }

    pub fn is_scheduled(&self) -> bool {
        self.at.is_some()
    }

    pub fn due(&self, now: Instant) -> bool {
        self.at.is_some_and(|deadline| now >= deadline)
    }

    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.at
            .map(|deadline| deadline.saturating_duration_since(now))
    }
}

/// Opacity envelope for secondary controls. These thresholds deliberately have
/// different starts so collapsing removes clocks, then the progress bar, then
/// the previous-track control, ahead of the geometry shrinking.
pub fn aux_alpha(value: f32) -> f32 {
    smoothstep(0.15, 0.62, value)
}

pub fn progress_alpha(value: f32) -> f32 {
    smoothstep(0.35, 0.82, value)
}

pub fn time_alpha(value: f32) -> f32 {
    smoothstep(0.55, 0.95, value)
}

pub fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    if edge1 <= edge0 {
        return if value >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_has_exact_endpoints() {
        for curve in [CubicBezier::EXPAND, CubicBezier::COLLAPSE] {
            assert_eq!(curve.ease(0.0), 0.0);
            assert_eq!(curve.ease(1.0), 1.0);
            assert!(curve.ease(0.5) > 0.0 && curve.ease(0.5) < 1.0);
        }
    }

    #[test]
    fn morph_expands_and_finishes() {
        let start = Instant::now();
        let mut morph = Morph::new_compact();
        morph.retarget(true, start, 1.0, false);
        assert!(morph.is_active());
        let mid = morph.tick(start + Duration::from_millis(180));
        assert!(mid > 0.0 && mid < 1.0, "midpoint was {mid}");
        let end = morph.tick(start + Duration::from_secs(1));
        assert_eq!(end, 1.0);
        assert_eq!(morph.phase(), Phase::Expanded);
        assert!(!morph.is_active());
    }

    #[test]
    fn reversal_starts_from_current_value_without_jump() {
        let start = Instant::now();
        let mut morph = Morph::new_compact();
        morph.retarget(true, start, 1.0, false);
        let mid = morph.tick(start + Duration::from_millis(140));
        morph.retarget(false, start + Duration::from_millis(140), 1.0, false);
        let next = morph.tick(start + Duration::from_millis(140));
        assert!((next - mid).abs() < 0.001);
        assert_eq!(morph.phase(), Phase::Collapsing);
        let end = morph.tick(start + Duration::from_secs(2));
        assert_eq!(end, 0.0);
        assert_eq!(morph.phase(), Phase::Compact);
    }

    #[test]
    fn reduced_motion_is_instant() {
        let start = Instant::now();
        let mut morph = Morph::new_compact();
        morph.retarget(true, start, 1.0, true);
        assert_eq!(morph.value(), 1.0);
        assert_eq!(morph.phase(), Phase::Expanded);
    }

    #[test]
    fn repeated_same_target_is_a_noop() {
        let start = Instant::now();
        let mut morph = Morph::new_compact();
        morph.retarget(true, start, 1.0, false);
        morph.tick(start + Duration::from_millis(100));
        let before = morph.value();
        morph.retarget(true, start + Duration::from_millis(120), 1.0, false);
        assert_eq!(morph.value(), before);
        assert_eq!(morph.phase(), Phase::Expanding);
    }

    #[test]
    fn deadlines_are_cancellable() {
        let start = Instant::now();
        let mut deadline = Deadline::default();
        deadline.schedule(start, Duration::from_millis(500));
        assert!(!deadline.due(start + Duration::from_millis(499)));
        assert!(deadline.due(start + Duration::from_millis(500)));
        deadline.cancel();
        assert!(!deadline.due(start + Duration::from_secs(1)));
    }

    #[test]
    fn detail_layers_appear_in_order() {
        assert_eq!(aux_alpha(0.0), 0.0);
        assert_eq!(progress_alpha(0.0), 0.0);
        assert_eq!(time_alpha(0.0), 0.0);
        assert_eq!(aux_alpha(1.0), 1.0);
        assert_eq!(progress_alpha(1.0), 1.0);
        assert_eq!(time_alpha(1.0), 1.0);
        assert!(aux_alpha(0.5) > progress_alpha(0.5));
        assert!(progress_alpha(0.7) > time_alpha(0.7));
    }
}
