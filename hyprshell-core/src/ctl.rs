//! The `hyprshellctl` wire protocol: one line in, one line out, over a Unix
//! socket next to Hyprland's own sockets.

use std::path::PathBuf;
use std::str::FromStr;

pub const SOCKET_NAME: &str = ".hyprshell.sock";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    /// Re-read the config file and Hyprland options.
    Reload,
    /// Show or hide every surface.
    Toggle,
}

impl Command {
    pub const ALL: [Command; 2] = [Command::Reload, Command::Toggle];

    pub fn as_str(self) -> &'static str {
        match self {
            Command::Reload => "reload",
            Command::Toggle => "toggle",
        }
    }
}

impl FromStr for Command {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "reload" => Ok(Command::Reload),
            "toggle" => Ok(Command::Toggle),
            other => Err(format!("unknown command {other:?}; expected one of: reload, toggle")),
        }
    }
}

/// `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.hyprshell.sock`.
pub fn socket_path() -> Result<PathBuf, crate::hypr::HyprError> {
    Ok(crate::hypr::instance_dir()?.join(SOCKET_NAME))
}

/// Reply line for a handled command.
pub fn reply_ok() -> &'static str {
    "ok"
}

pub fn reply_err(msg: &str) -> String {
    format!("err {msg}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_roundtrip() {
        for c in Command::ALL {
            assert_eq!(c.as_str().parse::<Command>().unwrap(), c);
        }
        assert_eq!(" toggle\n".parse::<Command>().unwrap(), Command::Toggle);
        assert!("hide".parse::<Command>().is_err());
    }
}
