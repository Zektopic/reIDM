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
use reidm_core::{AddOptions, Manager, Media, Request, SettingsPatch};
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
    /// When reIDM's own extension last checked in, so the window can say whether browser
    /// capture is actually connected rather than guessing.
    ext: Arc<std::sync::Mutex<Option<ExtSeen>>>,
}

#[derive(Clone)]
struct ExtSeen {
    at: std::time::SystemTime,
    version: String,
    browser: String,
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
    #[serde(default)]
    referer: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    cookie: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    dir: Option<String>,
    /// *Download Later*: list it in the scheduled queue without starting it.
    #[serde(default)]
    later: bool,
    /// Fetch the video on the page `url` through yt-dlp, in this format.
    #[serde(default)]
    media: Option<Media>,
}

async fn add(State(app): State<App>, h: HeaderMap, Json(b): Json<AddBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let req = Request {
        url: b.url.trim().into(),
        referer: b.referer,
        filename: b.filename.filter(|f| !f.trim().is_empty()),
        cookie: b.cookie,
        user_agent: b.user_agent,
        dir: b.dir.filter(|d| !d.trim().is_empty()).map(std::path::PathBuf::from),
        media: b.media,
    };
    match app.mgr.add_with(req, AddOptions { later: b.later, ..Default::default() }) {
        Ok(o) => Json(serde_json::json!({ "id": o.id, "outcome": o.outcome })).into_response(),
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
        "restart" => {
            if let Err(e) = app.mgr.restart(id).await {
                return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response();
            }
        }
        "schedule" | "unschedule" => {
            if let Err(e) = app.mgr.set_scheduled(id, what == "schedule") {
                return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response();
            }
        }
        // Take the next link the browser hands over as this download's new address.
        "refresh-capture" => {
            if let Err(e) = app.mgr.arm_refresh(id) {
                return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response();
            }
        }
        _ => return StatusCode::NOT_FOUND.into_response(),
    }
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
struct RefreshBody {
    url: String,
    #[serde(default)]
    referer: Option<String>,
    #[serde(default)]
    cookie: Option<String>,
}

/// *Refresh download address*: point an unfinished download at a new URL for the same file.
async fn refresh(State(app): State<App>, h: HeaderMap, Path(id): Path<u64>, Json(b): Json<RefreshBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let req = Request { url: b.url, referer: b.referer, cookie: b.cookie, ..Default::default() };
    match app.mgr.refresh_address(id, req).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

#[derive(Deserialize)]
struct LimitBody {
    /// Bytes per second; `null` or 0 lifts this download's own cap.
    #[serde(default)]
    limit: Option<u64>,
}

async fn set_limit(State(app): State<App>, h: HeaderMap, Path(id): Path<u64>, Json(b): Json<LimitBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    match app.mgr.set_speed_limit(id, b.limit) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

/// *Start queue* / *Stop queue* for the scheduled downloads.
async fn queue(State(app): State<App>, h: HeaderMap, Path(what): Path<String>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let n = match what.as_str() {
        "start" => app.mgr.start_queue(),
        "stop" => app.mgr.stop_queue(),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    Json(serde_json::json!({ "count": n })).into_response()
}

/// Stop waiting for the browser to supply a new address.
async fn refresh_cancel(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    app.mgr.disarm_refresh();
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
struct ExportQuery {
    #[serde(default)]
    format: Option<String>,
}

/// The download list as IDM's `.ef2` or a plain URL list. Cookies are never included: an export
/// file is the kind of thing people share.
async fn export(State(app): State<App>, h: HeaderMap, Query(q): Query<ExportQuery>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let items = app.mgr.list();
    let body = match q.format.as_deref().unwrap_or("txt") {
        "ef2" => {
            let entries: Vec<reidm_core::lists::Entry> = items
                .iter()
                .map(|i| reidm_core::lists::Entry {
                    url: i.request.url.clone(),
                    referer: i.request.referer.clone(),
                    cookie: None,
                    user_agent: i.request.user_agent.clone(),
                    post_data: None,
                })
                .collect();
            reidm_core::lists::write_ef2(&entries)
        }
        "txt" => {
            let mut urls: Vec<String> = items.iter().map(|i| i.request.url.clone()).collect();
            urls.sort();
            urls.dedup();
            reidm_core::grabber::url_list_text(&urls)
        }
        other => return (StatusCode::BAD_REQUEST, format!("unknown format {other:?} (use ef2 or txt)")).into_response(),
    };
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
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

#[derive(Deserialize, Default)]
struct PingQuery {
    /// The extension's version, so the window can show which one is connected.
    #[serde(default)]
    v: Option<String>,
}

/// What the extension answers on start-up to decide whether reIDM is running.
async fn ext_ping(State(app): State<App>, h: HeaderMap, Query(q): Query<PingQuery>) -> Response {
    if let Err(c) = extension_caller(&h) {
        return c.into_response();
    }
    // Chrome attaches `Origin` to an extension's POSTs but not to its GETs, so the version
    // parameter, which only reIDM's extension sends, is what identifies the ping.
    if q.v.is_some() || is_extension_origin(&h) {
        let browser = h.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(browser_name).unwrap_or_default();
        *app.ext.lock().unwrap() = Some(ExtSeen {
            at: std::time::SystemTime::now(),
            version: q.v.unwrap_or_default().chars().take(32).collect(),
            browser,
        });
    }
    let s = app.mgr.settings();
    Json(serde_json::json!({
        "app": "reIDM",
        "version": env!("CARGO_PKG_VERSION"),
        "default_dir": s.default_dir.to_string_lossy(),
        "connections": s.connections,
        "max_concurrent": s.max_concurrent,
        // Video pages can be handed over for yt-dlp to resolve (`media` in /ext/add).
        "media": true,
    }))
    .into_response()
}

/// The browser's name and major version from its user agent, for display only.
fn browser_name(ua: &str) -> String {
    for (token, name) in [("Edg/", "Edge"), ("OPR/", "Opera"), ("Vivaldi/", "Vivaldi"), ("Brave/", "Brave"), ("Chrome/", "Chrome"), ("Firefox/", "Firefox")] {
        if let Some(rest) = ua.split(token).nth(1) {
            let major = rest.split(['.', ' ']).next().unwrap_or("");
            return format!("{name} {major}").trim().to_string();
        }
    }
    String::new()
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
    /// Check the address works before accepting. The extension asks for this when it is
    /// holding a browser download: a refusal hands the download back to the browser intact.
    #[serde(default)]
    verify: bool,
    /// With `verify`: refuse an address that turns out to be a web page.
    #[serde(default)]
    refuse_html: bool,
    #[serde(default)]
    later: bool,
    /// A video the extension found on the page `url`, for yt-dlp to fetch in this format.
    #[serde(default)]
    media: Option<Media>,
}

fn ext_error(e: anyhow::Error) -> Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "ok": false, "error": format!("{e:#}") }))).into_response()
}

async fn ext_add(State(app): State<App>, h: HeaderMap, Json(b): Json<ExtAdd>) -> Response {
    if let Err(c) = extension_caller(&h) {
        return c.into_response();
    }
    let req = Request {
        url: b.url.trim().to_string(),
        filename: b.filename.filter(|f| !f.trim().is_empty()),
        referer: b.referer.filter(|r| !r.is_empty()),
        cookie: b.cookie.filter(|c| !c.is_empty()),
        user_agent: b.user_agent.filter(|u| !u.is_empty()),
        dir: None,
        media: b.media,
    };

    // A download waiting for a fresh address takes the next link that is the same file.
    if let Some(id) = app.mgr.refresh_armed().filter(|_| req.media.is_none()) {
        match app.mgr.refresh_address(id, req.clone()).await {
            Ok(()) => {
                app.mgr.disarm_refresh();
                tracing::info!("download #{id} continues from a new address");
                return Json(serde_json::json!({ "ok": true, "id": id, "outcome": "refreshed" })).into_response();
            }
            // Some other file: treat it as the ordinary new download it is, and keep waiting.
            Err(e) => tracing::info!("browser link is not the download being refreshed: {e:#}"),
        }
    }

    let opts = AddOptions { later: b.later, refuse_html: b.refuse_html };
    let r = if b.verify { app.mgr.add_checked(req, opts).await } else { app.mgr.add_with(req, opts) };
    match r {
        Ok(o) => Json(serde_json::json!({ "ok": true, "id": o.id, "outcome": o.outcome })).into_response(),
        Err(e) => ext_error(e),
    }
}

#[derive(Deserialize)]
struct ExtBatchItem {
    url: String,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    referer: Option<String>,
    #[serde(default)]
    cookie: Option<String>,
}

#[derive(Deserialize)]
struct ExtBatch {
    items: Vec<ExtBatchItem>,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    later: bool,
}

/// Many links at once, from the extension's resource grabber. Answers per URL.
async fn ext_add_batch(State(app): State<App>, h: HeaderMap, Json(b): Json<ExtBatch>) -> Response {
    if let Err(c) = extension_caller(&h) {
        return c.into_response();
    }
    if b.items.len() > 5000 {
        return ext_error(anyhow::anyhow!("{} links is more than one batch may hold (5000)", b.items.len()));
    }
    let reqs = b
        .items
        .into_iter()
        .map(|i| Request {
            url: i.url.trim().to_string(),
            filename: i.filename.filter(|f| !f.trim().is_empty()),
            referer: i.referer.filter(|r| !r.is_empty()),
            cookie: i.cookie.filter(|c| !c.is_empty()),
            user_agent: b.user_agent.clone().filter(|u| !u.is_empty()),
            dir: None,
            media: None,
        })
        .collect();
    let results: Vec<serde_json::Value> = app
        .mgr
        .add_many_with(reqs, AddOptions { later: b.later, ..Default::default() })
        .into_iter()
        .map(|(url, r)| match r {
            Ok(o) => serde_json::json!({ "url": url, "id": o.id, "outcome": o.outcome }),
            Err(e) => serde_json::json!({ "url": url, "error": format!("{e:#}") }),
        })
        .collect();
    Json(serde_json::json!({ "ok": true, "results": results })).into_response()
}

// ── batch and grabber ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct BatchBody {
    #[serde(default)]
    urls: Vec<String>,
    /// Entries with their own referer/cookie/user agent, e.g. from an `.ef2` import.
    #[serde(default)]
    items: Vec<reidm_core::lists::Entry>,
    #[serde(default)]
    dir: Option<String>,
    /// Applied to every URL in the batch.
    #[serde(default)]
    referer: Option<String>,
    /// Only meaningful for a single URL.
    #[serde(default)]
    filename: Option<String>,
    /// *Download Later* for the whole batch.
    #[serde(default)]
    later: bool,
}

