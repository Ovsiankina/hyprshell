# Hyprshell

Rust shell for Hyprland. `README.md` is the feature wish list. This file is the
plan of record, revised 2026-09-25 after a design review. `docs/architecture.html`
is the pre-review design and is stale; do not follow it where it conflicts with
this file.

## Decisions (made by the user, not up for re-litigation)

- **Rust, from existing crates.** Quickshell, eww and AGS are rejected: dynamic
  typing, poor docs, no clean Hyprland IPC, painful C++ rebuilds. Do not propose them.
- **iced via `iced_layershell` is the UI toolkit.** No custom wgpu scene, no
  sctk/calloop loop, no Bevy. Use what iced gives: its loop, its renderer, its
  `shader` widget, its text input. Custom GPU code lives only inside shader widgets.
- **One process for now.** State, Hyprland IPC, config and UI in one binary.
  Keep non-UI code GPU-free and testable without a display; the split into a
  daemon is a later decision, if ever.
- **Config is one TOML file.** Typed values with validation, hand-editable, and a
  future GUI edits the same file with `toml_edit` so comments and formatting
  survive. Nothing is configured in code.
- **Vim-style binds are ordinary Hyprland binds** in the user's own config, calling
  `hyprshellctl`. The shell does not inject binds.
- **Out of scope until the POC works:** color wheel, style picker, dashboard, side
  panels, app picker, notifications, apps-as-wallpaper (hyprwinwrap), daemon split,
  plugins, occlusion-based animation gating, SIGSTOP pausing.

### Decisions made while building the POC (2026-09-25)

- **Versions:** `iced 0.14.0` with `iced_exwlshell 0.20.1` (+ `iced_wayland_subscriber
  0.20.1`, `wgpu 27` to match `iced_wgpu 0.14`). `iced_layershell` was renamed to
  `iced_exwlshell` in 0.20.0 (2026-08-29); 0.19.1 is the last release under the
  old name. 0.20 was chosen over 0.19.1 because it adds output add/remove events
  (hotplug) and a per-message redraw policy (idle cost), both POC risks.
- **No `hyprland-rs`.** Own tolerant serde structs over `.socket.sock` JSON and a
  line parser for `.socket2.sock`; less code than isolating a beta crate.
- **Config path override:** `$HYPRSHELL_CONFIG` wins over
  `$XDG_CONFIG_HOME/hyprshell/hyprshell.toml`. Needed because this repo is
  `dotfiles/dots/.config/hyprshell`, not yet stowed to `~/.config/hyprshell`.
  A missing file means built-in defaults, never a failure.
- **Bad config keeps the last good one.** A parse or validation error is logged
  (all problems at once, with line/column) and the running config stays.
  Unknown keys are errors, not silently ignored.
- **`animation` means eased hand movement** on each tick versus an instant jump.
  It is the "clock arm easing" nice-to-have; the flag is parsed from day one.
  The 1 Hz second-hand tick is controlled by `show_seconds`, not by `animation`.
- **Fullscreen per monitor is re-queried, not tracked.** Hyprland's `fullscreen>>0|1`
  event carries no monitor. On `fullscreen` and `workspacev2` the shell asks
  `j/monitors` + `j/workspaces` and marks a monitor covered when its active
  workspace has `hasfullscreen`. A covered monitor's clock loses its second
  hand (nobody can see it) and its view-model stops changing, so it is not
  redrawn until uncovered.
- **`decoration:rounding` is the clock face's corner radius.** The face is a
  rounded square that becomes a circle at `size / 2`; config `clock.face.rounding`
  overrides. This is how "inherits Hyprland rounding" shows up on a clock.
- **Hyprland gradient colors** print as `AARRGGBB` hex per stop plus an angle
  (`"77919191 0deg"`); the first stop is used as the border color.
- **Hidden means unmapped.** `hyprshellctl toggle` closes every surface and
  reopens them; no transparent surface is left composited.
- **Surfaces are keyed by Wayland output name**, which equals Hyprland's monitor
  name (`DP-2`), so IPC results map onto surfaces without an id table.
- **`animation` defaults to off.** Easing is 8 timer-paced frames over 200 ms
  after every tick (8x the GPU submissions with seconds on), so it is opt-in.
- **GPU backend is wgpu's default (Vulkan).** `WGPU_BACKEND=gl` cuts idle
  wakeups from ~245/s to ~1/s on the NVIDIA driver; see the measurements. Not
  made a config option yet because it is a driver quirk, not a style choice.
