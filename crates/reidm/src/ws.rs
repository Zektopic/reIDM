//! WebSocket endpoint. The IDM extension tries `ws://127.0.0.1:1001` with the sub-protocol
//! `plugin.v3.internetdownloadmanager.com` before falling back to native messaging; the
//! reIDM native host and the CLI use the same endpoint on an unprivileged port.

use crate::session::Session;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use reidm_core::Manager;
use reidm_proto::Message;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::Message as Ws;

pub const SUBPROTOCOL: &str = "plugin.v3.internetdownloadmanager.com";
/// Port the real IDM network monitor listens on (privileged on Unix; best effort here).
pub const IDM_PORT: u16 = 1001;
/// Unprivileged port used by the native host and CLI.
pub const LOCAL_PORT: u16 = 61001;

/// Browsers let *any* web page open a WebSocket to localhost, so a page-controlled Origin
/// must never reach the download queue. Only extension origins (or no Origin: a local
/// process, which could already write to our data directory) are accepted.
pub fn origin_allowed(origin: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(o) => o.starts_with("chrome-extension://") || o.starts_with("moz-extension://") || o.starts_with("extension://"),
    }
}

#[allow(clippy::result_large_err)]
fn check(req: &Request, mut resp: Response) -> std::result::Result<Response, ErrorResponse> {
    let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
    if !origin_allowed(origin) {
        tracing::warn!("rejected WebSocket from origin {origin:?}");
        let mut r = ErrorResponse::new(Some("forbidden origin".into()));
        *r.status_mut() = StatusCode::FORBIDDEN;
        return Err(r);
    }
    let offered = req
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|p| p.trim() == SUBPROTOCOL));
    if offered {
        resp.headers_mut().insert("sec-websocket-protocol", HeaderValue::from_static(SUBPROTOCOL));
    }
    Ok(resp)
}

async fn serve_conn(stream: TcpStream, mgr: Manager) -> Result<()> {
    let ws = tokio_tungstenite::accept_hdr_async(stream, check).await?;
    let (mut tx, mut rx) = ws.split();
    let mut session = Session::new(mgr);
    while let Some(frame) = rx.next().await {
        let bytes = match frame? {
            Ws::Binary(b) => b,
            Ws::Text(t) => t.into_bytes(),
            Ws::Close(_) => break,
            _ => continue,
        };
        // A frame normally holds exactly one message, but tolerate several back to back.
        let mut rest = &bytes[..];
        while !rest.is_empty() {
            match Message::decode(rest) {
                Ok((m, n)) => {
                    rest = &rest[n..];
                    for out in session.handle(&m) {
                        // The extension reads replies as Blobs, so always send binary frames.
                        tx.send(Ws::Binary(out.encode().into_bytes())).await?;
                    }
                }
                Err(e) => {
                    tracing::debug!("dropping undecodable frame: {e}");
                    break;
                }
            }
        }
    }
    Ok(())
}

pub async fn listen(addr: &str, mgr: Manager) -> Result<()> {
    let l = TcpListener::bind(addr).await?;
    tracing::info!("listening on ws://{addr}");
    loop {
        let (s, peer) = l.accept().await?;
        let mgr = mgr.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_conn(s, mgr).await {
                tracing::debug!("connection {peer} ended: {e:#}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::origin_allowed;

    #[test]
    fn origins() {
        assert!(origin_allowed(None));
        assert!(origin_allowed(Some("chrome-extension://ngpampappnmepgilojfohadhhmbhlaek")));
        assert!(origin_allowed(Some("moz-extension://1234")));
        assert!(!origin_allowed(Some("https://evil.example")));
        assert!(!origin_allowed(Some("http://localhost:8080")));
        assert!(!origin_allowed(Some("null")));
    }
}
