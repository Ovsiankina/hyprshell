//! Every event source, as an iced `Subscription`. None of them polls: the
//! config watcher sleeps on inotify, the Hyprland reader blocks on its
//! socket, the ctl server blocks on `accept`, and the ticker sleeps until the
//! next wall-clock boundary.

use crate::Message;
use hyprshell_core::clock::ClockTime;
use hyprshell_core::config::Config;
use hyprshell_core::ctl::{self, Command};
use hyprshell_core::hypr;
use hyprshell_core::view::TickRate;
use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, StreamExt};
use iced::{Subscription, stream};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Editors write in bursts (truncate, write, close; or write-temp, rename).
/// Reload once the file has been quiet this long.
const CONFIG_QUIET: Duration = Duration::from_millis(100);
const MAX_EVENT_RECONNECTS: u32 = 5;
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(200);
const TICK_MARGIN: Duration = Duration::from_millis(2);
const EVENT_RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// Re-read the config whenever its file changes. Watches the parent directory
/// so atomic-replace saves (a new inode under the same name) are seen.
pub fn config_file(path: PathBuf) -> Subscription<Message> {
    Subscription::run_with(path, |path| {
        let path = path.clone();
        stream::channel(4, async move |mut out| {
            let Some(dir) = path.parent().map(Path::to_path_buf) else {
                tracing::error!("config path {} has no parent directory", path.display());
                return;
            };
            let file_name = path.file_name().map(|n| n.to_os_string());
            let (mut tx, mut rx) = mpsc::channel::<()>(8);
            let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(ev) => {
                    use notify::EventKind::*;
                    let relevant = matches!(ev.kind, Create(_) | Modify(_) | Remove(_))
                        && ev.paths.iter().any(|p| p.file_name() == file_name.as_deref());
                    if relevant {
                        // A full channel already holds a pending reload.
                        let _ = tx.try_send(());
                    }
                }
                Err(e) => tracing::warn!("config watcher: {e}"),
            });
            let mut watcher = match watcher {
                Ok(w) => w,
                Err(e) => {
                    tracing::error!("cannot create config watcher: {e}");
                    return;
                }
            };
            if let Err(e) = notify::Watcher::watch(&mut watcher, &dir, notify::RecursiveMode::NonRecursive) {
                tracing::error!("cannot watch {}: {e}; live reload is off", dir.display());
                return;
            }
            tracing::info!("watching {} for changes", path.display());

            while rx.next().await.is_some() {
                // Coalesce the burst: wait until nothing arrives for a while.
                while tokio::time::timeout(CONFIG_QUIET, rx.next()).await.is_ok_and(|e| e.is_some()) {}
                let result = load(&path);
                if out.send(Message::ConfigLoaded(result)).await.is_err() {
                    break;
                }
            }
            drop(watcher);
        })
    })
}

/// Load the file; a missing file is the default config, not an error.
pub fn load(path: &Path) -> Result<Config, String> {
    if !path.exists() {
        tracing::warn!("{} does not exist; using built-in defaults", path.display());
        return Ok(Config::default());
    }
    Config::load(path).map_err(|e| e.to_string())
}

