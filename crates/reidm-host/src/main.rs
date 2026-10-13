//! Native-messaging host for `com.tonec.idm`.
//!
//! Mirrors the original `IDMMsgHost.exe`: it only shuttles bytes. Browser -> host frames are
//! `u32 LE length + JSON string` (the string holds one IDM wire message); they are unwrapped and
//! forwarded to the daemon over WebSocket, and replies travel back JSON-escaped.

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use reidm_proto::native;
use std::time::Duration;
use tokio::io::{stdin, stdout, AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message as Ws;

const PORT: u16 = 61001;
const SUBPROTOCOL: &str = "plugin.v3.internetdownloadmanager.com";
/// Chrome caps host->browser messages at 1 MiB and browser->host at 4 GiB; refuse absurd sizes.
const MAX_IN: usize = 64 * 1024 * 1024;

async fn connect() -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>> {
    let mut req = format!("ws://127.0.0.1:{PORT}/").into_client_request()?;
    req.headers_mut().insert("Sec-WebSocket-Protocol", HeaderValue::from_static(SUBPROTOCOL));
    let mut spawned = false;
    for _ in 0..40 {
        match tokio_tungstenite::connect_async(req.clone()).await {
            Ok((s, _)) => return Ok(s),
            Err(_) if !spawned => {
                // Like the original host launching the manager: start the daemon next to us.
                let exe = std::env::current_exe()?.with_file_name(if cfg!(windows) { "reidm.exe" } else { "reidm" });
                let mut c = std::process::Command::new(exe);
                c.arg("daemon").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    c.creation_flags(0x0000_0008 | 0x0800_0000); // DETACHED_PROCESS | CREATE_NO_WINDOW
                }
                c.spawn().context("cannot start reidm daemon")?;
                spawned = true;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
    bail!("daemon did not come up on port {PORT}")
}

#[tokio::main]
async fn main() -> Result<()> {
    let ws = connect().await?;
    let (mut ws_tx, mut ws_rx) = ws.split();

    let to_daemon = async {
        let mut inp = stdin();
        loop {
            let mut len = [0u8; 4];
            if inp.read_exact(&mut len).await.is_err() {
                return Ok::<_, anyhow::Error>(()); // browser closed the pipe
            }
            let n = u32::from_le_bytes(len) as usize;
            if n > MAX_IN {
                bail!("oversized native message ({n} bytes)");
            }
            let mut buf = vec![0u8; n];
            inp.read_exact(&mut buf).await?;
            let Some(text) = native::unwrap_json_string(&buf) else { continue };
            ws_tx.send(Ws::Binary(text.into_bytes())).await?;
        }
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
            out.write_all(&(payload.len() as u32).to_le_bytes()).await?;
            out.write_all(&payload).await?;
            out.flush().await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! { r = to_daemon => r, r = to_browser => r }
}
