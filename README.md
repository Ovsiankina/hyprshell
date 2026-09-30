# Hyprshell

The Hyprcustom widget and shell system

## Features

- Menubar (top,bottom,left,right)
- apps as wallpaper
- app picker
- Latest Hyprshell api call status displayed in the background... "baked in the
    wallpaper" (can be moved, like the weather widget of end4)
- Customizability in a dedicated GUI menu, and a proper, beautifully made, TOML for simplicity.
- vim keybinds as the default, with a dedicated configurable super key
- Widgets made with simple descriptors, to allow fast, easy style switch
- Style picker (like color picker)
- Color wheel with color theory settings (triade, complements, etc) available for quick change, like in zen-browser
- side panels
- Dashboard
- Ascii prerender and shader support
- Inherits Hyprland's styling (borders, rounding, gaps, colors) when applicable
- `hyprshellctl` CLI, so Hyprland binds can drive the shell
    (`bind = SUPER, space, exec, hyprshellctl toggle picker`)

## Architecture

Customizability, type safety, fast data structures are important.

### Ideas

- Workspace monorepo, one crate per responsibility:
    - `proto`: shared types (state, events, requests)
    - `api`: daemon, the *only* thing talking to Hyprland IPC
      (`.socket.sock` for queries, `.socket2.sock` for events).
      Broadcasts typed events to every frontend over its own socket.
    - `ctl`: `hyprshellctl`, thin client of the daemon (like `hyprctl`)
    - `config`: TOML schema + defaults
    - `style`: palettes, color theory, themes (pure functions)
    - `ascii`: glyph atlas + WGSL shaders, renderer-agnostic
    - `layershell`: Wayland layer-shell <-> renderer bridge
    - `ui`: the frontend
- Hyprland styling is read at runtime through IPC (`getoption`), not by parsing
    its config files: always the final values, whatever the config language.
- Rule: `proto`, `api`, `config`, `style` never depend on a renderer, so the
    frontend choice stays reversible.
- Frontend candidate: Bevy (ECS, shaders, `bevy_reflect` for the config GUI) on
    a custom layer-shell bridge. Proven with a clock prototype. Open risks:
    idle power (needs on-demand rendering) and text input (app picker).