/// Hyprland's event stream, read on its own thread because the socket API is
/// blocking. Ends with `HyprDisconnected` when Hyprland goes away.
pub fn hyprland_events() -> Subscription<Message> {
    Subscription::run(|| {
        stream::channel(32, async move |mut out| {
            let (tx, mut rx) = mpsc::channel::<Option<hypr::Event>>(32);
            std::thread::Builder::new()
                .name("hypr-events".into())
                .spawn(move || {
                    let mut tx = tx;
                    // Hyprland drops a client that falls 64 events behind, so
                    // an EOF is not always Hyprland exiting: reconnect a few
                    // times before giving up.
                    let mut failures = 0;
                    loop {
                        let reader = match hypr::EventReader::connect() {
                            Ok(r) => r,
                            Err(e) => {
                                failures += 1;
                                tracing::error!("cannot connect to hyprland events ({failures}/{MAX_EVENT_RECONNECTS}): {e}");
                                if failures >= MAX_EVENT_RECONNECTS {
                                    break;
                                }
                                std::thread::sleep(EVENT_RECONNECT_DELAY);
                                continue;
                            }
                        };
                        failures = 0;
                        for ev in reader {
                            match ev {
                                // Events the shell does not react to stay on
                                // this thread; crossing into the loop would
                                // rebuild every surface's widget tree.
                                Ok(hypr::Event::Other { .. }) => continue,
                                Ok(ev) => {
                                    if iced::futures::executor::block_on(tx.send(Some(ev))).is_err() {
                                        return;
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("hyprland event socket: {e}");
                                    break;
                                }
                            }
                        }
                        tracing::warn!("hyprland event socket closed; reconnecting");
                        std::thread::sleep(EVENT_RECONNECT_DELAY);
                    }
                    let _ = iced::futures::executor::block_on(tx.send(None));
                })
                .expect("spawn hypr-events thread");
            while let Some(item) = rx.next().await {
                let msg = match item {
                    Some(ev) => Message::Hypr(ev),
                    None => Message::HyprDisconnected,
                };
                if out.send(msg).await.is_err() {
                    break;
                }
            }
        })
    })
}

/// The `hyprshellctl` server: one line per connection, `ok` or `err ...` back.
pub fn ctl_server() -> Subscription<Message> {
    Subscription::run(|| {
        stream::channel(8, async move |mut out| {
            let (tx, mut rx) = mpsc::channel::<Command>(8);
            std::thread::Builder::new()
                .name("hyprshellctl".into())
                .spawn(move || {
                    let mut tx = tx;
                    let listener = match bind() {
                        Ok(l) => l,
                        Err(e) => {
                            tracing::error!("hyprshellctl socket: {e}");
                            return;
                        }
                    };
                    for conn in listener.incoming() {
                        let mut conn = match conn {
                            Ok(c) => c,
                            Err(e) => {
                                // A persistent error (fd exhaustion) must not spin.
                                tracing::warn!("hyprshellctl accept: {e}");
                                std::thread::sleep(ACCEPT_RETRY_DELAY);
                                continue;
                            }
                        };
                        let mut line = String::new();
                        let reply = match BufReader::new(&conn).read_line(&mut line) {
                            Ok(_) => match line.parse::<Command>() {
                                Ok(cmd) => match iced::futures::executor::block_on(tx.send(cmd)) {
                                    Ok(()) => ctl::reply_ok().to_string(),
                                    Err(_) => return,
                                },
                                Err(e) => ctl::reply_err(&e),
                            },
                            Err(e) => ctl::reply_err(&e.to_string()),
                        };
                        let _ = writeln!(conn, "{reply}");
                    }
                })
                .expect("spawn hyprshellctl thread");
            while let Some(cmd) = rx.next().await {
                if out.send(Message::Ctl(cmd)).await.is_err() {
                    break;
                }
            }
        })
    })
}

/// Bind the ctl socket, replacing a stale file left by a crashed instance but
/// refusing to steal it from a live one.
fn bind() -> std::io::Result<UnixListener> {
    let path = ctl::socket_path().map_err(std::io::Error::other)?;
    match UnixListener::bind(&path) {
        Ok(l) => Ok(l),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                return Err(std::io::Error::other(format!(
                    "another hyprshell is listening on {}",
                    path.display()
                )));
            }
            tracing::info!("removing stale socket {}", path.display());
            std::fs::remove_file(&path)?;
            UnixListener::bind(&path)
        }
        Err(e) => Err(e),
    }
}

/// One message per wall-clock second or minute, on the boundary. Changing the
/// rate replaces the subscription, so a stale sleep never fires.
pub fn ticks(rate: TickRate) -> Subscription<Message> {
    if rate == TickRate::Off {
        return Subscription::none();
    }
    Subscription::run_with(rate, |rate| {
        let rate = *rate;
        stream::channel(1, async move |mut out| {
            loop {
                let now = ClockTime::now();
                let wait = match rate {
                    TickRate::Second => now.until_next_second(),
                    TickRate::Minute => now.until_next_minute(),
                    TickRate::Off => return,
                };
                // A hair past the boundary so a timer that wakes exactly on
                // time never reads the old second.
                tokio::time::sleep(wait + TICK_MARGIN).await;
                if out.send(Message::Tick).await.is_err() {
                    return;
                }
            }
        })
    })
}