- **Bad Hyprland events are dropped in the reader thread.** Only the events the
  shell reacts to cross into the iced loop; every other event would otherwise
  wake the loop and rebuild every surface's widget tree.
- **A compositor-closed surface takes its output with it.** When Hyprland closes
  a layer surface (`WindowClosed`), the shell forgets that output until the
  compositor announces it again, instead of reopening on a possibly dead global.

## The POC (build this first, nothing else)

A single layer-shell surface per monitor showing an analog clock drawn by a
custom shader, driven by Hyprland IPC, styled from a TOML file.

Must have:

1. Layer surface on every output, `bottom` layer, anchored where config says.
   Works on the user's dual-monitor setup, survives monitor hotplug, and handles
   fractional scaling.
2. Clock widget rendered through an iced `shader` widget: face, hour, minute,
   second hands. Simple shader effects allowed (glow, gradient), all cheap.
3. Config in `~/.config/hyprshell/hyprshell.toml`: colors, border width, hand
   lengths and widths, size, position, whether seconds are shown, animation on/off.
   Reload on file change and apply live.
4. Hyprland IPC: read `general:col.active_border`, `general:border_size`,
   `decoration:rounding` through `getoption` and use them as defaults when the
   TOML does not override. Listen on socket2; on `configreloaded` refresh
   options, on `fullscreen` stop the seconds animation on that monitor.
5. `hyprshellctl` with two commands: `reload` and `toggle` (show/hide surfaces).
   Unix socket under `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/`.
6. Idle cost near zero: no seconds shown means one redraw per minute aligned to
   the wall clock; seconds shown means a 1 Hz timer, not a frame loop. Animation
   is paced by a timer, never by frame-callback parity.

Nice to have, only after the above works: clock arm easing, a second widget
(text label bound to a state path) to prove the descriptor idea.

## Architecture for the POC

Cargo workspace, three crates:

- `hyprshell-core` (lib): config schema and loader, Hyprland IPC client, state
  struct, view-model types. No iced, no wgpu. Unit-testable.
- `hyprshell` (bin): iced_layershell app. Subscriptions for Hyprland events,
  config file watch, timer, ctl socket. Widgets. Shaders as WGSL files included
  with `include_str!`.
- `hyprshellctl` (bin): connects, sends one line, prints reply.

Data flow: events (Hyprland, config, timer, ctl) -> update `State` -> derive a
`ViewModel` (plain data, `PartialEq`) -> if unchanged, do nothing -> else build
the iced element tree from it. The diff and the skip happen on the view-model,
never on the iced tree. Snapshot-test the view-model.

Hyprland IPC: own tolerant serde structs (unknown fields ignored, defaults) over
`hyprctl -j`-style JSON on `.socket.sock`, and a line parser for the handful of
`.socket2.sock` events used. `hyprland-rs` may be used if it builds against the
installed version; isolate it behind one module so it can be swapped.

## Environment

- Hyprland 0.56.2 (tag, not main). Every upstream claim must be checked against
  that tag.
- Dual-monitor setup where a previous layer-shell tool (the lockscreen) failed to
  render on either output. Treat multi-output as the primary risk and test it first.
- The user runs blur and transparency, so geometry-based occlusion is never a
  visibility signal.

Verified on the v0.56.2 tag (Renderer.cpp:1161-1248, 2204-2210, Monitor.cpp:1858-1938):
render order is `background`, `bottom`, windows, `top`, `overlay`. Frame events
reach every mapped view only on idle frames; while an opaque fullscreen window
keeps damaging the screen ("solitary" mode) bottom layers are neither rendered
nor sent frame callbacks. Details under "Facts verified".

## Facts verified during the POC (2026-09-25, Hyprland 0.56.2, iced_exwlshell 0.20.1)

- **Multiple outputs:** works. The runtime (`daemon` builder, `StartMode::Background`)
  replays `ShellEvent::OutputAdded` for every connected output to a late
  subscriber, then streams live events. One `NewLayerShell` per output with
  `OutputOption::GlobalName(info.id)` puts the surface on that output. Verified
  on DP-2 + DP-3 plus a headless third output.
- **Hotplug:** `hyprctl output create headless X` / `output remove X` go through
  the same `CMonitor::onConnect/onDisconnect` path as a physical plug/unplug
  (Monitor.cpp:202-213, 258-286, 392-401 on v0.56.2). Add: `OutputAdded` arrives,
  surface opens in ~5 ms. Remove: `OutputRemoved` arrives, surface is closed.
  A physical unplug was not performed; it is the same compositor path.
