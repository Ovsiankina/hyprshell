//! hyprshell: one layer-shell surface per monitor with a shader-drawn clock.
//!
//! Data flow: events -> `State` -> `ViewModel` (plain data) -> per-surface
//! diff -> only surfaces whose view changed are redrawn.

mod clock;
mod subscriptions;

use hyprshell_core::clock::ClockTime;
use hyprshell_core::config::{Anchor as Position, Config};
use hyprshell_core::ctl::Command;
use hyprshell_core::hypr;
use hyprshell_core::state::State;
use hyprshell_core::view::{ClockView, ViewModel};
use iced::widget::shader;
use iced::{Color, Element, Length, Subscription, Task, window};
use iced_exwlshell::redraw::Scope;
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, LayerSize, NewLayerShellSettings, OutputOption,
};
use iced_exwlshell::settings::{LayerShellSettings, Settings, StartMode};
use iced_exwlshell::to_layer_message;
use iced_wayland_subscriber::shell::{ShellEvent, ShellReceiver};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

const NAMESPACE: &str = "hyprshell";

fn main() -> iced_exwlshell::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("hyprshell=info,hyprshell_core=info")),
        )
        .init();

    // No text input anywhere: skip the always-on clipboard worker thread.
    iced_exwlshell::disable_clipboard();
    let (shell_broadcast, shell_events) = iced_exwlshell::shell::channel();
    let config_path = Config::default_path();

    iced_exwlshell::daemon(
        move || App::boot(config_path.clone(), shell_events.clone()),
        NAMESPACE,
        App::update,
        App::view,
    )
    .style(App::style)
    .subscription(App::subscription)
    .redraw_scope(App::redraw_scope)
    .settings(Settings {
        layer_settings: LayerShellSettings {
            // No surface at boot; one is opened per output as outputs appear.
            start_mode: StartMode::Background,
            ..Default::default()
        },
        shell_broadcast,
        ..Default::default()
    })
    .run()
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Shell(ShellEvent),
    WindowClosed(window::Id),
    ConfigLoaded(Result<Config, String>),
    Hypr(hypr::Event),
    HyprDisconnected,
    HyprDefaults(hypr::Defaults),
    FullscreenMonitors(Option<Vec<String>>),
    Ctl(Command),
    Tick,
    /// One frame of the eased hand movement after a tick, progress `0..=1`,
    /// tagged with the tick generation that started the burst.
    Ease { generation: u64, progress: f32 },
    /// Emitted by `reconcile` for every surface whose view changed; the only
    /// message the redraw scope maps to a window.
    Redraw(window::Id),
}

struct App {
    state: State,
    vm: ViewModel,
    config_path: PathBuf,
    shell_events: ShellReceiver,
    /// Connected outputs: wl_registry global name -> output name.
    outputs: BTreeMap<u32, String>,
    /// Mapped surfaces by output name.
    windows: BTreeMap<String, window::Id>,
    names: BTreeMap<window::Id, String>,
}