/// Queue many URLs at once. Answers per URL, so the caller can say what happened to each.
async fn add_batch(State(app): State<App>, h: HeaderMap, Json(b): Json<BatchBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let dir = b.dir.filter(|d| !d.trim().is_empty()).map(std::path::PathBuf::from);
    let single = b.urls.len() + b.items.len() == 1;
    let mut reqs: Vec<Request> = b
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
    // A download that came from a form needs its POST body sent again, which reIDM's engine does
    // not do; fetching the address with a GET instead would quietly save some other response.
    let (posts, items): (Vec<_>, Vec<_>) = b.items.into_iter().partition(|e| e.post_data.is_some());
    reqs.extend(items.into_iter().filter(|e| !e.url.trim().is_empty()).map(|e| Request {
        url: e.url.trim().to_string(),
        dir: dir.clone(),
        referer: e.referer.or_else(|| b.referer.clone()),
        cookie: e.cookie,
        user_agent: e.user_agent,
        filename: if single { b.filename.clone() } else { None },
        media: None,
    }));
    let mut results: Vec<serde_json::Value> = app
        .mgr
        .add_many_with(reqs, AddOptions { later: b.later, ..Default::default() })
        .into_iter()
        .map(|(url, r)| match r {
            Ok(o) => serde_json::json!({ "url": url, "id": o.id, "outcome": o.outcome }),
            Err(e) => serde_json::json!({ "url": url, "error": format!("{e:#}") }),
        })
        .collect();
    results.extend(posts.into_iter().map(|e| {
        serde_json::json!({ "url": e.url, "error": "this download came from a form (POST), which reIDM cannot repeat; start it from the page in the browser" })
    }));
    Json(serde_json::json!({ "results": results })).into_response()
}

