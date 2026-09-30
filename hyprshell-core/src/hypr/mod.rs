//! Hyprland IPC client: JSON requests on `.socket.sock`, events on
//! `.socket2.sock`. Own tolerant serde structs: unknown fields are ignored and
//! missing ones default, so a Hyprland upgrade degrades instead of failing.
//!
//! Verified against Hyprland v0.56.2 source (`src/debug/HyprCtl.cpp`,
//! `src/managers/EventManager.cpp`).

pub mod event;

use crate::color::Rgba;
use serde::Deserialize;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

pub use event::{Event, EventReader};

#[derive(Debug, thiserror::Error)]
pub enum HyprError {
    #[error("HYPRLAND_INSTANCE_SIGNATURE is not set; is Hyprland running?")]
    NoInstance,
    #[error("XDG_RUNTIME_DIR is not set")]
    NoRuntimeDir,
    #[error("hyprland socket {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("hyprland reply to {request:?} is not the JSON we expect: {source}")]
    Json {
        request: String,
        #[source]
        source: serde_json::Error,
    },
}

/// `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/`.
pub fn instance_dir() -> Result<PathBuf, HyprError> {
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
        .filter(|s| !s.is_empty())
        .ok_or(HyprError::NoInstance)?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|s| !s.is_empty())
        .ok_or(HyprError::NoRuntimeDir)?;
    Ok(PathBuf::from(runtime).join("hypr").join(sig))
}

pub fn request_socket() -> Result<PathBuf, HyprError> {
    Ok(instance_dir()?.join(".socket.sock"))
}

pub fn event_socket() -> Result<PathBuf, HyprError> {
    Ok(instance_dir()?.join(".socket2.sock"))
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// One request, raw reply. `cmd` is what you would pass to `hyprctl`, e.g.
/// `"j/monitors"` or `"j/getoption general:border_size"`. Blocking.
pub fn request(cmd: &str) -> Result<String, HyprError> {
    let path = request_socket()?;
    let io = |source| HyprError::Io { path: path.clone(), source };
    let mut stream = UnixStream::connect(&path).map_err(io)?;
    stream.set_read_timeout(Some(REQUEST_TIMEOUT)).map_err(io)?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT)).map_err(io)?;
    stream.write_all(cmd.as_bytes()).map_err(io)?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).map_err(io)?;
    Ok(reply)
}

fn request_json<T: serde::de::DeserializeOwned>(cmd: &str) -> Result<T, HyprError> {
    let reply = request(cmd)?;
    serde_json::from_str(&reply).map_err(|source| HyprError::Json { request: cmd.to_string(), source })
}

/// `hyprctl -j getoption <name>`. Exactly one of the value fields is set,
/// depending on the option's type.
#[derive(Clone, PartialEq, Debug, Default, Deserialize)]
#[serde(default)]
pub struct OptionValue {
    pub option: String,
    pub int: Option<i64>,
    pub float: Option<f64>,
    pub str: Option<String>,
    /// Complex types print as a string. The legacy `.conf` config manager
    /// reports gradients under `custom`, the Lua manager under `gradient`.
    pub custom: Option<String>,
    pub gradient: Option<String>,
    pub set: bool,
}

impl OptionValue {
    /// The gradient string whichever key Hyprland used.
    pub fn gradient_str(&self) -> Option<&str> {
        self.gradient.as_deref().or(self.custom.as_deref())
    }
}

pub fn get_option(name: &str) -> Result<OptionValue, HyprError> {
    request_json(&format!("j/getoption {name}"))
}

/// The Hyprland options the shell uses as defaults. `None` when the option
/// could not be read or parsed; the config's own default applies then.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Defaults {
    /// First stop of `general:col.active_border`.
    pub active_border: Option<Rgba>,
    /// `general:border_size`.
    pub border_size: Option<u32>,
    /// `decoration:rounding`.
    pub rounding: Option<u32>,
}

impl Defaults {
    /// Three requests; each failure is logged and leaves that field `None`.
    pub fn fetch() -> Self {
        let int = |name: &str| match get_option(name) {
            Ok(v) => v.int.and_then(|i| u32::try_from(i).ok()),
            Err(e) => {
                tracing::warn!("getoption {name}: {e}");
                None
            }
        };
        let active_border = match get_option("general:col.active_border") {
            Ok(v) => v.gradient_str().and_then(|s| match Rgba::parse_hyprland_gradient(s) {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::warn!("cannot parse col.active_border {s:?}: {e}");
                    None
                }
            }),
            Err(e) => {
                tracing::warn!("getoption general:col.active_border: {e}");
                None
            }
        };
        Self {
            active_border,
            border_size: int("general:border_size"),
            rounding: int("decoration:rounding"),
        }
    }
}