- **Per-output config:** each surface gets its own `NewLayerShellSettings`, and
  `LayoutChange{id,anchor,size}` + `MarginChange{id,margin}` re-lay out one
  surface live (used on config reload). Margin tuple order is protocol order
  `(top, right, bottom, left)`; the macro doc comment saying top,left,bottom,right
  is wrong.
- **IME / text input:** not needed by the POC, not verified. `disable_clipboard()`
  is called so no clipboard worker thread runs.
- **Fractional scale + viewporter on layer surfaces:** yes. exwlshellev binds
  `wp_fractional_scale_v1` and `wp_viewporter` on every layer surface and
  `set_destination(logical)`; Hyprland 0.56.2 sends `preferred_scale` to layer
  surfaces on map and commit (LayerSurface.cpp:439-463). Verified: a headless
  output at scale 2 rendered at `@2`, at 1.25 rendered at `@1.25`, surface
  geometry stays in logical pixels.
- **Opaque regions:** Hyprland uses `wl_surface.set_opaque_region` only for
  occlusion culling of what is *below* (Pass.cpp:61-86); blur is decided by a
  layerrule or the background-effect protocol, never by the opaque region. A
  transparent clock with no rule costs one alpha blend of 220x220 px and no blur.
- **Zero GPU submissions when idle:** confirmed from source and measurement.
  exwlshellev renders only when a unit's refresh flag is `NextFrame`/`At` and a
  present slot is free; the loop blocks with `timeout = None` otherwise
  (exwlshellev lib.rs:909-953, 1139-1149, 3636-3642). After a present it requests
  one frame callback only to gate the next present; the callback never requests a
  render. Pointer events run `update` but never render unless a widget asks.
  Two things do force redraws of every surface and are disabled: the
  `linux-theme-detection` feature (mundy signal -> `request_refresh_all`) and the
  `unconditional-rendering` feature.
- **Redraw scope:** `Daemon::redraw_scope` decides per *message* before `update`
  runs, so the app maps every message to `Scope::None` and has `reconcile()`
  emit `Redraw(id)` (scope `Window(id)`) only for surfaces whose `ClockView`
  changed. `view()` is still rebuilt for every window on every message (CPU only).
- **`iced::time::every` does not exist** with iced's default `thread-pool`
  executor; the `tokio` feature is required for any timer. It is not wall-clock
  aligned anyway, so the shell sleeps until the next second/minute boundary itself.
- **Bottom layer under fullscreen:** with an opaque fullscreen window and no
  top/overlay layers, Hyprland enters "solitary" mode and neither renders bottom
  layers nor sends them frame callbacks while the window keeps damaging. A pending
  redraw then stalls (does not spin) until a callback arrives. The per-monitor
  fullscreen re-query removes the second hand on covered monitors so no redraw is
  pending there. Frame events are still sent on idle frames via
  `sendFrameEventsToWorkspace`; the earlier note "regardless of fullscreen" is
  only true for that idle path.
- **Same-level z-order is mapping order.** The current shell (quickshell) puts
  its wallpaper on the `bottom` layer too. On the physical monitors hyprshell
  mapped later and draws on top; on a hotplugged output quickshell mapped later
  and covered the clock. This goes away once hyprshell owns the wallpaper; until
  then a hotplugged output may hide the clock behind quickshell's background.
- **Surface format:** `Rgba8Unorm`, alpha mode `PreMultiplied`, adapter NVIDIA
  RTX 5070 via Vulkan. The shader writes premultiplied colors.
- **Hyprland IPC:** request = raw bytes, `j/` prefix for JSON, reply until EOF,
  `[[BATCH]]a;b` supported (replies joined by `\n\n\n`). Event lines
  `EVENT>>DATA\n`, data cut at 1024 bytes, a client 64 events behind is dropped
  (the reader reconnects). `fullscreen>>0|1` carries no window or monitor.
  `getoption` reports a gradient under `custom` (legacy `.conf` manager) or
  `gradient` (Lua manager); both are parsed. Colors are `AARRGGBB` hex without
  zero padding, followed by `<int>deg`.
