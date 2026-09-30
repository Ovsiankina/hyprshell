//! Wall-clock time to hand angles, and the sleep until the next tick.
//! Pure functions over a `ClockTime` so tests never touch the real clock.

use chrono::Timelike;
use std::f32::consts::TAU;
use std::time::Duration;

const NANOS_PER_SEC: u64 = 1_000_000_000;

/// Local wall-clock time of day.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ClockTime {
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub nanos: u32,
}

impl ClockTime {
    pub fn now() -> Self {
        Self::from(chrono::Local::now().time())
    }

    pub const fn hms(hour: u32, minute: u32, second: u32) -> Self {
        Self { hour, minute, second, nanos: 0 }
    }

    /// Nanoseconds elapsed since midnight.
    fn since_midnight(self) -> u64 {
        u64::from(self.hour) * 3600 * NANOS_PER_SEC
            + u64::from(self.minute) * 60 * NANOS_PER_SEC
            + u64::from(self.second) * NANOS_PER_SEC
            + u64::from(self.nanos)
    }

    /// Time until the next whole second (never zero).
    pub fn until_next_second(self) -> Duration {
        Duration::from_nanos(NANOS_PER_SEC - u64::from(self.nanos.min(NANOS_PER_SEC as u32 - 1)))
    }

    /// Time until the next whole minute (never zero).
    pub fn until_next_minute(self) -> Duration {
        let minute = 60 * NANOS_PER_SEC;
        let into = self.since_midnight() % minute;
        Duration::from_nanos(minute - into)
    }
}

impl From<chrono::NaiveTime> for ClockTime {
    fn from(t: chrono::NaiveTime) -> Self {
        Self {
            hour: t.hour(),
            minute: t.minute(),
            // chrono folds leap seconds into nanosecond >= 1e9; clamp them away.
            second: t.second().min(59),
            nanos: t.nanosecond().min(NANOS_PER_SEC as u32 - 1),
        }
    }
}

/// Hand angles in radians, clockwise from 12 o'clock.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Angles {
    pub hour: f32,
    pub minute: f32,
    pub second: f32,
}

impl Angles {
    /// Hands at whole ticks: the second hand at whole seconds, the minute and
    /// hour hands advancing continuously within the minute and hour, so the
    /// per-minute redraw shows them exactly where they belong.
    pub fn at(t: ClockTime) -> Self {
        let second = t.second as f32;
        let minute = t.minute as f32 + second / 60.0;
        let hour = (t.hour % 12) as f32 + minute / 60.0;
        Self {
            hour: hour / 12.0 * TAU,
            minute: minute / 60.0 * TAU,
            second: second / 60.0 * TAU,
        }
    }
}

impl Angles {
    /// Move from `from` towards `self` by `t` in `0.0..=1.0`, always in the
    /// clockwise direction so 59 s -> 0 s sweeps through 12, not backwards.
    pub fn eased_from(self, from: Angles, t: f32) -> Angles {
        let t = ease_out(t.clamp(0.0, 1.0));
        let step = |a: f32, b: f32| {
            let delta = (b - a).rem_euclid(TAU);
            (a + delta * t).rem_euclid(TAU)
        };
        Angles {
            hour: step(from.hour, self.hour),
            minute: step(from.minute, self.minute),
            second: step(from.second, self.second),
        }
    }
}

/// Cubic ease-out: fast start, gentle stop, like a real escapement.
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_at_three_o_clock() {
        let a = Angles::at(ClockTime::hms(15, 0, 0));
        assert!((a.hour - TAU / 4.0).abs() < 1e-6);
        assert_eq!(a.minute, 0.0);
        assert_eq!(a.second, 0.0);
    }

    #[test]
    fn hour_hand_moves_within_the_hour() {
        let a = Angles::at(ClockTime::hms(0, 30, 0));
        assert!((a.hour - TAU / 24.0).abs() < 1e-6);
        assert!((a.minute - TAU / 2.0).abs() < 1e-6);
    }

    #[test]
    fn easing_wraps_clockwise_and_lands_exactly() {
        let from = Angles::at(ClockTime::hms(0, 0, 59));
        let to = Angles::at(ClockTime::hms(0, 1, 0));
        let mid = to.eased_from(from, 0.5);
        // Halfway the second hand is between 59 s and 60 s, i.e. just before 12.
        assert!(mid.second > from.second && mid.second < TAU, "{mid:?}");
        assert_eq!(to.eased_from(from, 1.0), Angles { second: 0.0, ..to });
        assert_eq!(to.eased_from(from, 0.0), from);
    }

    #[test]
    fn until_next_boundaries() {
        let t = ClockTime { hour: 1, minute: 2, second: 3, nanos: 250_000_000 };
        assert_eq!(t.until_next_second(), Duration::from_millis(750));
        assert_eq!(t.until_next_minute(), Duration::from_millis(56_750));
        let exact = ClockTime::hms(1, 2, 0);
        assert_eq!(exact.until_next_minute(), Duration::from_secs(60));
        assert_eq!(exact.until_next_second(), Duration::from_secs(1));
    }
}
