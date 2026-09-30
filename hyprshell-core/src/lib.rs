//! Hyprshell core: config, Hyprland IPC, state and view-model.
//!
//! No iced, no wgpu, no display needed. Everything here is unit-testable.

pub mod clock;
pub mod color;
pub mod config;
pub mod ctl;
pub mod hypr;
pub mod state;
pub mod view;

pub use color::Rgba;
pub use config::Config;
pub use state::State;
pub use view::{ClockView, TickRate, ViewModel};