/// A page to scan, and how.
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
    #[serde(default)]
    speed_limit: Option<u64>,
    #[serde(default)]
    limit_enabled: Option<bool>,
    #[serde(default)]
    schedule: Option<reidm_core::schedule::Schedule>,
}

/// What the daemon is actually able to offer right now.
async fn status(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let ext = app.ext.lock().unwrap().clone().map(|e| {
        serde_json::json!({
            "seconds_ago": e.at.elapsed().map(|d| d.as_secs()).unwrap_or(0),
            "version": e.version,
            "browser": e.browser,
        })
    });
    Json(serde_json::json!({
        "idm_port": app.idm_port.load(std::sync::atomic::Ordering::Relaxed),
        "idm_port_number": crate::ws::IDM_PORT,
        "local_port": crate::ws::LOCAL_PORT,
        "ui_port": UI_PORT,
        "platform": std::env::consts::OS,
        "version": env!("CARGO_PKG_VERSION"),
        "extension": ext,
        "refresh_armed": app.mgr.refresh_armed(),
        "extension_dir": crate::install::extension_dir().map(|p| p.to_string_lossy().into_owned()),
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
        "speed_limit": s.speed_limit,
        "limit_enabled": s.limit_enabled,
        "schedule": s.schedule,
    }))
    .into_response()
}

