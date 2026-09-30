//! Everything the shell knows, in one plain struct. Events mutate it; the
//! view-model is derived from it. No I/O here.

use crate::clock::ClockTime;
use crate::config::Config;
use crate::hypr;
use crate::view::ViewModel;
use std::collections::BTreeMap;

#[derive(Clone, PartialEq, Debug)]
pub struct State {
    pub config: Config,
    /// The last config load error, shown in logs; the last good config stays
    /// in `config` so a typo never blanks the screen.
    pub config_error: Option<String>,
    pub hypr: hypr::Defaults,
    /// `hyprshellctl toggle` flips this.
    pub visible: bool,
    /// Wall-clock time as of the last tick, truncated to the tick granularity.
    pub time: ClockTime,
    /// Time as of the tick before, where an eased hand movement starts.
    pub prev_time: ClockTime,
    /// Easing progress from `prev_time` to `time`, `0.0..=1.0`. Jumps straight
    /// to `1.0` when `clock.animation` is off.
    pub ease: f32,
    /// Incremented by every tick; frames from an older burst are ignored.
    pub ease_generation: u64,
    /// Connected outputs by Wayland output name (matches Hyprland's monitor
    /// name). Fed by the compositor, not by Hyprland events.
    pub monitors: BTreeMap<String, MonitorState>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MonitorState {
    /// The monitor's active workspace holds a fullscreen window: the clock is
    /// covered, so its second hand stops.
    pub fullscreen: bool,
}

impl State {
    pub fn new(config: Config, hypr: hypr::Defaults, time: ClockTime) -> Self {
        Self {
            config,
            config_error: None,
            hypr,
            visible: true,
            time,
            prev_time: time,
            ease: 1.0,
            ease_generation: 0,
            monitors: BTreeMap::new(),
        }
    }

    pub fn monitor_added(&mut self, name: &str) {
        self.monitors.entry(name.to_string()).or_default();
    }

    pub fn monitor_removed(&mut self, name: &str) {
        self.monitors.remove(name);
    }

    /// Replace the fullscreen flags from a fresh Hyprland query.
    pub fn set_fullscreen_monitors(&mut self, fullscreen: &[String]) {
        for (name, m) in &mut self.monitors {
            m.fullscreen = fullscreen.iter().any(|f| f == name);
        }
    }

    pub fn set_config(&mut self, result: Result<Config, String>) {
        match result {
            Ok(config) => {
                self.config = config;
                self.config_error = None;
            }
            Err(e) => self.config_error = Some(e),
        }
    }

    pub fn tick(&mut self, now: ClockTime) {
        self.prev_time = self.time;
        self.time = ClockTime { nanos: 0, ..now };
        self.ease = if self.config.clock.animation { 0.0 } else { 1.0 };
        self.ease_generation += 1;
    }

    /// Jump straight to `now` with no easing: used when the surfaces reappear,
    /// where sweeping from the time they were hidden would be wrong.
    pub fn snap(&mut self, now: ClockTime) {
        self.time = ClockTime { nanos: 0, ..now };
        self.prev_time = self.time;
        self.ease = 1.0;
        self.ease_generation += 1;
    }

    /// Whether the last tick started an eased movement that needs frames.
    pub fn easing(&self) -> bool {
        self.ease < 1.0
    }

    /// Apply one frame of a burst. A frame from an earlier burst (older
    /// generation) is dropped so two bursts can never fight.
    pub fn set_ease(&mut self, generation: u64, progress: f32) {
        if generation == self.ease_generation {
            self.ease = progress.clamp(0.0, 1.0);
        }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    pub fn view_model(&self) -> ViewModel {
        ViewModel::derive(self)
    }
}