/// One entry of `hyprctl -j monitors`.
#[derive(Clone, PartialEq, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    pub transform: i32,
    pub focused: bool,
    pub disabled: bool,
    #[serde(rename = "dpmsStatus")]
    pub dpms_status: bool,
    #[serde(rename = "activeWorkspace")]
    pub active_workspace: WorkspaceRef,
}

#[derive(Clone, PartialEq, Debug, Default, Deserialize)]
#[serde(default)]
pub struct WorkspaceRef {
    pub id: i64,
    pub name: String,
}

/// One entry of `hyprctl -j workspaces`.
#[derive(Clone, PartialEq, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
    pub monitor: String,
    /// `null` while a workspace has no monitor (mid-hotplug).
    #[serde(rename = "monitorID")]
    pub monitor_id: Option<i64>,
    pub windows: u32,
    pub hasfullscreen: bool,
}

pub fn monitors() -> Result<Vec<Monitor>, HyprError> {
    request_json("j/monitors")
}

pub fn workspaces() -> Result<Vec<Workspace>, HyprError> {
    request_json("j/workspaces")
}

/// Monitor names whose active workspace currently holds a fullscreen window.
/// Pure so it can be tested on canned JSON.
pub fn fullscreen_monitors(monitors: &[Monitor], workspaces: &[Workspace]) -> Vec<String> {
    monitors
        .iter()
        .filter(|m| {
            workspaces
                .iter()
                .any(|w| w.id == m.active_workspace.id && w.hasfullscreen)
        })
        .map(|m| m.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getoption_shapes() {
        let v: OptionValue =
            serde_json::from_str(r#"{"option": "general:col.active_border", "custom": "77919191 0deg", "set": true }"#)
                .unwrap();
        assert_eq!(v.gradient_str(), Some("77919191 0deg"));
        assert!(v.set);
        let v: OptionValue =
            serde_json::from_str(r#"{"option": "general:col.active_border", "gradient": "ff00ff00 -45deg", "set": true }"#)
                .unwrap();
        assert_eq!(v.gradient_str(), Some("ff00ff00 -45deg"));
        let v: OptionValue = serde_json::from_str(r#"{"option": "general:border_size", "int": 1, "set": true }"#).unwrap();
        assert_eq!(v.int, Some(1));
        // Unknown fields and a new type must not break us.
        let v: OptionValue = serde_json::from_str(r#"{"option": "x", "vec2": [1,2], "future": {}}"#).unwrap();
        assert_eq!(v.int, None);
        assert!(!v.set);
    }

    #[test]
    fn monitors_and_workspaces_tolerate_unknown_fields() {
        let ms: Vec<Monitor> = serde_json::from_str(
            r#"[{"id":0,"name":"DP-2","width":2560,"height":1440,"scale":1,"focused":true,
                 "activeWorkspace":{"id":1,"name":"1"},"availableModes":["x"],"newField":null}]"#,
        )
        .unwrap();
        assert_eq!(ms[0].name, "DP-2");
        assert_eq!(ms[0].active_workspace.id, 1);
        let ws: Vec<Workspace> = serde_json::from_str(
            r#"[{"id":1,"name":"1","monitor":"DP-2","monitorID":0,"hasfullscreen":true,"tiledLayout":"dwindle"},
                {"id":3,"name":"3","monitor":"DP-3","monitorID":1,"hasfullscreen":false},
                {"id":7,"name":"7","monitor":"?","monitorID":null,"hasfullscreen":false}]"#,
        )
        .unwrap();
        assert!(ws[0].hasfullscreen);
        assert_eq!(ws[2].monitor_id, None);
        let mut ms2 = ms.clone();
        ms2.push(Monitor { name: "DP-3".into(), active_workspace: WorkspaceRef { id: 3, name: "3".into() }, ..Default::default() });
        assert_eq!(fullscreen_monitors(&ms2, &ws), vec!["DP-2".to_string()]);
    }
}