async fn set_settings(State(app): State<App>, h: HeaderMap, Json(b): Json<SettingsBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let dir = b.default_dir.map(std::path::PathBuf::from).filter(|d| !d.as_os_str().is_empty());
    let patch = SettingsPatch {
        default_dir: dir,
        connections: b.connections,
        max_concurrent: b.max_concurrent,
        speed_limit: b.speed_limit,
        limit_enabled: b.limit_enabled,
        schedule: b.schedule,
    };
    match app.mgr.update_settings(patch) {
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
    let app = App { mgr, token: Arc::new(token), idm_port, ext: Default::default() };
    let router = Router::new()
        .route("/", get(index))
        .route("/api/downloads", get(list).post(add))
        .route("/api/downloads/:id", delete(remove))
        .route("/api/downloads/:id/refresh", post(refresh))
        .route("/api/downloads/:id/limit", post(set_limit))
        .route("/api/downloads/:id/:what", post(act))
        .route("/api/queue/:what", post(queue))
        .route("/api/refresh-capture/cancel", post(refresh_cancel))
        .route("/api/export", get(export))
        .route("/api/settings", get(settings).post(set_settings))
        .route("/api/status", get(status))
        .route("/api/downloads/batch", post(add_batch))
        .route("/api/grab", post(grab))
        .route("/ext/ping", get(ext_ping).options(ext_preflight))
        .route("/ext/add", post(ext_add).options(ext_preflight))
        .route("/ext/add-batch", post(ext_add_batch).options(ext_preflight))
        .with_state(app);
    tracing::info!("web UI at http://127.0.0.1:{UI_PORT}/");
    let r = axum::serve(l, router).await;
    clear_token_file(&data_dir);
    r?;
    Ok(())
}