- **Idle cost, measured 2026-09-25** on DP-2 + DP-3, one 220 px clock each.
  GPU submissions are counted from the `hyprshell::gpu` debug log line (one per
  `prepare`, i.e. one per `queue.submit`). CPU is the process's utime+stime.
  `nvidia-smi` numbers are for the whole GPU while a browser was playing video
  on screen, so they are an upper bound, not the clock's cost.

  | build   | mode                     | window | GPU submissions | CPU     | RSS    | GPU util / power (whole card) |
  |---------|--------------------------|--------|-----------------|---------|--------|-------------------------------|
  | debug   | seconds on               | 60 s   | 120 (2/s)       | 220 ms  | 329 MB | 18.2 % / 40.7 W               |
  | debug   | seconds off              | 120 s  | 6 (2 minute marks + reload) | 170 ms | 329 MB | 12.8 % / 37.9 W    |
  | debug   | hidden (`toggle`)        | 60 s   | 0               | 40 ms   | 329 MB | 9.4 % / 35.6 W                |
  | release | seconds on               | 60 s   | 120 (2/s)       | 80 ms   | 311 MB | -                             |
  | release | seconds on + `animation` | 60 s   | 960 (8 per tick per monitor) | 230 ms | 311 MB | -               |

  Seconds off is one redraw per monitor per minute, on the minute; hidden is
  zero GPU work. RSS is mostly the NVIDIA Vulkan driver (36 `/dev/nvidia0` fds,
  33 threads).
- **Wakeup attribution** (release build, 10 s probes, context switches per second):

  | backend | state              | main thread | whole process | threads |
  |---------|--------------------|-------------|---------------|---------|
  | Vulkan  | visible, seconds on| 6.5         | 269           | 33      |
  | Vulkan  | hidden             | 0.0         | 245           | 33      |
  | GL      | visible, seconds on| 5.3         | 15            | 28      |
  | GL      | hidden             | 0.2         | 1.0           | 28      |

  The shell's own loop is idle-clean: the main thread sits in `epoll_wait` and
  does not wake at all while hidden. The ~245/s residual is NVIDIA's Vulkan
  driver threads (`[vkps]`, `[vkcf]`, ...), present even with no surface.
  `WGPU_BACKEND=gl` avoids them entirely; rendering was identical. Whether to
  default to GL on NVIDIA is an open decision; the POC leaves wgpu's default
  (Vulkan) and documents the env var.
- **Wayland traffic at steady state** (`WAYLAND_DEBUG=1`): exactly 12 incoming
  events per second with seconds on, all belonging to the two presents
  (presentation feedback, frame callback, buffer release, delete_id). No pointer
  events reach the surfaces (empty input region).

## What did not work, and the workarounds (POC build, 2026-09-25)

- `iced_layershell` no longer exists at a current version; the crate is
  `iced_exwlshell` 0.20. Same authors, same code, renamed 2026-08-29.
- `iced::time::every` is absent without the `tokio`/`smol` feature and is not
  wall-clock aligned. Workaround: `tokio` feature plus own `ticks` subscription
  that sleeps until the next second/minute boundary.
- The redraw scope is decided per message *before* `update` runs, so the diff
  result cannot feed it directly. Workaround: `reconcile()` emits a `Redraw(id)`
  message per changed surface and only that message maps to `Scope::Window`.
- A shell action (`LayoutChange`, `close`) for a window id the runtime already
  dropped is re-queued and retried on every loop pass forever
  (`iced_exwlshell` `multi_window.rs:858-862`). Only reachable in the few
  milliseconds between a compositor-side close and our `WindowClosed`; the shell
  forgets the output on `WindowClosed` to keep that window as small as possible.
  Upstream issue material.
- The published `iced_layershell`/`iced_exwlshell` crates ship no examples; the
  repo tag was cloned to read them. No `custom_shader` example exists for
  iced 0.14 either; the `Primitive`/`Pipeline` split was taken from the traits.
- quickshell (the current shell) keeps its wallpaper on the `bottom` layer, so
  on a hotplugged output it maps after hyprshell and hides the clock. Not fixable
  from this side without moving to a different layer; `toggle` twice re-maps on top.
- strace is blocked (`ptrace_scope = 1`), so wakeups were attributed with
  `/proc/<pid>/task/*/status` deltas, `wchan` sampling and `WAYLAND_DEBUG=1`.
- A physical monitor unplug was not performed (nobody at the machine); the
  headless output add/remove takes the same compositor path.
- Accepted: when a monitor becomes covered by a fullscreen window its minute
  hand snaps back to the whole minute (up to 6 degrees) because that surface
  stops receiving seconds; nobody can see it at that moment.

## Style rules for this repo

- Idle means zero work. Never poll. Never request frames without an animation.
- One surface per output per role. Never one surface per widget.
- Config values are the single source of truth; Hyprland options are defaults only.
- Prefer a crate that exists and is maintained over writing it. Prefer deleting
  a feature over designing it twice.
- Keep this file current: when a decision is made or a fact verified, edit it here.
