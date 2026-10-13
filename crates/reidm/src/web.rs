//! Local web UI + JSON API (served on 127.0.0.1 only).
//!
//! A web page on the internet can reach localhost, so every request is checked:
//! the `Host` header must be our own address (blocks DNS rebinding), mutating calls need the
//! per-run token that is only embedded in the page we serve, and cross-origin `Origin`s are refused.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use reidm_core::{Manager, Request};
use serde::Deserialize;
use std::sync::Arc;

pub const UI_PORT: u16 = 61002;
const PAGE: &str = include_str!("../assets/ui.html");

#[derive(Clone)]
struct App {
    mgr: Manager,
    token: Arc<String>,
    /// Whether the daemon owns the port the *unmodified* extension dials first (1001). On Windows
    /// that is the difference between "works out of the box" and "another download manager got
    /// there first", which is worth telling the user about rather than burying in a log.
    idm_port: Arc<std::sync::atomic::AtomicBool>,
}

fn random_token() -> String {
    let mut b = [0u8; 32];
    // std has no CSPRNG; read the OS source directly.
    #[cfg(unix)]
    let ok = {
        use std::io::Read;
        std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_ok()
    };
    #[cfg(windows)]
    let ok = {
        #[link(name = "advapi32")]
        extern "system" {
            #[link_name = "SystemFunction036"]
            fn RtlGenRandom(buf: *mut u8, len: u32) -> u8;
        }
        unsafe { RtlGenRandom(b.as_mut_ptr(), b.len() as u32) != 0 }
    };
    #[cfg(not(any(unix, windows)))]
    let ok = false;

    if !ok {
        // The token is the only thing guarding the control API, so never fall back to a constant.
        // `RandomState` is itself OS-seeded; mixing in per-run values keeps two runs from sharing
        // a token if the OS source is unavailable.
        use std::hash::{BuildHasher, Hasher};
        tracing::warn!("no OS random source available; deriving the API token from process entropy");
        let mut x = std::collections::hash_map::RandomState::new().build_hasher().finish()
            ^ ((std::process::id() as u64) << 32)
            ^ std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
        for chunk in b.chunks_mut(8) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let n = chunk.len();
            chunk.copy_from_slice(&x.to_le_bytes()[..n]);
        }
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn guard(app: &App, h: &HeaderMap, need_token: bool) -> Result<(), StatusCode> {
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if host != format!("127.0.0.1:{UI_PORT}") && host != format!("localhost:{UI_PORT}") {
        return Err(StatusCode::FORBIDDEN);
    }
    if let Some(o) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if o != format!("http://{host}") {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    if need_token && h.get("x-reidm-token").and_then(|v| v.to_str().ok()) != Some(app.token.as_str()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

async fn index(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, false) {
        return c.into_response();
    }
    let mut r = Html(PAGE.replace("__TOKEN__", &app.token)).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    r.headers_mut().insert("content-security-policy", "default-src 'self' 'unsafe-inline'; frame-ancestors 'none'".parse().unwrap());
    r
}

async fn list(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let mut items = app.mgr.list();
    for i in &mut items {
        i.request.cookie = None; // never expose session cookies to the page
    }
    Json(items).into_response()
}

#[derive(Deserialize)]
struct AddBody {
    url: String,
    referer: Option<String>,
    filename: Option<String>,
}

async fn add(State(app): State<App>, h: HeaderMap, Json(b): Json<AddBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    match app.mgr.add(Request { url: b.url.trim().into(), referer: b.referer, filename: b.filename, ..Default::default() }) {
        Ok(id) => Json(serde_json::json!({ "id": id })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

async fn act(State(app): State<App>, h: HeaderMap, Path((id, what)): Path<(u64, String)>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    match what.as_str() {
        "pause" => app.mgr.pause(id),
        "resume" => {
            if let Err(e) = app.mgr.resume(id) {
                return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response();
            }
        }
        "open" | "folder" => {
            let Some(it) = app.mgr.get(id) else { return StatusCode::NOT_FOUND.into_response() };
            crate::launch::open_path(&it.path, what == "folder");
        }
        _ => return StatusCode::NOT_FOUND.into_response(),
    }
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
struct SettingsBody {
    #[serde(default)]
    default_dir: Option<String>,
    #[serde(default)]
    connections: Option<usize>,
}

/// What the daemon is actually able to offer right now.
async fn status(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    Json(serde_json::json!({
        "idm_port": app.idm_port.load(std::sync::atomic::Ordering::Relaxed),
        "idm_port_number": crate::ws::IDM_PORT,
        "local_port": crate::ws::LOCAL_PORT,
        "ui_port": UI_PORT,
        "platform": std::env::consts::OS,
    }))
    .into_response()
}

async fn settings(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let s = app.mgr.settings();
    Json(serde_json::json!({
        "default_dir": s.default_dir.to_string_lossy(),
        "connections": s.connections,
    }))
    .into_response()
}

async fn set_settings(State(app): State<App>, h: HeaderMap, Json(b): Json<SettingsBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let dir = b.default_dir.map(std::path::PathBuf::from).filter(|d| !d.as_os_str().is_empty());
    match app.mgr.set_settings(dir, b.connections) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

#[derive(Deserialize)]
struct Purge {
    #[serde(default)]
    purge: bool,
}

async fn remove(State(app): State<App>, h: HeaderMap, Path(id): Path<u64>, Query(q): Query<Purge>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    app.mgr.remove(id, q.purge);
    StatusCode::NO_CONTENT.into_response()
}

/// Publish the API token so a *local, same-user* client (the desktop GUI) can authenticate without
/// scraping the page. The file lives in the daemon's own data directory, which that client could
/// already read and write, so this grants no new access. Mode 0600 keeps other users out.
fn write_token_file(dir: &std::path::Path, token: &str) {
    let path = dir.join("api-token");
    let tmp = dir.join("api-token.tmp");
    // Create the file already private: chmod-ing afterwards leaves a window where the token is
    // world-readable on a system with a permissive umask.
    let file = {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        o.open(&tmp)
    };
    let Ok(mut f) = file else { return };
    use std::io::Write as _;
    if f.write_all(token.as_bytes()).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    drop(f);
    let _ = std::fs::rename(&tmp, &path);
}

/// Remove the token file on shutdown so a stale token is never trusted by a later client.
fn clear_token_file(dir: &std::path::Path) {
    let _ = std::fs::remove_file(dir.join("api-token"));
}

pub async fn serve(mgr: Manager, idm_port: Arc<std::sync::atomic::AtomicBool>) -> anyhow::Result<()> {
    // Bind *before* touching the token file: a losing second daemon must not overwrite the token
    // the daemon that actually owns the port is handing out.
    let l = tokio::net::TcpListener::bind(("127.0.0.1", UI_PORT)).await?;

    let token = random_token();
    let data_dir = mgr.data_dir().to_path_buf();
    write_token_file(&data_dir, &token);
    let app = App { mgr, token: Arc::new(token), idm_port };
    let router = Router::new()
        .route("/", get(index))
        .route("/api/downloads", get(list).post(add))
        .route("/api/downloads/:id", delete(remove))
        .route("/api/downloads/:id/:what", post(act))
        .route("/api/settings", get(settings).post(set_settings))
        .route("/api/status", get(status))
        .with_state(app);
    tracing::info!("web UI at http://127.0.0.1:{UI_PORT}/");
    let r = axum::serve(l, router).await;
    clear_token_file(&data_dir);
    r?;
    Ok(())
}
