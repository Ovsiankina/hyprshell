//! `hyprshellctl <reload|toggle>`: send one command to the running shell.

use hyprshell_core::ctl::{self, Command};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match args.as_slice() {
        [one] => match one.parse::<Command>() {
            Ok(c) => c,
            Err(e) => return usage(&e),
        },
        _ => return usage("expected exactly one command"),
    };
    match send(command) {
        Ok(reply) if reply == ctl::reply_ok() => ExitCode::SUCCESS,
        Ok(reply) => {
            eprintln!("hyprshell: {reply}");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("hyprshellctl: {e}");
            ExitCode::FAILURE
        }
    }
}

fn send(command: Command) -> Result<String, String> {
    let path = ctl::socket_path().map_err(|e| e.to_string())?;
    let mut stream = UnixStream::connect(&path)
        .map_err(|e| format!("cannot connect to {}: {e} (is hyprshell running?)", path.display()))?;
    writeln!(stream, "{}", command.as_str()).map_err(|e| e.to_string())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).map_err(|e| e.to_string())?;
    Ok(reply.trim_end().to_string())
}

fn usage(problem: &str) -> ExitCode {
    eprintln!("hyprshellctl: {problem}");
    eprintln!("usage: hyprshellctl <reload|toggle>");
    ExitCode::from(2)
}
