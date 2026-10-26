//! Native-messaging host, registered as `com.zektopic.reidm` for reIDM's own extension.
//!
//! The extension talks to the daemon over its HTTP API; this host exists for one thing only:
//! starting reIDM when it is not running, so capturing a browser download never depends on the
//! user having opened reIDM first. It answers `{"cmd":"launch"}` and `{"cmd":"ping"}`.
//!
//! Chrome passes the calling extension's origin as the first argument and Firefox passes the
//! manifest's path and then the extension's ID; either way the caller must be reIDM's extension.

use anyhow::{bail, Context, Result};
use std::time::Duration;
use tokio::io::{stdin, stdout, AsyncReadExt, AsyncWriteExt};

/// Chrome caps browser->host messages at 4 GiB; refuse absurd sizes.
const MAX_IN: usize = 64 * 1024 * 1024;
/// Chrome caps host->browser messages at 1 MiB and stops a host that sends a bigger one.
const MAX_OUT: usize = 1024 * 1024;
/// reIDM's own extension. Its Chromium ID is fixed by the `key` in `extension/manifest.json`;
/// Firefox has no such key, so its `browser_specific_settings.gecko.id` names it instead.
const REIDM_EXTENSION_ID: &str = "pgeonmdaagbiccimgojpafdjcipcfnlc";
const REIDM_FIREFOX_ID: &str = "reidm@zektopic.in";

/// The daemon's HTTP API ports; `REIDM_UI_PORT` moves them, as for the daemon.
fn port(var: &str, default: u16) -> u16 {
    std::env::var(var).ok().and_then(|v| v.trim().parse().ok()).filter(|p| *p != 0).unwrap_or(default)
}

/// The daemon's HTTP API, which reIDM's extension uses.
fn ui_port() -> u16 {
    port("REIDM_UI_PORT", 61002)
}

/// Is the browser starting this host for reIDM's extension? Chromium passes the caller's origin
/// as the first argument; Firefox passes the manifest's path and then the extension's ID.
fn is_reidm_caller(args: &[String]) -> bool {
    let first = args.first().map(|a| a.trim_end_matches('/')).unwrap_or_default();
    first == format!("chrome-extension://{REIDM_EXTENSION_ID}") || args.get(1).is_some_and(|a| a == REIDM_FIREFOX_ID)
}

/// Start `reidm daemon` so that it outlives this host.
///
/// Both halves matter. On Windows, Chrome runs a native host inside a job object that kills
/// every process in it when the host exits, so the daemon must break away from that job. On
/// Unix, the browser may signal the host's process group, so the daemon gets its own.
fn spawn_daemon() -> Result<()> {
    let exe = std::env::current_exe()?.with_file_name(if cfg!(windows) { "reidm.exe" } else { "reidm" });
    let mut c = std::process::Command::new(&exe);
    c.arg("daemon").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB | CREATE_NO_WINDOW
        const FLAGS: u32 = 0x0000_0008 | 0x0000_0200 | 0x0100_0000 | 0x0800_0000;
        if c.creation_flags(FLAGS).spawn().is_ok() {
            return Ok(());
        }
        // A job that forbids breakaway refuses the flag outright; better a daemon that lives as
        // long as the browser session than none at all.
        let mut c = std::process::Command::new(&exe);
        c.arg("daemon").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        c.creation_flags(0x0000_0008 | 0x0800_0000).spawn().with_context(|| format!("cannot start {}", exe.display()))?;
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    #[cfg(not(windows))]
    c.spawn().with_context(|| format!("cannot start {}", exe.display()))?;
    #[allow(unreachable_code)]
    Ok(())
}

fn port_open(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Read one `u32 LE length + payload` frame; `None` when the browser closed the pipe.
async fn read_frame(inp: &mut tokio::io::Stdin) -> Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    if inp.read_exact(&mut len).await.is_err() {
        return Ok(None);
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_IN {
        bail!("oversized native message ({n} bytes)");
    }
    let mut buf = vec![0u8; n];
    inp.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

async fn write_frame(out: &mut tokio::io::Stdout, payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_OUT {
        // Chrome would end the connection over it, and with it every later message.
        eprintln!("reidm-host: dropping a {} byte message, over the browser's 1 MiB limit", payload.len());
        return Ok(());
    }
    out.write_all(&(payload.len() as u32).to_le_bytes()).await?;
    out.write_all(payload).await?;
    out.flush().await?;
    Ok(())
}

/// `{"cmd":"launch"}` makes sure the daemon is up, then answers `{"ok":true}` (or
/// `{"ok":false,"error":…}`) and exits.
async fn launcher() -> Result<()> {
    let mut inp = stdin();
    let mut out = stdout();
    while let Some(buf) = read_frame(&mut inp).await? {
        let msg: serde_json::Value = serde_json::from_slice(&buf).unwrap_or_default();
        let reply = match msg["cmd"].as_str() {
            Some("launch") => match ensure_daemon().await {
                Ok(started) => serde_json::json!({ "ok": true, "started": started, "version": env!("CARGO_PKG_VERSION") }),
                Err(e) => serde_json::json!({ "ok": false, "error": format!("{e:#}") }),
            },
            Some("ping") => serde_json::json!({ "ok": true, "running": port_open(ui_port()), "version": env!("CARGO_PKG_VERSION") }),
            _ => serde_json::json!({ "ok": false, "error": "unknown command" }),
        };
        write_frame(&mut out, reply.to_string().as_bytes()).await?;
    }
    Ok(())
}

/// Returns whether the daemon had to be started.
async fn ensure_daemon() -> Result<bool> {
    if port_open(ui_port()) {
        return Ok(false);
    }
    spawn_daemon()?;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if port_open(ui_port()) {
            return Ok(true);
        }
    }
    bail!("reIDM was started but did not answer on port {}", ui_port())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !is_reidm_caller(&args) {
        // The browser's manifest admits only reIDM's extension, so this is someone running the
        // host by hand; starting the service is not something to hand out to anyone who asks.
        bail!("reidm-host is started by the browser for reIDM's extension, not by {:?}", args.first());
    }
    launcher().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_reidms_extension_may_start_the_daemon() {
        assert!(is_reidm_caller(&args(&["chrome-extension://pgeonmdaagbiccimgojpafdjcipcfnlc/"])));
        assert!(is_reidm_caller(&args(&["/usr/lib/mozilla/native-messaging-hosts/com.zektopic.reidm.json", "reidm@zektopic.in"])));
        assert!(!is_reidm_caller(&args(&["chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"])));
        assert!(!is_reidm_caller(&args(&[])));
    }
}