impl App {
    fn boot(config_path: PathBuf, shell_events: ShellReceiver) -> (Self, Task<Message>) {
        let config = match subscriptions::load(&config_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("{e}");
                Config::default()
            }
        };
        let state = State::new(config, hypr::Defaults::default(), ClockTime::now());
        let vm = state.view_model();
        let app = Self {
            state,
            vm,
            config_path,
            shell_events,
            outputs: BTreeMap::new(),
            windows: BTreeMap::new(),
            names: BTreeMap::new(),
        };
        (app, fetch_defaults())
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Shell(ShellEvent::OutputAdded(info)) => {
                let name = info.name.clone().unwrap_or_else(|| format!("output-{}", info.id));
                tracing::info!(
                    "output added: {name} (global {}) logical {:?} scale {}",
                    info.id,
                    info.logical_size,
                    info.scale_factor
                );
                self.outputs.insert(info.id, name.clone());
                self.state.monitor_added(&name);
                Task::batch([self.reconcile(), query_fullscreen()])
            }
            Message::Shell(ShellEvent::OutputRemoved(info)) => {
                if let Some(name) = self.outputs.remove(&info.id) {
                    tracing::info!("output removed: {name} (global {})", info.id);
                    // A re-enabled monitor keeps its name under a new global;
                    // only forget the monitor when no global carries it.
                    if !self.outputs.values().any(|n| *n == name) {
                        self.state.monitor_removed(&name);
                    }
                }
                self.reconcile()
            }
            Message::Shell(_) => Task::none(),
            Message::WindowClosed(id) => {
                // The compositor closes a layer surface when its output goes
                // away. Forget the output too, so the next tick does not
                // reopen a surface on a dead global; OutputAdded brings it back.
                if let Some(name) = self.names.remove(&id) {
                    tracing::info!("surface for {name} closed by the compositor");
                    self.windows.remove(&name);
                    self.outputs.retain(|_, n| *n != name);
                    self.state.monitor_removed(&name);
                    self.vm = self.state.view_model();
                }
                Task::none()
            }
            Message::ConfigLoaded(result) => {
                match &result {
                    Ok(_) => tracing::info!("config reloaded from {}", self.config_path.display()),
                    Err(e) => tracing::error!("{e}"),
                }
                self.state.set_config(result);
                self.reconcile()
            }
            Message::Hypr(event) => match event {
                hypr::Event::ConfigReloaded => fetch_defaults(),
                hypr::Event::Fullscreen(_) | hypr::Event::Workspace { .. } => query_fullscreen(),
                hypr::Event::MonitorAdded { name, .. } => {
                    tracing::debug!("hyprland: monitor {name} added");
                    Task::none()
                }
                hypr::Event::MonitorRemoved { name, .. } => {
                    tracing::debug!("hyprland: monitor {name} removed");
                    Task::none()
                }
                hypr::Event::Other { .. } => Task::none(),
            },
            Message::HyprDisconnected => {
                tracing::error!("hyprland event socket closed; fullscreen and reload events are gone");
                Task::none()
            }
            Message::HyprDefaults(defaults) => {
                tracing::info!("hyprland defaults: {defaults:?}");
                self.state.hypr = defaults;
                self.reconcile()
            }
            Message::FullscreenMonitors(Some(names)) => {
                self.state.set_fullscreen_monitors(&names);
                self.reconcile()
            }
            Message::FullscreenMonitors(None) => Task::none(),
            Message::Ctl(Command::Reload) => {
                tracing::info!("hyprshellctl reload");
                let result = subscriptions::load(&self.config_path);
                if let Err(e) = &result {
                    tracing::error!("{e}");
                }
                self.state.set_config(result);
                Task::batch([fetch_defaults(), self.reconcile()])
            }
            Message::Ctl(Command::Toggle) => {
                self.state.toggle();
                tracing::info!("hyprshellctl toggle -> visible={}", self.state.visible);
                if self.state.visible {
                    self.state.snap(ClockTime::now());
                }
                self.reconcile()
            }
            Message::Tick => {
                self.state.tick(ClockTime::now());
                let burst = if self.state.easing() {
                    ease_burst(self.state.ease_generation)
                } else {
                    Task::none()
                };
                Task::batch([self.reconcile(), burst])
            }
            Message::Ease { generation, progress } => {
                self.state.set_ease(generation, progress);
                self.reconcile()
            }
            Message::Redraw(_) => Task::none(),
            // Injected by `to_layer_message`; the runtime consumes them.
            _ => Task::none(),
        }
    }

    /// Re-derive the view-model and turn the difference into surface actions:
    /// open, close, re-layout, or redraw exactly the surfaces that changed.
    fn reconcile(&mut self) -> Task<Message> {
        let new = self.state.view_model();
        let mut tasks = Vec::new();

        for (name, id) in self.windows.clone() {
            if !new.surfaces.contains_key(&name) {
                tracing::debug!("closing surface on {name}");
                self.windows.remove(&name);
                self.names.remove(&id);
                tasks.push(window::close(id));
            }
        }

        for (name, view) in &new.surfaces {
            match self.windows.get(name) {
                None => {
                    let Some(global) = self.outputs.iter().find(|(_, n)| *n == name).map(|(g, _)| *g) else {
                        continue;
                    };
                    let id = window::Id::unique();
                    tracing::debug!("opening surface on {name} as {id}");
                    self.windows.insert(name.clone(), id);
                    self.names.insert(id, name.clone());
                    tasks.push(Task::done(Message::NewLayerShell {
                        settings: NewLayerShellSettings {
                            size: LayerSize::px(view.size, view.size),
                            layer: Layer::Bottom,
                            anchor: layer_anchor(view.anchor),
                            // Protocol default 0: sit inside other surfaces'
                            // exclusive zones (bars), never over them.
                            exclusive_zone: None,
                            margin: Some(layer_margin(view.anchor, view.margin)),
                            keyboard_interactivity: KeyboardInteractivity::None,
                            output_option: OutputOption::GlobalName(global),
                            // Empty input region: the pointer never enters,
                            // so mouse motion never wakes the loop.
                            events_transparent: true,
                            namespace: Some(NAMESPACE.to_string()),
                            ..Default::default()
                        },
                        id,
                    }));
                }
                Some(&id) => {
                    let old = self.vm.surfaces.get(name);
                    if old == Some(view) {
                        continue;
                    }
                    if old.is_none_or(|o| (o.size, o.anchor, o.margin) != (view.size, view.anchor, view.margin)) {
                        tasks.push(Task::done(Message::LayoutChange {
                            id,
                            anchor: layer_anchor(view.anchor),
                            size: LayerSize::px(view.size, view.size),
                        }));
                        tasks.push(Task::done(Message::MarginChange {
                            id,
                            margin: layer_margin(view.anchor, view.margin),
                        }));
                    }
                    tasks.push(Task::done(Message::Redraw(id)));
                }
            }
        }

        self.vm = new;
        Task::batch(tasks)
    }

    fn view(&self, id: window::Id) -> Element<'_, Message> {
        match self.names.get(&id).and_then(|name| self.vm.surfaces.get(name)) {
            Some(view) => shader(clock::ClockProgram::new(view.clone()))
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => iced::widget::space::horizontal().into(),
        }
    }

    fn style(&self, theme: &iced::Theme) -> iced::theme::Style {
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: theme.palette().text,
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            self.shell_events.listen().map(Message::Shell),
            window::close_events().map(Message::WindowClosed),
            subscriptions::config_file(self.config_path.clone()),
            subscriptions::hyprland_events(),
            subscriptions::ctl_server(),
            subscriptions::ticks(self.vm.tick),
        ])
    }

    /// Only `Redraw(id)` paints, and only that surface. Everything else is
    /// bookkeeping: it may *produce* a `Redraw` but never paints by itself.
    fn redraw_scope(message: &Message) -> Scope {
        match message {
            Message::Redraw(id) => Scope::Window(*id),
            _ => Scope::None,
        }
    }
}

