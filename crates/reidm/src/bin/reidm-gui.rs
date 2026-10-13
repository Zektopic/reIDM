//! No-console launcher (Windows GUI subsystem). `reidm-gui` starts the daemon if needed and opens the
//! web UI; `reidm-gui --background` only makes sure the daemon is running (used at login).
#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "../launch.rs"]
mod launch;

use std::net::TcpStream;
use std::time::Duration;

fn up() -> bool {
    TcpStream::connect_timeout(&"127.0.0.1:61002".parse().unwrap(), Duration::from_millis(300)).is_ok()
}

fn main() {
    let background = std::env::args().any(|a| a == "--background");
    if !up() {
        let _ = launch::spawn_daemon();
        for _ in 0..40 {
            if up() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    if !background {
        launch::open_url("http://127.0.0.1:61002/");
    }
}
