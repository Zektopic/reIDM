//! Native-messaging host, registered under two names:
//!
//! * `com.tonec.idm` for the official IDM extension. Mirrors the original `IDMMsgHost.exe`: it
//!   only shuttles bytes. Browser -> host frames are `u32 LE length + JSON string` (the string
//!   holds one IDM wire message); they are unwrapped and forwarded to the daemon over WebSocket,
//!   and replies travel back JSON-escaped.
//! * `com.zektopic.reidm` for reIDM's own extension, which talks to the daemon over HTTP and
//!   uses this host for one thing only: starting reIDM when it is not running, so capturing a
//!   browser download never depends on the user having opened reIDM first.
//!
//! Chrome passes the calling extension's origin as the first argument, which is how the two are
//! told apart; each host manifest only admits its own extension.

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use reidm_proto::native;
use std::time::Duration;
use tokio::io::{stdin, stdout, AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message as Ws;

const SUBPROTOCOL: &str = "plugin.v3.internetdownloadmanager.com";
/// Chrome caps browser->host messages at 4 GiB; refuse absurd sizes.
const MAX_IN: usize = 64 * 1024 * 1024;
/// Chrome caps host->browser messages at 1 MiB and stops a host that sends a bigger one.
const MAX_OUT: usize = 1024 * 1024;
/// reIDM's own extension. Its ID is fixed by the `key` in `extension/manifest.json`.
const REIDM_EXTENSION_ID: &str = "pgeonmdaagbiccimgojpafdjcipcfnlc";
/// The official IDM extension, as its host manifests admit it (`reidm::install`).
const IDM_CHROME_IDS: [&str; 2] = ["ngpampappnmepgilojfohadhhmbhlaek", "llbjbkhnmlidjebalopleeepgdfgcpec"];
const IDM_FIREFOX_ID: &str = "mozilla_cc3@internetdownloadmanager.com";
/// Message types from here up were reIDM's own control messages, which the extension never
/// sends and the service no longer honours; they are not passed on either.
const RESERVED_TYPES: u32 = 1000;

/// The daemon's ports; `REIDM_LOCAL_PORT` and `REIDM_UI_PORT` move them, as for the daemon.
fn port(var: &str, default: u16) -> u16 {
    std::env::var(var).ok().and_then(|v| v.trim().parse().ok()).filter(|p| *p != 0).unwrap_or(default)
}

fn ws_port() -> u16 {
    port("REIDM_LOCAL_PORT", 61001)
}

/// The daemon's HTTP API, which reIDM's own extension uses.
fn ui_port() -> u16 {
    port("REIDM_UI_PORT", 61002)
}

/// Which extension started this host. Chromium passes the caller's origin as the first
/// argument; Firefox passes the manifest's path and then the extension's ID.
#[derive(Debug, PartialEq, Eq)]
enum Caller {
    Reidm,
    Idm,
    Other,
}

fn caller(args: &[String]) -> Caller {
    let first = args.first().map(|a| a.trim_end_matches('/')).unwrap_or_default();
    if first == format!("chrome-extension://{REIDM_EXTENSION_ID}") {
        return Caller::Reidm;
    }
    if IDM_CHROME_IDS.iter().any(|id| first == format!("chrome-extension://{id}")) || args.get(1).is_some_and(|a| a == IDM_FIREFOX_ID) {
        return Caller::Idm;
    }
    Caller::Other
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

async fn connect() -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>> {
    let mut req = format!("ws://127.0.0.1:{}/", ws_port()).into_client_request()?;
    req.headers_mut().insert("Sec-WebSocket-Protocol", HeaderValue::from_static(SUBPROTOCOL));
    let mut spawned = false;
    for _ in 0..40 {
        match tokio_tungstenite::connect_async(req.clone()).await {
            Ok((s, _)) => return Ok(s),
            Err(_) if !spawned => {
                // Like the original host launching the manager: start the daemon next to us.
                spawn_daemon()?;
                spawned = true;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
    bail!("daemon did not come up on port {}", ws_port())
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

/// reIDM's own extension: `{"cmd":"launch"}` makes sure the daemon is up, then answers
/// `{"ok":true}` (or `{"ok":false,"error":…}`) and exits.
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
    match caller(&args) {
        Caller::Reidm => return launcher().await,
        Caller::Idm => {}
        // The browser's manifests admit only those two, so this is someone running the host by
        // hand; a relay to the service is not something to hand out to anyone who asks.
        Caller::Other => bail!("reidm-host is started by the browser for reIDM's or IDM's extension, not by {:?}", args.first()),
    }

    let ws = connect().await?;
    let (mut ws_tx, mut ws_rx) = ws.split();

    let to_daemon = async {
        let mut inp = stdin();
        while let Some(buf) = read_frame(&mut inp).await? {
            let Some(text) = native::unwrap_json_string(&buf) else { continue };
            if !relayed(&text) {
                continue;
            }
            ws_tx.send(Ws::Binary(text.into_bytes())).await?;
        }
        Ok::<_, anyhow::Error>(()) // browser closed the pipe
    };
    let to_browser = async {
        let mut out = stdout();
        while let Some(f) = ws_rx.next().await {
            let bytes = match f? {
                Ws::Binary(b) => b,
                Ws::Text(t) => t.into_bytes(),
                Ws::Close(_) => break,
                _ => continue,
            };
            let payload = native::wrap_json_string(&String::from_utf8_lossy(&bytes));
            write_frame(&mut out, &payload).await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! { r = to_daemon => r, r = to_browser => r }
}

/// Whether a message from the browser goes on to the service: everything the IDM protocol
/// defines, but none of reIDM's former control messages.
fn relayed(text: &str) -> bool {
    match reidm_proto::Message::decode(text.as_bytes()) {
        Ok((m, _)) => m.ty < RESERVED_TYPES,
        // Not ours to judge; the service drops what it cannot decode.
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn callers() {
        assert_eq!(caller(&args(&["chrome-extension://pgeonmdaagbiccimgojpafdjcipcfnlc/"])), Caller::Reidm);
        assert_eq!(caller(&args(&["chrome-extension://ngpampappnmepgilojfohadhhmbhlaek/"])), Caller::Idm);
        assert_eq!(caller(&args(&["/usr/lib/mozilla/native-messaging-hosts/com.tonec.idm.json", "mozilla_cc3@internetdownloadmanager.com"])), Caller::Idm);
        assert_eq!(caller(&args(&["chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"])), Caller::Other);
        assert_eq!(caller(&args(&[])), Caller::Other);
    }

    #[test]
    fn control_messages_are_not_relayed() {
        let ctl = reidm_proto::Message::new(1, 1003, 1, 1).with_args([5, 1]).encode();
        assert!(!relayed(&ctl), "remove-and-purge must not pass");
        let hello = reidm_proto::Message::new(1, 2, 5, 0).with_args([113, 93]).encode();
        assert!(relayed(&hello));
    }
}
