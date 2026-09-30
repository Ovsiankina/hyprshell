//! Prints what the shell would read from the running Hyprland. Run with
//! `cargo run -p hyprshell-core --example hypr_probe`.

use hyprshell_core::hypr;

fn main() {
    println!("instance dir: {:?}", hypr::instance_dir());
    println!("defaults:     {:?}", hypr::Defaults::fetch());
    let monitors = hypr::monitors().expect("monitors");
    for m in &monitors {
        println!(
            "monitor {} id={} {}x{} @{},{} scale={} focused={} disabled={} ws={}",
            m.name, m.id, m.width, m.height, m.x, m.y, m.scale, m.focused, m.disabled, m.active_workspace.id
        );
    }
    let workspaces = hypr::workspaces().expect("workspaces");
    for w in &workspaces {
        println!("workspace {} on {} fullscreen={}", w.id, w.monitor, w.hasfullscreen);
    }
    println!("fullscreen monitors: {:?}", hypr::fullscreen_monitors(&monitors, &workspaces));
    if std::env::args().any(|a| a == "--events") {
        println!("listening on socket2 (ctrl-c to stop)...");
        for ev in hypr::EventReader::connect().expect("socket2") {
            println!("{ev:?}");
        }
    }
}
