//! The view-model: plain data derived from `State`, compared with `PartialEq`
//! before anything touches the GPU. Hyprland options are folded in here as
//! defaults; config values always win.

use crate::clock::{Angles, ClockTime};
use crate::color::Rgba;
use crate::config::{Anchor, Hand};
use crate::state::State;
use std::collections::BTreeMap;

#[derive(Clone, PartialEq, Debug)]
pub struct ViewModel {
    /// Surfaces that should exist, by monitor name. Empty while hidden.
    pub surfaces: BTreeMap<String, ClockView>,
    pub tick: TickRate,
}

/// How often the state must be re-derived. `Off` means no timer at all.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TickRate {
    Off,
    /// One tick per wall-clock minute, on the minute.
    Minute,
    /// One tick per wall-clock second, on the second.
    Second,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ClockView {
    pub size: u32,
    pub anchor: Anchor,
    pub margin: u32,
    pub face: FaceView,
    pub hour: HandView,
    pub minute: HandView,
    /// `None` when seconds are off or the monitor is covered by a fullscreen
    /// window.
    pub second: Option<HandView>,
    pub animate: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct FaceView {
    pub color: Rgba,
    pub border_color: Rgba,
    pub border_width: f32,
    /// Corner radius in logical pixels, already clamped to `size / 2`.
    pub rounding: f32,
}

#[derive(Clone, PartialEq, Debug)]
pub struct HandView {
    /// Radians clockwise from 12 o'clock.
    pub angle: f32,
    /// Fraction of the face radius.
    pub length: f32,
    /// Logical pixels.
    pub width: f32,
    pub color: Rgba,
}

impl HandView {
    fn new(hand: &Hand, angle: f32) -> Self {
        Self { angle, length: hand.length, width: hand.width, color: hand.color }
    }
}

/// Used when neither the config nor Hyprland provides a value.
const FALLBACK_BORDER: Rgba = Rgba::new(0.57, 0.57, 0.57, 1.0);
const FALLBACK_BORDER_WIDTH: u32 = 2;

impl ViewModel {
    pub fn derive(state: &State) -> Self {
        let c = &state.config.clock;
        let mut surfaces = BTreeMap::new();
        let mut any_seconds = false;

        if state.visible {
            for (name, monitor) in &state.monitors {
                let show_second = c.show_seconds && !monitor.fullscreen;
                any_seconds |= show_second;
                // A monitor without a second hand must not change every second.
                let minute_only = |t: ClockTime| ClockTime { second: 0, nanos: 0, ..t };
                let (time, prev) = if show_second {
                    (state.time, state.prev_time)
                } else {
                    (minute_only(state.time), minute_only(state.prev_time))
                };
                let angles = Angles::at(time).eased_from(Angles::at(prev), state.ease);
                let half = (c.size / 2) as f32;
                let rounding = c
                    .face
                    .rounding
                    .or(state.hypr.rounding)
                    .map(|r| (r as f32).min(half))
                    .unwrap_or(half);
                surfaces.insert(
                    name.clone(),
                    ClockView {
                        size: c.size,
                        anchor: c.position,
                        margin: c.margin,
                        face: FaceView {
                            color: c.face.color,
                            border_color: c.face.border_color.or(state.hypr.active_border).unwrap_or(FALLBACK_BORDER),
                            border_width: c
                                .face
                                .border_width
                                .or(state.hypr.border_size)
                                .unwrap_or(FALLBACK_BORDER_WIDTH) as f32,
                            rounding,
                        },
                        hour: HandView::new(&c.hands.hour, angles.hour),
                        minute: HandView::new(&c.hands.minute, angles.minute),
                        second: show_second.then(|| HandView::new(&c.hands.second, angles.second)),
                        animate: c.animation,
                    },
                );
            }
        }

        let tick = if surfaces.is_empty() {
            TickRate::Off
        } else if any_seconds {
            TickRate::Second
        } else {
            TickRate::Minute
        };
        Self { surfaces, tick }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::hypr::Defaults;

    fn state() -> State {
        let hypr = Defaults {
            active_border: Some(Rgba::from_rgba8(0x91, 0x91, 0x91, 0x77)),
            border_size: Some(1),
            rounding: Some(18),
        };
        let mut s = State::new(Config::default(), hypr, ClockTime::hms(10, 9, 30));
        s.monitor_added("DP-2");
        s.monitor_added("DP-3");
        s
    }

    #[test]
    fn hyprland_options_are_defaults_and_config_wins() {
        let s = state();
        let vm = s.view_model();
        let dp2 = &vm.surfaces["DP-2"];
        assert_eq!(dp2.face.border_color, Rgba::from_rgba8(0x91, 0x91, 0x91, 0x77));
        assert_eq!(dp2.face.border_width, 1.0);
        assert_eq!(dp2.face.rounding, 18.0);
        assert_eq!(vm.tick, TickRate::Second);

        let mut s = s;
        s.config.clock.face.border_width = Some(4);
        s.config.clock.face.border_color = Some(Rgba::from_rgba8(1, 2, 3, 255));
        s.config.clock.face.rounding = Some(1000);
        let dp2 = &s.view_model().surfaces["DP-2"];
        assert_eq!(dp2.face.border_width, 4.0);
        assert_eq!(dp2.face.border_color, Rgba::from_rgba8(1, 2, 3, 255));
        assert_eq!(dp2.face.rounding, 110.0, "rounding clamps to half the size");
    }

    #[test]
    fn no_hyprland_no_config_falls_back_to_circle() {
        let mut s = state();
        s.hypr = Defaults::default();
        let dp2 = &s.view_model().surfaces["DP-2"];
        assert_eq!(dp2.face.rounding, 110.0);
        assert_eq!(dp2.face.border_width, 2.0);
    }

    #[test]
    fn fullscreen_monitor_drops_second_hand_and_stays_stable_across_seconds() {
        let mut s = state();
        s.set_fullscreen_monitors(&["DP-2".to_string()]);
        let before = s.view_model();
        assert!(before.surfaces["DP-2"].second.is_none());
        assert!(before.surfaces["DP-3"].second.is_some());
        assert_eq!(before.tick, TickRate::Second, "DP-3 still needs seconds");

        s.tick(ClockTime::hms(10, 9, 31));
        let after = s.view_model();
        assert_eq!(before.surfaces["DP-2"], after.surfaces["DP-2"], "covered clock must not redraw");
        assert_ne!(before.surfaces["DP-3"], after.surfaces["DP-3"]);

        s.set_fullscreen_monitors(&["DP-2".to_string(), "DP-3".to_string()]);
        assert_eq!(s.view_model().tick, TickRate::Minute);
    }

    #[test]
    fn seconds_off_means_minute_ticks_and_minute_granularity() {
        let mut s = state();
        s.config.clock.show_seconds = false;
        let a = s.view_model();
        assert_eq!(a.tick, TickRate::Minute);
        s.tick(ClockTime::hms(10, 9, 45));
        assert_eq!(a, s.view_model());
        s.tick(ClockTime::hms(10, 10, 0));
        assert_ne!(a, s.view_model());
    }

    #[test]
    fn hidden_means_no_surfaces_and_no_timer() {
        let mut s = state();
        s.toggle();
        let vm = s.view_model();
        assert!(vm.surfaces.is_empty());
        assert_eq!(vm.tick, TickRate::Off);
        s.toggle();
        assert_eq!(s.view_model().surfaces.len(), 2);
    }

    #[test]
    fn easing_only_moves_when_animation_is_on() {
        let mut s = state();
        s.config.clock.animation = false;
        s.tick(ClockTime::hms(10, 9, 31));
        assert!(!s.easing());
        let jumped = s.view_model();
        assert_eq!(jumped.surfaces["DP-2"].second.as_ref().unwrap().angle, Angles::at(ClockTime::hms(10, 9, 31)).second);

        s.config.clock.animation = true;
        s.tick(ClockTime::hms(10, 9, 32));
        assert!(s.easing());
        let start = s.view_model();
        assert_eq!(start.surfaces["DP-2"].second.as_ref().unwrap().angle, Angles::at(ClockTime::hms(10, 9, 31)).second);
        let generation = s.ease_generation;
        s.set_ease(generation - 1, 0.9);
        assert_eq!(start, s.view_model(), "a frame from an older burst is ignored");
        s.set_ease(generation, 0.5);
        let mid = s.view_model();
        assert_ne!(start, mid);
        s.set_ease(generation, 1.0);
        assert_eq!(s.view_model().surfaces["DP-2"].second.as_ref().unwrap().angle, Angles::at(ClockTime::hms(10, 9, 32)).second);
    }

    #[test]
    fn snap_never_eases() {
        let mut s = state();
        s.config.clock.animation = true;
        s.snap(ClockTime::hms(11, 0, 0));
        assert!(!s.easing());
        assert_eq!(s.view_model().surfaces["DP-2"].hour.angle, Angles::at(ClockTime::hms(11, 0, 0)).hour);
    }

    #[test]
    fn covered_monitor_does_not_change_while_easing_seconds() {
        let mut s = state();
        s.config.clock.animation = true;
        s.set_fullscreen_monitors(&["DP-2".to_string()]);
        s.tick(ClockTime::hms(10, 9, 32));
        let a = s.view_model();
        s.set_ease(s.ease_generation, 0.5);
        let b = s.view_model();
        assert_eq!(a.surfaces["DP-2"], b.surfaces["DP-2"]);
        assert_ne!(a.surfaces["DP-3"], b.surfaces["DP-3"]);
    }

    #[test]
    fn removed_monitor_disappears() {
        let mut s = state();
        s.monitor_removed("DP-3");
        assert_eq!(s.view_model().surfaces.keys().collect::<Vec<_>>(), vec!["DP-2"]);
    }
}