/// The eased movement: a fixed number of frames on a timer, never a frame loop.
const EASE_FRAMES: u32 = 8;
const EASE_FRAME_TIME: std::time::Duration = std::time::Duration::from_millis(25);

fn ease_burst(generation: u64) -> Task<Message> {
    Task::stream(iced::stream::channel(EASE_FRAMES as usize, async move |mut out| {
        use iced::futures::SinkExt;
        for frame in 1..=EASE_FRAMES {
            tokio::time::sleep(EASE_FRAME_TIME).await;
            let progress = frame as f32 / EASE_FRAMES as f32;
            if out.send(Message::Ease { generation, progress }).await.is_err() {
                return;
            }
        }
    }))
}

fn fetch_defaults() -> Task<Message> {
    Task::perform(
        async { tokio::task::spawn_blocking(hypr::Defaults::fetch).await.unwrap_or_default() },
        Message::HyprDefaults,
    )
}

fn query_fullscreen() -> Task<Message> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(|| {
                let monitors = hypr::monitors()?;
                let workspaces = hypr::workspaces()?;
                Ok::<_, hypr::HyprError>(hypr::fullscreen_monitors(&monitors, &workspaces))
            })
            .await
            .ok()
            .and_then(|r| r.map_err(|e| tracing::warn!("fullscreen query: {e}")).ok())
        },
        Message::FullscreenMonitors,
    )
}

fn layer_anchor(position: Position) -> Anchor {
    match position {
        Position::TopLeft => Anchor::Top | Anchor::Left,
        Position::Top => Anchor::Top,
        Position::TopRight => Anchor::Top | Anchor::Right,
        Position::Left => Anchor::Left,
        Position::Center => Anchor::empty(),
        Position::Right => Anchor::Right,
        Position::BottomLeft => Anchor::Bottom | Anchor::Left,
        Position::Bottom => Anchor::Bottom,
        Position::BottomRight => Anchor::Bottom | Anchor::Right,
    }
}

/// Margin only on the anchored edges, in protocol order (top, right, bottom, left).
fn layer_margin(position: Position, margin: u32) -> (i32, i32, i32, i32) {
    let anchor = layer_anchor(position);
    let m = margin as i32;
    let on = |edge: Anchor| if anchor.contains(edge) { m } else { 0 };
    (on(Anchor::Top), on(Anchor::Right), on(Anchor::Bottom), on(Anchor::Left))
}

#[allow(dead_code)]
fn _assert_view_is_plain_data(v: &ClockView) -> &ClockView {
    v
}
