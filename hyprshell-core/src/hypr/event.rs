//! `.socket2.sock` line parser. Lines are `EVENT>>DATA\n`, data at most 1024
//! bytes with newlines replaced by spaces (EventManager.cpp:128). Nothing is
//! sent to the socket; Hyprland streams as soon as we connect.

use super::HyprError;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;

/// The events the shell reacts to. Everything else is `Other` so callers can
/// log it without us having to model Hyprland's whole vocabulary.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    /// Hyprland re-read its config; our option defaults may have changed.
    ConfigReloaded,
    /// A window's fullscreen state changed (`1` entered, `0` left). Hyprland
    /// does not say which monitor, so the caller re-queries.
    Fullscreen(bool),
    /// `workspacev2>>ID,NAME`: the focused monitor switched workspace, which
    /// changes whether it shows a fullscreen window.
    Workspace { id: i64, name: String },
    /// `monitoraddedv2>>ID,NAME,DESCRIPTION`.
    MonitorAdded { id: i64, name: String },
    /// `monitorremovedv2>>ID,NAME,DESCRIPTION`.
    MonitorRemoved { id: i64, name: String },
    Other { name: String, data: String },
}

impl Event {
    /// Parse one line without its trailing newline. `None` for malformed lines.
    pub fn parse(line: &str) -> Option<Event> {
        let (name, data) = line.split_once(">>")?;
        let id_name = || -> Option<(i64, String)> {
            let mut parts = data.splitn(3, ',');
            let id = parts.next()?.parse().ok()?;
            let name = parts.next()?.to_string();
            Some((id, name))
        };
        Some(match name {
            "configreloaded" => Event::ConfigReloaded,
            "fullscreen" => Event::Fullscreen(data.trim() == "1"),
            "workspacev2" => {
                let (id, name) = id_name()?;
                Event::Workspace { id, name }
            }
            "monitoraddedv2" => {
                let (id, name) = id_name()?;
                Event::MonitorAdded { id, name }
            }
            "monitorremovedv2" => {
                let (id, name) = id_name()?;
                Event::MonitorRemoved { id, name }
            }
            _ => Event::Other { name: name.to_string(), data: data.to_string() },
        })
    }
}

/// Blocking line reader over `.socket2.sock`. Iterate it on its own thread.
pub struct EventReader {
    lines: std::io::Lines<BufReader<UnixStream>>,
}

impl EventReader {
    pub fn connect() -> Result<Self, HyprError> {
        let path = super::event_socket()?;
        let stream = UnixStream::connect(&path).map_err(|source| HyprError::Io { path, source })?;
        Ok(Self { lines: BufReader::new(stream).lines() })
    }
}

impl Iterator for EventReader {
    type Item = std::io::Result<Event>;

    /// Ends when Hyprland closes the socket (it exited).
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.lines.next()? {
                Ok(line) => {
                    if let Some(ev) = Event::parse(&line) {
                        return Some(Ok(ev));
                    }
                    tracing::debug!("ignoring malformed hyprland event line {line:?}");
                }
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_events_we_use() {
        assert_eq!(Event::parse("configreloaded>>"), Some(Event::ConfigReloaded));
        assert_eq!(Event::parse("fullscreen>>1"), Some(Event::Fullscreen(true)));
        assert_eq!(Event::parse("fullscreen>>0"), Some(Event::Fullscreen(false)));
        assert_eq!(
            Event::parse("monitoraddedv2>>2,HEADLESS-1,"),
            Some(Event::MonitorAdded { id: 2, name: "HEADLESS-1".into() })
        );
        assert_eq!(
            Event::parse("monitorremovedv2>>0,DP-2,AOC AG276QZD2 2OMR3JA012634"),
            Some(Event::MonitorRemoved { id: 0, name: "DP-2".into() })
        );
        assert_eq!(Event::parse("workspacev2>>3,3"), Some(Event::Workspace { id: 3, name: "3".into() }));
        assert_eq!(
            Event::parse("activewindow>>kitty,~"),
            Some(Event::Other { name: "activewindow".into(), data: "kitty,~".into() })
        );
        assert_eq!(Event::parse("garbage"), None);
        assert_eq!(Event::parse("monitoraddedv2>>notanumber,DP-1,x"), None);
    }
}
