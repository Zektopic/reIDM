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

// ── endpoints for reIDM's own browser extension ──────────────────────────
//
// These are reached from an extension service worker, which has no API token. The origin check
// is the authorisation instead, exactly as on the WebSocket: a page cannot forge `Origin`, and a
// page cannot add a custom header here either, because we never answer a CORS preflight.
// Extension requests with `host_permissions` are not subject to CORS, so nothing is needed for
// the extension itself to read the responses.

/// Is this an extension origin? Browsers set `Origin` themselves and page script cannot forge
/// it, so this is a reliable discriminator — a web page always arrives with its own http(s)
/// origin. It is not a *credential*: any local process can send the header, but such a process
/// could already read the daemon's token file.
fn is_extension_origin(h: &HeaderMap) -> bool {
    matches!(
        h.get(header::ORIGIN).and_then(|v| v.to_str().ok()),
        Some(o) if o.starts_with("chrome-extension://")
            || o.starts_with("moz-extension://")
            || o.starts_with("safari-web-extension://")
    )
}

fn extension_caller(h: &HeaderMap) -> Result<(), StatusCode> {
    match h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        Some(_) if is_extension_origin(h) => Ok(()),
        // No Origin: a local process (curl, a script). It could already read the token file, so
        // this grants nothing new.
        None => Ok(()),
        Some(_) => Err(StatusCode::FORBIDDEN),
    }
}

/// Answer a CORS preflight.
///
/// Chrome may preflight the extension's JSON POST even though host permissions normally lift
/// the response check, so refusing every preflight would break the extension. The reply is
/// scoped to the asking extension origin, which a web page cannot obtain: it would have to send
/// `Origin: chrome-extension://…`, and browsers do not let page script set that header.
async fn ext_preflight(h: HeaderMap) -> Response {
    if !is_extension_origin(&h) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let origin = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let mut r = StatusCode::NO_CONTENT.into_response();
    let hdrs = r.headers_mut();
    if let Ok(v) = origin.parse() {
        hdrs.insert("access-control-allow-origin", v);
    }
    hdrs.insert("access-control-allow-methods", "POST, GET, OPTIONS".parse().unwrap());
    hdrs.insert("access-control-allow-headers", "Content-Type".parse().unwrap());
    hdrs.insert("access-control-max-age", "600".parse().unwrap());
    hdrs.insert("vary", "Origin".parse().unwrap());
    r
}

/// What the extension answers on start-up to decide whether reIDM is running.
async fn ext_ping(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = extension_caller(&h) {
        return c.into_response();
    }
    let s = app.mgr.settings();
    Json(serde_json::json!({
        "app": "reIDM",
        "version": env!("CARGO_PKG_VERSION"),
        "default_dir": s.default_dir.to_string_lossy(),
        "connections": s.connections,
        "max_concurrent": s.max_concurrent,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct ExtAdd {
    url: String,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    referer: Option<String>,
    #[serde(default)]
    cookie: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
}

async fn ext_add(State(app): State<App>, h: HeaderMap, Json(b): Json<ExtAdd>) -> Response {
    if let Err(c) = extension_caller(&h) {
        return c.into_response();
    }
    let req = Request {
        url: b.url.trim().to_string(),
        filename: b.filename,
        referer: b.referer,
        cookie: b.cookie,
        user_agent: b.user_agent,
        dir: None,
    };
    match app.mgr.add_detailed(req) {
        Ok(o) => Json(serde_json::json!({ "ok": true, "id": o.id, "outcome": o.outcome })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "ok": false, "error": format!("{e:#}") }))).into_response(),
    }
}

// ── batch and grabber ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct BatchBody {
    urls: Vec<String>,
    #[serde(default)]
    dir: Option<String>,
    /// Applied to every URL in the batch.
    #[serde(default)]
    referer: Option<String>,
    /// Only meaningful for a single URL.
    #[serde(default)]
    filename: Option<String>,
}

/// Queue many URLs at once. Answers per URL, so the caller can say what happened to each.
async fn add_batch(State(app): State<App>, h: HeaderMap, Json(b): Json<BatchBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let dir = b.dir.map(std::path::PathBuf::from);
    let single = b.urls.len() == 1;
    let reqs: Vec<Request> = b
        .urls
        .into_iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .map(|url| Request {
            url,
            dir: dir.clone(),
            referer: b.referer.clone(),
            filename: if single { b.filename.clone() } else { None },
            ..Default::default()
        })
        .collect();
    let results: Vec<serde_json::Value> = app
        .mgr
        .add_many(reqs)
        .into_iter()
        .map(|(url, r)| match r {
            Ok(o) => serde_json::json!({ "url": url, "id": o.id, "outcome": o.outcome }),
            Err(e) => serde_json::json!({ "url": url, "error": format!("{e:#}") }),
        })
        .collect();
    Json(serde_json::json!({ "results": results })).into_response()
}

/// Backwards-compatible alias used by the engine CLI: `POST /api/downloads?batch=1`.
#[derive(Deserialize)]
struct GrabBody {
    url: String,
    #[serde(default)]
    options: reidm_core::grabber::Options,
}

/// Scan a page for downloadable files. Read-only: nothing is queued by this call.
async fn grab(State(_app): State<App>, h: HeaderMap, Json(b): Json<GrabBody>) -> Response {
    if let Err(c) = guard(&_app, &h, true) {
        return c.into_response();
    }
    match reidm_core::grabber::grab_page(&b.url, &b.options).await {
        Ok(files) => Json(serde_json::json!({ "files": files })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

#[derive(Deserialize)]
struct SettingsBody {
    #[serde(default)]
    default_dir: Option<String>,
    #[serde(default)]
    connections: Option<usize>,
    #[serde(default)]
    max_concurrent: Option<usize>,
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
        "max_concurrent": s.max_concurrent,
    }))
    .into_response()
}

async fn set_settings(State(app): State<App>, h: HeaderMap, Json(b): Json<SettingsBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let dir = b.default_dir.map(std::path::PathBuf::from).filter(|d| !d.as_os_str().is_empty());
    match app.mgr.set_settings(dir, b.connections, b.max_concurrent) {
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
        .route("/api/downloads/batch", post(add_batch))
        .route("/api/grab", post(grab))
        .route("/ext/ping", get(ext_ping).options(ext_preflight))
        .route("/ext/add", post(ext_add).options(ext_preflight))
        .with_state(app);
    tracing::info!("web UI at http://127.0.0.1:{UI_PORT}/");
    let r = axum::serve(l, router).await;
    clear_token_file(&data_dir);
    r?;
    Ok(())
}
