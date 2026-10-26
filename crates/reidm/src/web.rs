//! Local web UI + JSON API (served on 127.0.0.1 only).
//!
//! A web page on the internet can reach localhost, so every request is checked:
//! the `Host` header must be our own address (blocks DNS rebinding), mutating calls need the
//! per-run token that is only embedded in the page we serve, and cross-origin `Origin`s are refused.

use crate::rows::{Answer, RowCache};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use reidm_core::checksum::Algo;
use reidm_core::{AddOptions, Category, Checksum, Manager, Media, Move, Performance, Proxy, Request, SettingsPatch, WhenDone};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The port the API is served on, unless `REIDM_UI_PORT` says otherwise. The override exists so
/// a second, throw-away service (a test, a measurement) can run beside the real one; the browser
/// extension always dials this one.
pub const DEFAULT_UI_PORT: u16 = 61002;
const PAGE: &str = include_str!("../assets/ui.html");

/// Request bodies are small except where a file or a long list rides along. Anything bigger is
/// refused before it is read, with a message saying so (see `explain_too_large`).
const SMALL_BODY: usize = 64 * 1024;
/// A `.torrent` file, base64 inside JSON: base64 makes the engine's largest torrent a third
/// bigger, plus room for the rest of the request (14.3 MiB for the 10 MiB cap).
const TORRENT_BODY: usize = reidm_core::torrent::MAX_TORRENT_FILE / 3 * 4 + 1024 * 1024;
/// A batch of URLs, or the extension's grabber with a cookie on every link.
const BATCH_BODY: usize = 8 * 1024 * 1024;
/// The most downloads one batch or bulk request may name.
const MAX_BATCH: usize = 10_000;
/// How long an `/ext/add` answer is remembered for a repeat of the same `request_id`.
const REQUEST_ID_KEPT: Duration = Duration::from_secs(600);
/// The longest `request_id` accepted.
const MAX_REQUEST_ID: usize = 64;
/// How many answers are remembered at most: a caller sending endless new ids can't make the
/// service hold endless answers.
const MAX_REQUEST_IDS: usize = 4096;
/// After *exit when done*, how long the service answers `exiting: true` before it stops, so a
/// window polling its status (every few seconds at most) learns to close too.
const EXIT_GRACE: Duration = Duration::from_secs(6);

/// An `/ext/add` answer, shared by every request with the same `request_id`.
type Answer1 = Arc<tokio::sync::OnceCell<(StatusCode, serde_json::Value)>>;

pub fn ui_port() -> u16 {
    port_from_env("REIDM_UI_PORT", DEFAULT_UI_PORT)
}

pub fn port_from_env(var: &str, default: u16) -> u16 {
    std::env::var(var).ok().and_then(|v| v.trim().parse().ok()).filter(|p| *p != 0).unwrap_or(default)
}

#[derive(Clone)]
pub struct App {
    mgr: Manager,
    token: Arc<String>,
    /// The port this API is served on, which the `Host` header must name.
    port: u16,
    /// Random per run: part of every `ETag` and list cursor, so one from a previous run of the
    /// service is never mistaken for a current one.
    boot: u64,
    /// When reIDM's own extension last checked in, so the window can say whether browser
    /// capture is actually connected rather than guessing.
    ext: Arc<Mutex<Option<ExtSeen>>>,
    /// The list as rows, and what changed in it when (`GET /api/downloads/rows`).
    rows: Arc<Mutex<RowCache>>,
    /// Set to `true` to stop the service (`POST /api/shutdown`).
    shutdown: Arc<tokio::sync::watch::Sender<bool>>,
    /// The service is about to stop because everything is done (`exiting` in the status).
    exiting: Arc<AtomicBool>,
    exit_grace: Duration,
    /// Answers to `/ext/add` by `request_id`, so a repeat (the extension retrying after a
    /// timeout) gets the first answer instead of a second download.
    ext_answers: Arc<Mutex<HashMap<String, (Instant, Answer1)>>>,
}

impl App {
    pub fn new(mgr: Manager, port: u16, shutdown: Arc<tokio::sync::watch::Sender<bool>>) -> App {
        // Its own random value rather than part of the token: the boot id is sent in every
        // `ETag`, and nothing about the token should ever be.
        let boot = u64::from_str_radix(&random_token()[..16], 16).unwrap_or(0);
        App {
            mgr,
            token: Arc::new(random_token()),
            port,
            boot,
            ext: Default::default(),
            rows: Arc::new(Mutex::new(RowCache::new(boot))),
            shutdown,
            exiting: Arc::default(),
            exit_grace: EXIT_GRACE,
            ext_answers: Arc::default(),
        }
    }

    /// Stop the service because everything is done: the status says `exiting` for a moment (a
    /// window then quits too), then it stops as `POST /api/shutdown` would.
    pub fn begin_exit(&self) {
        if self.exiting.swap(true, Ordering::SeqCst) {
            return;
        }
        let (stop, grace) = (self.shutdown.clone(), self.exit_grace);
        tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            let _ = stop.send(true);
        });
    }
}

#[derive(Clone)]
struct ExtSeen {
    at: std::time::SystemTime,
    version: String,
    browser: String,
}

/// Run blocking engine work (file system, a full-list clone) off the async workers, which also
/// serve the browser extension and must not stall behind a slow disk.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(f).await {
        Ok(v) => v,
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
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

/// Our own address as the `Host` header names it, or `None` for any other: a page that rebinds
/// its own host name to 127.0.0.1 still sends that name.
fn our_host(h: &HeaderMap, port: u16) -> Option<&str> {
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok())?;
    let (name, p) = host.rsplit_once(':')?;
    (p.parse::<u16>().ok()? == port && matches!(name, "127.0.0.1" | "localhost" | "[::1]")).then_some(host)
}

/// Compare a secret without stopping at the first difference, so how long a wrong guess takes
/// says nothing about how much of it was right.
fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && std::hint::black_box(a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y))) == 0
}

fn guard(app: &App, h: &HeaderMap, need_token: bool) -> Result<(), StatusCode> {
    let Some(host) = our_host(h, app.port) else { return Err(StatusCode::FORBIDDEN) };
    if let Some(o) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if o != format!("http://{host}") {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    if need_token {
        let given = h.get("x-reidm-token").map(|v| v.as_bytes()).unwrap_or_default();
        if !same_secret(given, app.token.as_bytes()) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    Ok(())
}

/// Does `If-None-Match` name `tag`?
fn etag_matches(h: &HeaderMap, tag: &str) -> bool {
    h.get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|t| t.trim().trim_start_matches("W/") == tag)
}

fn with_etag(mut r: Response, tag: &str) -> Response {
    if let Ok(v) = tag.parse() {
        r.headers_mut().insert(header::ETAG, v);
    }
    // Revalidate every time: the list is live, and a 304 costs next to nothing.
    r.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache"));
    r
}

fn json_bytes(body: Vec<u8>) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// "No such download", as every per-download call says it.
fn not_found(id: u64) -> Response {
    (StatusCode::NOT_FOUND, format!("there is no download #{id} (it may have been removed)")).into_response()
}

async fn exists(mgr: &Manager, id: u64) -> bool {
    let mgr = mgr.clone();
    blocking(move || mgr.get(id).is_some()).await
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

/// Every download with every field. Answers `304 Not Modified` to an `If-None-Match` naming the
/// list as it still is, so a client that polls costs nothing while nothing changes.
async fn list(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    // The revision first, the list after: the list is then never older than its tag.
    let mgr = app.mgr.clone();
    let rev = blocking(move || mgr.revision()).await;
    let tag = format!("\"{:x}-r{rev:x}\"", app.boot);
    if etag_matches(&h, &tag) {
        return with_etag(StatusCode::NOT_MODIFIED.into_response(), &tag);
    }
    let mgr = app.mgr.clone();
    let body = blocking(move || {
        let mut items = mgr.list();
        for i in &mut items {
            i.request.cookie = None; // never expose session cookies to the page
        }
        serde_json::to_vec(&items).unwrap_or_default()
    })
    .await;
    with_etag(json_bytes(body), &tag)
}

#[derive(Deserialize)]
struct RowsQuery {
    #[serde(default)]
    since: Option<String>,
}

/// The list as the window shows it: small rows, and after the first request only the rows that
/// changed (see `rows.rs`). The cursor comes back as `cursor` and as the `ETag`; send it as
/// `?since=` or `If-None-Match`.
async fn rows(State(app): State<App>, h: HeaderMap, Query(q): Query<RowsQuery>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let since = q.since.or_else(|| h.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).map(str::to_string));
    let (mgr, cache) = (app.mgr.clone(), app.rows.clone());
    let (cursor, answer) = blocking(move || {
        // The revision first: what changed is then looked up as of then or later, never earlier.
        let rev = mgr.revision();
        let mut c = cache.lock().unwrap_or_else(|p| p.into_inner());
        if !c.is_current(rev) {
            match c.revision().and_then(|was| mgr.changed_since(was)) {
                Some(ids) => c.update(rev, &mgr.get_many(&ids)),
                None => c.refresh(rev, &mgr.list()),
            }
        }
        (c.cursor(), c.answer(since.as_deref()))
    })
    .await;
    let tag = format!("\"{cursor}\"");
    match answer {
        Answer::NotModified => with_etag(StatusCode::NOT_MODIFIED.into_response(), &tag),
        Answer::Body(b) => with_etag(json_bytes(b), &tag),
    }
}

/// One download with every field, for the Properties window: its segment map and a torrent's
/// files, which the rows leave out.
async fn get_one(State(app): State<App>, h: HeaderMap, Path(id): Path<u64>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let mgr = app.mgr.clone();
    match blocking(move || mgr.get(id)).await {
        Some(mut it) => {
            it.request.cookie = None;
            Json(it).into_response()
        }
        None => not_found(id),
    }
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
    /// A torrent: `url` is a magnet link or a `.torrent` address — or empty, with the
    /// `.torrent` file itself in `data`.
    #[serde(default)]
    torrent: Option<TorrentBody>,
    /// The checksum the finished file should have.
    #[serde(default)]
    checksum: Option<Checksum>,
}

#[derive(Deserialize)]
struct TorrentBody {
    /// The files to download, by index; all when missing.
    #[serde(default)]
    files: Option<Vec<usize>>,
    /// A `.torrent` file, base64.
    #[serde(default)]
    data: Option<String>,
}

fn decode_torrent(data: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| format!("the .torrent file is not base64: {e}"))
}

async fn add(State(app): State<App>, h: HeaderMap, Json(b): Json<AddBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    // A .torrent file sent whole.
    if let Some(data) = b.torrent.as_ref().and_then(|t| t.data.as_deref()) {
        let bytes = match decode_torrent(data) {
            Ok(b) => b,
            Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
        };
        let (mgr, files, later) = (app.mgr.clone(), b.torrent.as_ref().and_then(|t| t.files.clone()), b.later);
        return match blocking(move || mgr.add_torrent_file(&bytes, files, later)).await {
            Ok(o) => Json(serde_json::json!({ "id": o.id, "outcome": o.outcome })).into_response(),
            Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
        };
    }
    let req = Request {
        url: b.url.trim().into(),
        referer: b.referer,
        filename: b.filename.filter(|f| !f.trim().is_empty()),
        cookie: b.cookie,
        user_agent: b.user_agent,
        dir: b.dir.filter(|d| !d.trim().is_empty()).map(std::path::PathBuf::from),
        media: b.media,
        torrent: b.torrent.as_ref().map(|t| reidm_core::TorrentRequest { files: t.files.clone(), ..Default::default() }),
        checksum: b.checksum,
    };
    let (mgr, later) = (app.mgr.clone(), b.later);
    match blocking(move || mgr.add_with(req, AddOptions { later, ..Default::default() })).await {
        Ok(o) => Json(serde_json::json!({ "id": o.id, "outcome": o.outcome })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

async fn act(State(app): State<App>, h: HeaderMap, Path((id, what)): Path<(u64, String)>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    if !ACTIONS.contains(&what.as_str()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !exists(&app.mgr, id).await {
        return not_found(id);
    }
    match act_on(&app.mgr, id, &what).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// What `POST /api/downloads/{id}/{what}` and the bulk call can do to a download.
const ACTIONS: [&str; 10] = ["pause", "resume", "open", "folder", "restart", "seed", "stop-seeding", "schedule", "unschedule", "refresh-capture"];

/// Do one of `ACTIONS` (or `remove`/`purge`, for the bulk call) to a download that exists.
async fn act_on(mgr: &Manager, id: u64, what: &str) -> Result<(), String> {
    let m = mgr.clone();
    let r = match what {
        "pause" => blocking(move || {
            m.pause(id);
            Ok(())
        }).await,
        "resume" => blocking(move || m.resume(id)).await,
        "open" | "folder" => {
            let reveal = what == "folder";
            blocking(move || {
                if let Some(it) = m.get(id) {
                    // A torrent's files are in a folder of their own: that folder is what
                    // "open folder" means, and what "open" means for more than one file.
                    let folder = it.torrent.as_ref().and_then(|t| t.folder.clone().filter(|_| reveal || t.files.len() > 1));
                    match folder {
                        Some(f) => crate::launch::open_path(&f, false),
                        None => crate::launch::open_path(&it.path, reveal),
                    }
                }
                Ok(())
            })
            .await
        }
        "restart" => m.restart(id).await,
        "seed" | "stop-seeding" => m.set_seeding(id, what == "seed").await,
        "schedule" | "unschedule" => {
            let on = what == "schedule";
            blocking(move || m.set_scheduled(id, on)).await
        }
        "remove" | "purge" => {
            let purge = what == "purge";
            blocking(move || {
                m.remove(id, purge);
                Ok(())
            }).await
        }
        // Take the next link the browser hands over as this download's new address.
        "refresh-capture" => blocking(move || m.arm_refresh(id)).await,
        other => return Err(format!("unknown action {other:?}")),
    };
    r.map_err(|e| format!("{e:#}"))
}

#[derive(Deserialize)]
struct MoveBody {
    /// `top`, `up`, `down` or `bottom`.
    to: String,
}

fn parse_move(to: &str) -> Result<Move, String> {
    Move::from_name(to).ok_or_else(|| format!("cannot move a download {to:?}: use top, up, down or bottom"))
}

/// Move a download in the queue.
async fn move_one(State(app): State<App>, h: HeaderMap, Path(id): Path<u64>, Json(b): Json<MoveBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let to = match parse_move(&b.to) {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    if !exists(&app.mgr, id).await {
        return not_found(id);
    }
    let mgr = app.mgr.clone();
    match blocking(move || mgr.move_in_queue(&[id], to)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

#[derive(Deserialize)]
struct VerifyBody {
    /// `md5`, `sha1`, `sha256` (when left out) or `sha512`.
    #[serde(default)]
    algo: Option<String>,
}

/// A finished download's checksum, worked out now: Properties' *Integrity* row.
async fn verify(State(app): State<App>, h: HeaderMap, Path(id): Path<u64>, Json(b): Json<VerifyBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let name = b.algo.unwrap_or_else(|| "sha256".into());
    let Some(algo) = Algo::from_name(&name) else {
        return (StatusCode::BAD_REQUEST, format!("{name:?} is not a checksum reIDM knows: use md5, sha1, sha256 or sha512")).into_response();
    };
    if !exists(&app.mgr, id).await {
        return not_found(id);
    }
    match app.mgr.checksum_of(id, algo).await {
        Ok(value) => Json(serde_json::json!({ "algo": algo, "value": value })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

#[derive(Deserialize)]
struct BulkBody {
    ids: Vec<u64>,
    action: String,
    /// For `move`: where to.
    #[serde(default)]
    to: Option<String>,
}

/// One action on many downloads, in one request: *Delete completed*, *Start all*, a selection of
/// thousands. Answers per download, like the batch add.
async fn bulk(State(app): State<App>, h: HeaderMap, Json(b): Json<BulkBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let action = b.action.as_str();
    if !(ACTIONS.contains(&action) || matches!(action, "remove" | "purge" | "move")) || matches!(action, "open" | "folder" | "refresh-capture") {
        return (StatusCode::BAD_REQUEST, format!("unknown bulk action {action:?}: use pause, resume, restart, remove, purge, schedule, unschedule, seed, stop-seeding or move")).into_response();
    }
    if b.ids.len() > MAX_BATCH {
        return (StatusCode::BAD_REQUEST, format!("{} downloads is more than one request may name ({MAX_BATCH}); send them in parts", b.ids.len())).into_response();
    }
    let mgr = app.mgr.clone();
    let ids = b.ids.clone();
    let known: std::collections::HashSet<u64> = blocking(move || mgr.get_many(&ids).into_iter().filter(|(_, it)| it.is_some()).map(|(id, _)| id).collect()).await;
    if action == "move" {
        // One move of the whole selection, so a block keeps its order.
        let to = match parse_move(b.to.as_deref().unwrap_or("")) {
            Ok(t) => t,
            Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
        };
        let present: Vec<u64> = b.ids.iter().copied().filter(|id| known.contains(id)).collect();
        let mgr = app.mgr.clone();
        let moved = blocking(move || mgr.move_in_queue(&present, to)).await.map_err(|e| format!("{e:#}"));
        let results: Vec<serde_json::Value> = b
            .ids
            .iter()
            .map(|id| match (&moved, known.contains(id)) {
                (_, false) => serde_json::json!({ "id": id, "error": format!("there is no download #{id}") }),
                (Ok(()), true) => serde_json::json!({ "id": id }),
                (Err(e), true) => serde_json::json!({ "id": id, "error": e }),
            })
            .collect();
        let ok = results.iter().filter(|r| r.get("error").is_none()).count();
        return Json(serde_json::json!({ "ok": ok, "failed": results.len() - ok, "results": results })).into_response();
    }
    let (mut ok, mut results) = (0usize, Vec::with_capacity(b.ids.len()));
    for id in b.ids {
        let r = if known.contains(&id) { act_on(&app.mgr, id, action).await } else { Err(format!("there is no download #{id}")) };
        results.push(match r {
            Ok(()) => {
                ok += 1;
                serde_json::json!({ "id": id })
            }
            Err(e) => serde_json::json!({ "id": id, "error": e }),
        });
    }
    let failed = results.len() - ok;
    Json(serde_json::json!({ "ok": ok, "failed": failed, "results": results })).into_response()
}

/// Stop the service: *Quit reIDM completely* in the window. Downloads stop where they are and
/// carry on from there the next time it starts.
async fn shutdown(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    tracing::info!("asked to shut down");
    let _ = app.shutdown.send(true);
    StatusCode::ACCEPTED.into_response()
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
    if !exists(&app.mgr, id).await {
        return not_found(id);
    }
    let mgr = app.mgr.clone();
    match blocking(move || mgr.set_speed_limit(id, b.limit)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

/// *Start queue* / *Stop queue* for the scheduled downloads.
async fn queue(State(app): State<App>, h: HeaderMap, Path(what): Path<String>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let start = match what.as_str() {
        "start" => true,
        "stop" => false,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let mgr = app.mgr.clone();
    let n = blocking(move || if start { mgr.start_queue() } else { mgr.stop_queue() }).await;
    Json(serde_json::json!({ "count": n })).into_response()
}

/// Stop waiting for the browser to supply a new address.
async fn refresh_cancel(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let mgr = app.mgr.clone();
    blocking(move || mgr.disarm_refresh()).await;
    StatusCode::NO_CONTENT.into_response()
}

/// The download list as a plain URL list. Cookies are never included: an export file is the
/// kind of thing people share.
async fn export(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let mgr = app.mgr.clone();
    let items = blocking(move || mgr.list()).await;
    let mut urls: Vec<String> = items.iter().map(|i| i.request.url.clone()).collect();
    urls.sort();
    urls.dedup();
    let body = reidm_core::grabber::url_list_text(&urls);
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

// ── endpoints for reIDM's own browser extension ──────────────────────────
//
// These are reached from an extension service worker, which has no API token. The origin check
// is the authorisation instead, exactly as on the WebSocket: a page cannot forge `Origin`, and a
// page cannot add a custom header here either, because we never answer a CORS preflight.
// Extension requests with `host_permissions` are not subject to CORS, so nothing is needed for
// the extension itself to read the responses.

/// Is this an extension's origin? Browsers set `Origin` themselves and page script cannot forge
/// it, so this is a reliable discriminator — a web page always arrives with its own http(s)
/// origin. It is not a *credential*: any local process can send the header, but such a process
/// could already read the daemon's token file. The WebSocket uses the same test.
pub fn is_extension_origin_str(o: &str) -> bool {
    ["chrome-extension://", "moz-extension://", "safari-web-extension://", "extension://"].iter().any(|p| o.starts_with(p))
}

fn is_extension_origin(h: &HeaderMap) -> bool {
    h.get(header::ORIGIN).and_then(|v| v.to_str().ok()).is_some_and(is_extension_origin_str)
}

/// Who may use `/ext/*`: requests addressed to us by our own name (so a page that rebinds its
/// host name to 127.0.0.1 is refused, as on `/api`), from an extension or from a local process.
fn extension_caller(app: &App, h: &HeaderMap) -> Result<(), StatusCode> {
    if our_host(h, app.port).is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    match h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        Some(_) if is_extension_origin(h) => Ok(()),
        // No Origin: a local process (curl, a script). It could already read the token file, so
        // this grants nothing new.
        None => Ok(()),
        Some(_) => Err(StatusCode::FORBIDDEN),
    }
}

/// Whether a ping without an extension `Origin` may count as the extension checking in. Chrome
/// sends no `Origin` on an extension's GET, but it does say where a request came from: a web
/// page's `<img src="http://127.0.0.1:61002/ext/ping?v=x">` or no-cors `fetch` arrives marked
/// `Sec-Fetch-Site: cross-site`, and must not make the window report "Browser: connected".
/// An extension's own request, and a local tool that sends no such header, are not refused.
fn may_check_in(h: &HeaderMap) -> bool {
    is_extension_origin(h)
        || !matches!(h.get("sec-fetch-site").and_then(|v| v.to_str().ok()), Some("cross-site" | "same-site"))
}

/// Answer a CORS preflight.
///
/// Chrome may preflight the extension's JSON POST even though host permissions normally lift
/// the response check, so refusing every preflight would break the extension. The reply is
/// scoped to the asking extension origin, which a web page cannot obtain: it would have to send
/// `Origin: chrome-extension://…`, and browsers do not let page script set that header.
async fn ext_preflight(State(app): State<App>, h: HeaderMap) -> Response {
    if !is_extension_origin(&h) || our_host(&h, app.port).is_none() {
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
    if let Err(c) = extension_caller(&app, &h) {
        return c.into_response();
    }
    // Chrome attaches `Origin` to an extension's POSTs but not to its GETs, so the version
    // parameter, which only reIDM's extension sends, is what identifies the ping.
    if (q.v.is_some() || is_extension_origin(&h)) && may_check_in(&h) {
        let browser = h.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(browser_name).unwrap_or_default();
        *app.ext.lock().unwrap_or_else(|p| p.into_inner()) = Some(ExtSeen {
            at: std::time::SystemTime::now(),
            version: q.v.unwrap_or_default().chars().take(32).collect(),
            browser,
        });
    }
    let s = app.mgr.settings();
    // A download waiting for its new address, for the extension to say which one a click
    // would refresh.
    let mgr = app.mgr.clone();
    let armed = blocking(move || {
        let id = mgr.refresh_armed()?;
        let name = mgr.get(id)?.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Some(serde_json::json!({ "id": id, "name": name }))
    })
    .await;
    Json(serde_json::json!({
        "app": "reIDM",
        "version": env!("CARGO_PKG_VERSION"),
        "refresh_armed": armed,
        "default_dir": s.default_dir.to_string_lossy(),
        "connections": s.connections,
        "max_concurrent": s.max_concurrent,
        // Video pages can be handed over for yt-dlp to resolve (`media` in /ext/add).
        "media": true,
        // Where each type of file goes, for the extension's "Download file info" dialog.
        "folders": folders_json(&app),
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
    /// The checksum the finished file should have.
    #[serde(default)]
    checksum: Option<Checksum>,
    /// Makes the request safe to send again: a repeat with the same id (within ten minutes)
    /// gets the first one's answer — waiting for it, if the first is still being checked —
    /// instead of adding the download a second time.
    #[serde(default)]
    request_id: Option<String>,
    /// This link is the new address of exactly this download (*Refresh download address*):
    /// refused, never added as a new download, when it isn't the same file or that download
    /// is no longer waiting for one.
    #[serde(default)]
    refresh_id: Option<u64>,
    /// This link is a new download: don't try it against a download waiting for a new
    /// address, which stays waiting.
    #[serde(default)]
    no_refresh: bool,
}

fn ext_error(e: anyhow::Error) -> Response {
    let (code, v) = ext_refusal(e);
    (code, Json(v)).into_response()
}

fn ext_refusal(e: anyhow::Error) -> (StatusCode, serde_json::Value) {
    (StatusCode::BAD_REQUEST, serde_json::json!({ "ok": false, "error": format!("{e:#}") }))
}

async fn ext_add(State(app): State<App>, h: HeaderMap, Json(b): Json<ExtAdd>) -> Response {
    if let Err(c) = extension_caller(&app, &h) {
        return c.into_response();
    }
    let Some(rid) = b.request_id.clone() else {
        let (code, v) = ext_add_answer(&app, b).await;
        return (code, Json(v)).into_response();
    };
    if rid.is_empty() || rid.len() > MAX_REQUEST_ID {
        return ext_error(anyhow::anyhow!("a request_id is 1 to {MAX_REQUEST_ID} characters"));
    }
    let cell = {
        let mut seen = app.ext_answers.lock().unwrap_or_else(|p| p.into_inner());
        seen.retain(|_, (at, _)| at.elapsed() < REQUEST_ID_KEPT);
        if seen.len() >= MAX_REQUEST_IDS && !seen.contains_key(&rid) {
            // Full of recent answers: the oldest go. Anything repeating that late has long
            // been answered.
            let mut by_age: Vec<(Instant, String)> = seen.iter().map(|(k, (at, _))| (*at, k.clone())).collect();
            by_age.sort_unstable();
            for (_, k) in by_age.into_iter().take(seen.len() + 1 - MAX_REQUEST_IDS) {
                seen.remove(&k);
            }
        }
        seen.entry(rid).or_insert_with(|| (Instant::now(), Answer1::default())).1.clone()
    };
    // The first request with this id does the work; a repeat waits for its answer. If the first
    // is dropped half way (its caller went away), the next one does the work instead.
    let (code, v) = cell.get_or_init(|| ext_add_answer(&app, b)).await.clone();
    (code, Json(v)).into_response()
}

/// What `/ext/add` answers to `b`.
async fn ext_add_answer(app: &App, b: ExtAdd) -> (StatusCode, serde_json::Value) {
    let req = Request {
        url: b.url.trim().to_string(),
        filename: b.filename.filter(|f| !f.trim().is_empty()),
        referer: b.referer.filter(|r| !r.is_empty()),
        cookie: b.cookie.filter(|c| !c.is_empty()),
        user_agent: b.user_agent.filter(|u| !u.is_empty()),
        dir: None,
        media: b.media,
        torrent: None,
        checksum: b.checksum,
    };
    let refreshed = |id: u64| (StatusCode::OK, serde_json::json!({ "ok": true, "id": id, "outcome": "refreshed" }));

    // The link the user clicked to give one download its new address: that, or nothing. Adding
    // it as a new download instead would take a held browser download away from the browser
    // for something the user didn't ask for.
    if let Some(id) = b.refresh_id {
        if app.mgr.refresh_armed() != Some(id) {
            return ext_refusal(anyhow::anyhow!(
                "download #{id} is no longer waiting for a new address (its three minutes ran out, or it was cancelled); choose Refresh download address in reIDM again"
            ));
        }
        return match app.mgr.refresh_address(id, req).await {
            Ok(()) => {
                app.mgr.disarm_refresh();
                tracing::info!("download #{id} continues from a new address");
                refreshed(id)
            }
            Err(e) => ext_refusal(e.context(format!("this link can't be the new address of download #{id}"))),
        };
    }

    // A download waiting for a fresh address takes the next link that is the same file.
    if let Some(id) = app.mgr.refresh_armed().filter(|_| req.media.is_none() && !b.no_refresh) {
        match app.mgr.refresh_address(id, req.clone()).await {
            Ok(()) => {
                app.mgr.disarm_refresh();
                tracing::info!("download #{id} continues from a new address");
                return refreshed(id);
            }
            // Some other file: treat it as the ordinary new download it is, and keep waiting.
            Err(e) => tracing::info!("browser link is not the download being refreshed: {e:#}"),
        }
    }

    let opts = AddOptions { later: b.later, refuse_html: b.refuse_html };
    let r = if b.verify {
        app.mgr.add_checked(req, opts).await
    } else {
        let mgr = app.mgr.clone();
        blocking(move || mgr.add_with(req, opts)).await
    };
    match r {
        Ok(o) => (StatusCode::OK, serde_json::json!({ "ok": true, "id": o.id, "outcome": o.outcome })),
        Err(e) => ext_refusal(e),
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
    if let Err(c) = extension_caller(&app, &h) {
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
            torrent: None,
            checksum: None,
        })
        .collect();
    let (mgr, later) = (app.mgr.clone(), b.later);
    let results: Vec<serde_json::Value> = blocking(move || mgr.add_many_with(reqs, AddOptions { later, ..Default::default() }))
        .await
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
    if b.urls.len() > MAX_BATCH {
        return (StatusCode::BAD_REQUEST, format!("{} addresses is more than one batch may hold ({MAX_BATCH}); add them in parts", b.urls.len())).into_response();
    }
    let dir = b.dir.filter(|d| !d.trim().is_empty()).map(std::path::PathBuf::from);
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
    let (mgr, later) = (app.mgr.clone(), b.later);
    let results: Vec<serde_json::Value> = blocking(move || mgr.add_many_with(reqs, AddOptions { later, ..Default::default() }))
        .await
        .into_iter()
        .map(|(url, r)| match r {
            Ok(o) => serde_json::json!({ "url": url, "id": o.id, "outcome": o.outcome }),
            Err(e) => serde_json::json!({ "url": url, "error": format!("{e:#}") }),
        })
        .collect();
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
#[derive(Deserialize)]
struct UrlBody {
    url: String,
}

/// The videos of a playlist, for the window's *Add YouTube playlist*. Runs yt-dlp, so the first
/// call can take a while (reIDM fetches yt-dlp first).
async fn media_playlist(State(app): State<App>, h: HeaderMap, Json(b): Json<UrlBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    match app.mgr.playlist(&b.url).await {
        Ok(p) => Json(p).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

#[derive(Deserialize)]
struct InspectBody {
    #[serde(default)]
    url: Option<String>,
    /// A `.torrent` file, base64.
    #[serde(default)]
    data: Option<String>,
}

/// What a torrent holds, to choose its files before adding it. For a magnet link this waits for
/// a peer to describe it (up to two minutes).
async fn torrent_inspect(State(app): State<App>, h: HeaderMap, Json(b): Json<InspectBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let bytes = match b.data.as_deref().map(decode_torrent).transpose() {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    match app.mgr.inspect_torrent(b.url.as_deref(), bytes).await {
        Ok(i) => Json(i).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

/// The heights and codecs one video comes in.
async fn media_info(State(app): State<App>, h: HeaderMap, Json(b): Json<UrlBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    match app.mgr.video_info(&b.url).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

async fn grab(State(app): State<App>, h: HeaderMap, Json(b): Json<GrabBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    // Through the proxy the downloads use: a network that needs one for them needs it here.
    match reidm_core::grabber::grab_through(&b.url, &b.options, &app.mgr.settings().proxy).await {
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
    #[serde(default)]
    sort_by_category: Option<bool>,
    /// Every category's folder; an empty or missing one is its default.
    #[serde(default)]
    category_dirs: Option<std::collections::BTreeMap<Category, String>>,
    /// The temporary folder; empty is the default.
    #[serde(default)]
    temp_dir: Option<String>,
    #[serde(default)]
    seed_ratio: Option<f64>,
    #[serde(default)]
    upload_limit: Option<u64>,
    #[serde(default)]
    torrent_port: Option<u16>,
    #[serde(default)]
    torrent_dht: Option<bool>,
    #[serde(default)]
    performance: Option<Performance>,
    #[serde(default)]
    write_buffer_kib: Option<usize>,
    #[serde(default)]
    retries: Option<u32>,
    #[serde(default)]
    retry_wait_secs: Option<u64>,
    #[serde(default)]
    idle_timeout_secs: Option<u64>,
    #[serde(default)]
    connect_timeout_secs: Option<u64>,
    /// The whole proxy. Without `password`, the one already set is kept; `""` removes it.
    #[serde(default)]
    proxy: Option<Proxy>,
    #[serde(default)]
    keep_awake: Option<bool>,
    #[serde(default)]
    keep_awake_seeding: Option<bool>,
    #[serde(default)]
    min_free_space_mib: Option<u64>,
}

/// The folders, as the window's Folders tab and the extension's dialog show them: each
/// category's folder in effect, which ones the user chose, and the temporary folder.
fn folders_json(app: &App) -> serde_json::Value {
    let s = app.mgr.settings();
    let dirs: serde_json::Map<String, serde_json::Value> = reidm_core::category::ALL.iter().map(|c| (c.name().to_string(), s.folder(*c).to_string_lossy().into_owned().into())).collect();
    let custom: Vec<&str> = s.category_dirs.keys().map(|c| c.name()).collect();
    let types: serde_json::Map<String, serde_json::Value> = reidm_core::category::ALL.iter().map(|c| (c.name().to_string(), c.extensions().into())).collect();
    serde_json::json!({
        "sort": s.sort_by_category,
        "dirs": dirs,
        "custom": custom,
        "types": types,
        "temp_dir": app.mgr.temp_dir().to_string_lossy(),
        "temp_custom": s.temp_dir.is_some(),
    })
}

/// What the daemon is actually able to offer right now.
async fn status(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let ext = app.ext.lock().unwrap_or_else(|p| p.into_inner()).clone().map(|e| {
        serde_json::json!({
            "seconds_ago": e.at.elapsed().map(|d| d.as_secs()).unwrap_or(0),
            "version": e.version,
            "browser": e.browser,
        })
    });
    // Asks the file system: off the async workers, in case a drive is slow to answer.
    let mgr = app.mgr.clone();
    let free: Vec<serde_json::Value> = blocking(move || mgr.free_space())
        .await
        .into_iter()
        .map(|(path, free)| serde_json::json!({ "path": path.to_string_lossy(), "free": free }))
        .collect();
    Json(serde_json::json!({
        "ui_port": app.port,
        "platform": std::env::consts::OS,
        "version": env!("CARGO_PKG_VERSION"),
        "extension": ext,
        "refresh_armed": app.mgr.refresh_armed(),
        "extension_dir": crate::install::extension_dir().map(|p| p.to_string_lossy().into_owned()),
        // What the user should know about the list's storage: a damaged file restored from its
        // backup, saves that are failing.
        "warnings": app.mgr.warnings(),
        // For the status bar: the download folder's drive and the temporary folder's.
        "free_space": free,
        "when_done": when_done_json(&app.mgr),
        // The service is stopping because everything is done: a window should quit too.
        "exiting": app.exiting.load(Ordering::SeqCst),
    }))
    .into_response()
}

fn when_done_json(mgr: &Manager) -> serde_json::Value {
    let (action, fires_at) = mgr.when_done();
    serde_json::json!({ "action": action, "fires_at": fires_at })
}

/// What happens when every download is done, and the countdown to it.
async fn when_done(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    Json(when_done_json(&app.mgr)).into_response()
}

#[derive(Deserialize)]
struct WhenDoneBody {
    action: String,
}

/// Choose what happens when every download is done (`none` for nothing). Not saved: it lasts
/// until the service stops.
async fn set_when_done(State(app): State<App>, h: HeaderMap, Json(b): Json<WhenDoneBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let Some(action) = WhenDone::from_name(&b.action) else {
        return (StatusCode::BAD_REQUEST, format!("{:?} is not something reIDM can do when downloads are done: use none, exit, sleep, hibernate or shutdown", b.action)).into_response();
    };
    app.mgr.set_when_done(action);
    Json(when_done_json(&app.mgr)).into_response()
}

/// Cancel the countdown, and the action with it.
async fn cancel_when_done(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    app.mgr.set_when_done(WhenDone::None);
    Json(when_done_json(&app.mgr)).into_response()
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
        "folders": folders_json(&app),
        "seed_ratio": s.seed_ratio,
        "upload_limit": s.upload_limit,
        "torrent_port": s.torrent_port,
        "torrent_dht": s.torrent_dht,
        "performance": s.performance,
        // What each preset sets, so the window shows the numbers without keeping its own copy.
        "profiles": profiles_json(),
        "write_buffer_kib": s.write_buffer_kib,
        "retries": s.retries,
        "retry_wait_secs": s.retry_wait_secs,
        "idle_timeout_secs": s.idle_timeout_secs,
        "connect_timeout_secs": s.connect_timeout_secs,
        "proxy": proxy_json(&s.proxy),
        "keep_awake": s.keep_awake,
        "keep_awake_seeding": s.keep_awake_seeding,
        "min_free_space_mib": s.min_free_space_mib,
    }))
    .into_response()
}

/// The proxy as the API shows it: the password is never sent back, only whether there is one.
fn proxy_json(p: &Proxy) -> serde_json::Value {
    let (shown, set) = p.without_password();
    let mut v = serde_json::to_value(shown).unwrap_or_default();
    v["password_set"] = set.into();
    v
}

fn profiles_json() -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for p in [Performance::Light, Performance::Balanced, Performance::Maximum] {
        if let Some(pr) = p.profile() {
            let name = serde_json::to_value(p).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            m.insert(
                name,
                serde_json::json!({ "connections": pr.connections, "max_concurrent": pr.max_concurrent, "write_buffer_kib": pr.write_buffer_kib, "torrent_peers": pr.torrent_peers }),
            );
        }
    }
    m.into()
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
        sort_by_category: b.sort_by_category,
        category_dirs: b.category_dirs.map(|m| m.into_iter().filter(|(_, d)| !d.trim().is_empty()).map(|(c, d)| (c, std::path::PathBuf::from(d.trim()))).collect()),
        temp_dir: b.temp_dir.map(|d| std::path::PathBuf::from(d.trim())),
        seed_ratio: b.seed_ratio,
        upload_limit: b.upload_limit,
        torrent_port: b.torrent_port,
        torrent_dht: b.torrent_dht,
        performance: b.performance,
        write_buffer_kib: b.write_buffer_kib,
        retries: b.retries,
        retry_wait_secs: b.retry_wait_secs,
        idle_timeout_secs: b.idle_timeout_secs,
        connect_timeout_secs: b.connect_timeout_secs,
        proxy: b.proxy,
        keep_awake: b.keep_awake,
        keep_awake_seeding: b.keep_awake_seeding,
        min_free_space_mib: b.min_free_space_mib,
    };
    let mgr = app.mgr.clone();
    let (r, applied) = blocking(move || {
        let before = serde_json::to_value(mgr.settings()).ok();
        let r = mgr.update_settings(patch);
        let applied = r.is_err() && serde_json::to_value(mgr.settings()).ok() != before;
        (r, applied)
    })
    .await;
    match r {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        // In effect, but not on disk (a full disk, say): not a refusal of the request, so not a
        // 400 — the caller should say the change will not outlast a restart.
        Err(e) if applied => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
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
    if !exists(&app.mgr, id).await {
        return not_found(id);
    }
    match act_on(&app.mgr, id, if q.purge { "purge" } else { "remove" }).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
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

/// Every route, with its body limit.
pub fn router(app: App) -> Router {
    let big = |n| DefaultBodyLimit::max(n);
    Router::new()
        .route("/", get(index))
        .route("/api/downloads", get(list).post(add).layer(big(TORRENT_BODY)))
        .route("/api/downloads/rows", get(rows))
        .route("/api/downloads/bulk", post(bulk).layer(big(BATCH_BODY)))
        .route("/api/downloads/batch", post(add_batch).layer(big(BATCH_BODY)))
        .route("/api/downloads/:id", get(get_one).delete(remove))
        .route("/api/downloads/:id/refresh", post(refresh))
        .route("/api/downloads/:id/move", post(move_one))
        .route("/api/downloads/:id/verify", post(verify))
        .route("/api/downloads/:id/limit", post(set_limit))
        .route("/api/downloads/:id/:what", post(act))
        .route("/api/queue/:what", post(queue))
        .route("/api/refresh-capture/cancel", post(refresh_cancel))
        .route("/api/export", get(export))
        .route("/api/settings", get(settings).post(set_settings))
        .route("/api/status", get(status))
        .route("/api/shutdown", post(shutdown))
        .route("/api/when-done", get(when_done).post(set_when_done))
        .route("/api/when-done/cancel", post(cancel_when_done))
        .route("/api/media/playlist", post(media_playlist))
        .route("/api/media/info", post(media_info))
        .route("/api/torrent/inspect", post(torrent_inspect).layer(big(TORRENT_BODY)))
        .route("/api/grab", post(grab))
        .route("/ext/ping", get(ext_ping).options(ext_preflight))
        .route("/ext/add", post(ext_add).options(ext_preflight))
        .route("/ext/add-batch", post(ext_add_batch).options(ext_preflight).layer(big(BATCH_BODY)))
        .layer(DefaultBodyLimit::max(SMALL_BODY))
        .layer(axum::middleware::from_fn(explain_too_large))
        .with_state(app)
}

/// axum answers an oversized body with a bare `413`. Say what happened instead, in the shape
/// each caller reads: JSON for the extension, text for the API.
async fn explain_too_large(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let ext = req.uri().path().starts_with("/ext/");
    let r = next.run(req).await;
    if r.status() != StatusCode::PAYLOAD_TOO_LARGE {
        return r;
    }
    let msg = "the request is larger than reIDM accepts for this call; send fewer items at once";
    if ext {
        (StatusCode::PAYLOAD_TOO_LARGE, Json(serde_json::json!({ "ok": false, "error": msg }))).into_response()
    } else {
        (StatusCode::PAYLOAD_TOO_LARGE, msg).into_response()
    }
}

/// Serve the API on a port the caller has already taken. Publishes the token, and removes it
/// again when the service stops: on `POST /api/shutdown`, or when `shutdown` is set.
pub async fn serve(l: tokio::net::TcpListener, app: App) -> anyhow::Result<()> {
    let data_dir = app.mgr.data_dir().to_path_buf();
    write_token_file(&data_dir, &app.token);
    let mut stop = app.shutdown.subscribe();
    let port = app.port;
    tracing::info!("web UI at http://127.0.0.1:{port}/");
    let r = axum::serve(l, router(app))
        .with_graceful_shutdown(async move {
            let _ = stop.wait_for(|s| *s).await;
        })
        .await;
    clear_token_file(&data_dir);
    r?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    struct Server {
        base: String,
        port: u16,
        token: String,
        stop: Arc<tokio::sync::watch::Sender<bool>>,
        dir: std::path::PathBuf,
        mgr: Manager,
        app: App,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.stop.send(true);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// A service on an ephemeral port over a data folder holding `n` finished downloads, written
    /// straight to `downloads.json` (adding them one by one would take minutes).
    async fn server(name: &str, n: u64) -> Server {
        server_with(name, n, None).await
    }

    /// As `server`, with `downloads.json` replaced by `raw` when given.
    async fn server_with(name: &str, n: u64, raw: Option<&[u8]>) -> Server {
        let dir = std::env::temp_dir().join(format!("reidm-web-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let segs: Vec<serde_json::Value> = (0..8u64).map(|k| serde_json::json!({ "start": k << 20, "end": (k << 20) + 1048575, "done": 1048576, "finished": true })).collect();
        let items: serde_json::Map<String, serde_json::Value> = (1..=n)
            .map(|id| {
                (id.to_string(), serde_json::json!({
                    "id": id,
                    "request": { "url": format!("https://downloads.example.com/releases/v{id}/package-{id}.tar.gz?token=abcdef0123456789"), "referer": "https://example.com/download", "cookie": "session=secret", "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/141.0.0.0", "filename": null, "dir": null },
                    "path": format!("/home/user/Downloads/package-{id}.tar.gz"),
                    "size": 8388608, "downloaded": 8388608, "status": { "state": "Done" },
                    "segments": segs, "resumable": true, "added": 1_790_000_000u64, "last_try": 1_790_000_100u64, "finished_at": 1_790_000_100u64,
                    "etag": "\"5f3e-abc\"", "last_modified": "Mon, 01 Jan 2026 00:00:00 GMT",
                }))
            })
            .collect();
        let list = serde_json::to_vec(&serde_json::json!({ "next_id": n + 1, "items": items })).unwrap();
        std::fs::write(dir.join("downloads.json"), raw.unwrap_or(&list)).unwrap();
        let mgr = Manager::open(&dir, dir.join("dl"), 2).unwrap();
        let l = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = l.local_addr().unwrap().port();
        let (stop, _) = tokio::sync::watch::channel(false);
        let stop = Arc::new(stop);
        let mut app = App::new(mgr.clone(), port, stop.clone());
        app.exit_grace = Duration::from_millis(300);
        let token = app.token.to_string();
        tokio::spawn(serve(l, app.clone()));
        // `serve` writes the token file first; wait until it answers.
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Server { base: format!("http://127.0.0.1:{port}"), port, token, stop, dir, mgr, app }
    }

    impl Server {
        fn get(&self, path: &str) -> reqwest::RequestBuilder {
            reqwest::Client::new().get(format!("{}{path}", self.base)).header("X-Reidm-Token", &self.token)
        }
        fn post(&self, path: &str, body: serde_json::Value) -> reqwest::RequestBuilder {
            reqwest::Client::new().post(format!("{}{path}", self.base)).header("X-Reidm-Token", &self.token).json(&body)
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fifty_thousand_downloads_are_served_and_an_unchanged_list_costs_nothing() {
        let s = server("big", 50_000).await;

        let t = Instant::now();
        let r = s.get("/api/downloads").send().await.unwrap();
        assert_eq!(r.status(), 200);
        let tag = r.headers()[header::ETAG].to_str().unwrap().to_string();
        let full = r.bytes().await.unwrap();
        let items: Vec<serde_json::Value> = serde_json::from_slice(&full).unwrap();
        assert_eq!(items.len(), 50_000);
        assert!(items.iter().all(|i| i["request"]["cookie"].is_null()), "cookies never leave the service");
        eprintln!("GET /api/downloads, 50k: {} bytes in {:?}", full.len(), t.elapsed());

        let t = Instant::now();
        let r = s.get("/api/downloads").header(header::IF_NONE_MATCH, &tag).send().await.unwrap();
        assert_eq!(r.status(), 304, "the same list again is a 304");
        assert!(r.bytes().await.unwrap().is_empty());
        eprintln!("GET /api/downloads, 50k, unchanged: 304 in {:?}", t.elapsed());

        let t = Instant::now();
        let r = s.get("/api/downloads/rows").send().await.unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = r.json().await.unwrap();
        assert_eq!(v["full"], true);
        assert_eq!(v["rows"].as_array().unwrap().len(), 50_000);
        let rows_len = serde_json::to_vec(&v["rows"]).unwrap().len();
        eprintln!("GET /api/downloads/rows, 50k: {rows_len} bytes in {:?}", t.elapsed());
        assert!(rows_len * 2 < full.len(), "rows {rows_len} vs items {}", full.len());
        let cursor = v["cursor"].as_str().unwrap().to_string();

        let t = Instant::now();
        let r = s.get(&format!("/api/downloads/rows?since={cursor}")).send().await.unwrap();
        assert_eq!(r.status(), 304);
        eprintln!("GET /api/downloads/rows?since, 50k, unchanged: 304 in {:?}", t.elapsed());

        // One change shows up as one row.
        let r = s.post("/api/downloads/7/schedule", serde_json::json!({})).send().await.unwrap();
        assert_eq!(r.status(), 400, "a finished download can't be scheduled: {}", r.text().await.unwrap());
        let r = reqwest::Client::new().delete(format!("{}/api/downloads/9", s.base)).header("X-Reidm-Token", &s.token).send().await.unwrap();
        assert_eq!(r.status(), 204);
        let v: serde_json::Value = s.get(&format!("/api/downloads/rows?since={cursor}")).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["full"], false);
        assert_eq!(v["rows"].as_array().unwrap().len(), 0);
        assert_eq!(v["removed"], serde_json::json!([9]));

        // The full item, for Properties.
        let one: serde_json::Value = s.get("/api/downloads/5").send().await.unwrap().json().await.unwrap();
        assert_eq!(one["segments"].as_array().unwrap().len(), 8);
        assert!(one["request"]["cookie"].is_null());
        assert_eq!(s.get("/api/downloads/9").send().await.unwrap().status(), 404);
    }

    #[tokio::test]
    async fn unknown_downloads_are_reported_not_silently_accepted() {
        let s = server("unknown", 3).await;
        let r = s.post("/api/downloads/999/pause", serde_json::json!({})).send().await.unwrap();
        assert_eq!(r.status(), 404);
        assert!(r.text().await.unwrap().contains("no download #999"));
        let r = reqwest::Client::new().delete(format!("{}/api/downloads/999", s.base)).header("X-Reidm-Token", &s.token).send().await.unwrap();
        assert_eq!(r.status(), 404);

        let v: serde_json::Value = s.post("/api/downloads/bulk", serde_json::json!({ "ids": [1, 2, 999], "action": "remove" })).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["ok"], 2);
        assert_eq!(v["failed"], 1);
        assert!(v["results"][2]["error"].as_str().unwrap().contains("999"));
        assert!(s.mgr.get(1).is_none() && s.mgr.get(3).is_some());
        let r = s.post("/api/downloads/bulk", serde_json::json!({ "ids": [3], "action": "open" })).send().await.unwrap();
        assert_eq!(r.status(), 400, "opening thousands of files at once is not a bulk action");
    }

    #[tokio::test]
    async fn the_token_is_checked_whatever_its_length() {
        let s = server("token", 0).await;
        let c = reqwest::Client::new();
        for t in ["", "x", &"0".repeat(64), &format!("{}0", s.token)] {
            let r = c.get(format!("{}/api/status", s.base)).header("X-Reidm-Token", t).send().await.unwrap();
            assert_eq!(r.status(), 401, "token {t:?}");
        }
        assert_eq!(s.get("/api/status").send().await.unwrap().status(), 200);
        assert!(same_secret(b"abc", b"abc"));
        assert!(!same_secret(b"abc", b"abd") && !same_secret(b"abc", b"abcd"));
    }

    #[tokio::test]
    async fn ext_endpoints_refuse_other_host_names() {
        let s = server("host", 0).await;
        let c = reqwest::Client::new();
        let ping = |host: String| c.get(format!("{}/ext/ping", s.base)).header(header::HOST, host).send();
        // DNS rebinding: the page's own name, pointed at 127.0.0.1.
        assert_eq!(ping(format!("evil.example:{}", s.port)).await.unwrap().status(), 403);
        assert_eq!(ping(format!("127.0.0.1:{}", s.port + 1)).await.unwrap().status(), 403);
        for ok in [format!("127.0.0.1:{}", s.port), format!("localhost:{}", s.port), format!("[::1]:{}", s.port)] {
            assert_eq!(ping(ok.clone()).await.unwrap().status(), 200, "{ok}");
        }
    }

    #[tokio::test]
    async fn a_web_page_cannot_fake_the_extension_checking_in() {
        let s = server("checkin", 0).await;
        let c = reqwest::Client::new();
        let seen = || async { s.get("/api/status").send().await.unwrap().json::<serde_json::Value>().await.unwrap()["extension"].clone() };
        // `<img src="http://127.0.0.1:61002/ext/ping?v=x">` on some web page.
        let r = c.get(format!("{}/ext/ping?v=9.9", s.base)).header("sec-fetch-site", "cross-site").header("sec-fetch-dest", "image").send().await.unwrap();
        assert_eq!(r.status(), 200, "the answer itself is harmless: a page can't read it");
        assert!(seen().await.is_null(), "a page's ping must not count as the extension");
        // The extension's own ping.
        c.get(format!("{}/ext/ping?v=0.8.0", s.base)).header("sec-fetch-site", "none").send().await.unwrap();
        assert_eq!(seen().await["version"], "0.8.0");
    }

    #[tokio::test]
    async fn oversized_requests_are_refused_with_an_explanation() {
        let s = server("limits", 0).await;
        let big = "x".repeat(SMALL_BODY + 1);
        let r = s.post("/api/settings", serde_json::json!({ "default_dir": big })).send().await.unwrap();
        assert_eq!(r.status(), 413);
        assert!(r.text().await.unwrap().contains("larger than reIDM accepts"));
        let r = reqwest::Client::new().post(format!("{}/ext/add", s.base)).json(&serde_json::json!({ "url": big })).send().await.unwrap();
        assert_eq!(r.status(), 413);
        assert_eq!(r.json::<serde_json::Value>().await.unwrap()["ok"], false, "the extension reads JSON");

        let urls: Vec<String> = (0..=MAX_BATCH).map(|i| format!("https://e.invalid/{i}")).collect();
        let r = s.post("/api/downloads/batch", serde_json::json!({ "urls": urls })).send().await.unwrap();
        assert_eq!(r.status(), 400);
        assert!(r.text().await.unwrap().contains("in parts"));

        // A torrent file well over the old 2 MB limit is read (and then refused as not a torrent).
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(vec![b'd'; 3 << 20]);
        let r = s.post("/api/torrent/inspect", serde_json::json!({ "data": data })).send().await.unwrap();
        assert_eq!(r.status(), 400, "read, not refused for its size");
    }

    #[tokio::test]
    async fn shutdown_stops_the_service_and_removes_the_token() {
        let s = server("shutdown", 0).await;
        assert!(s.dir.join("api-token").exists());
        assert_eq!(s.post("/api/shutdown", serde_json::json!({})).send().await.unwrap().status(), 202);
        for _ in 0..100 {
            if !s.dir.join("api-token").exists() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the token file outlived the service");
    }

    #[tokio::test]
    async fn storage_problems_reach_the_status() {
        let s = server("healthy", 1).await;
        let v: serde_json::Value = s.get("/api/status").send().await.unwrap().json().await.unwrap();
        assert_eq!(v["warnings"], serde_json::json!([]));
        let s = server_with("damaged", 0, Some(b"{ this is not json")).await;
        let v: serde_json::Value = s.get("/api/status").send().await.unwrap().json().await.unwrap();
        assert!(!v["warnings"].as_array().unwrap().is_empty(), "a damaged list is reported: {v}");
    }

    /// A file server for the extension's tests: `/a.bin` and `/b.bin` (different sizes), with
    /// ranges, answering the first byte after `delay`. Counts the requests it gets.
    fn files(delay: Duration) -> (u16, Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{BufRead, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { break };
                let h = h.clone();
                std::thread::spawn(move || {
                    let mut r = std::io::BufReader::new(s.try_clone().unwrap());
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut range = None;
                    loop {
                        let mut l = String::new();
                        if r.read_line(&mut l).unwrap_or(0) == 0 || l.trim().is_empty() {
                            break;
                        }
                        if let Some(v) = l.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            range = v.trim().split_once('-').and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok())));
                        }
                    }
                    h.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(delay);
                    let len = if line.contains("/b.bin") { 200_000 } else { 100_000 };
                    let (a, b) = range.map(|(a, b)| (a, b.unwrap_or(len - 1).min(len - 1))).unwrap_or((0, len - 1));
                    let head = format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{len}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b + 1 - a);
                    let _ = s.write_all(head.as_bytes());
                    let _ = s.write_all(&vec![7u8; b + 1 - a]);
                });
            }
        });
        (port, hits)
    }

    fn ext(s: &Server, body: serde_json::Value) -> reqwest::RequestBuilder {
        reqwest::Client::new().post(format!("{}/ext/add", s.base)).json(&body)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_proxy_password_is_kept_but_never_shown() {
        let s = server("proxy", 0).await;
        let proxy = serde_json::json!({ "mode": "manual", "kind": "socks5", "host": "proxy.lan", "port": 1080, "username": "me", "password": "secret" });
        assert_eq!(s.post("/api/settings", serde_json::json!({ "proxy": proxy, "performance": "light" })).send().await.unwrap().status(), 204);
        let v: serde_json::Value = s.get("/api/settings").send().await.unwrap().json().await.unwrap();
        assert_eq!(v["proxy"]["password_set"], true);
        assert!(v["proxy"].get("password").is_none(), "{v}");
        assert_eq!(v["proxy"]["host"], "proxy.lan");
        assert_eq!((v["performance"].as_str(), v["connections"].as_u64(), v["max_concurrent"].as_u64()), (Some("light"), Some(4), Some(2)));
        assert_eq!(v["profiles"]["maximum"]["connections"], 16);
        // The window sends back what it was shown: the password stays.
        let mut shown = v["proxy"].clone();
        shown.as_object_mut().unwrap().remove("password_set");
        assert_eq!(s.post("/api/settings", serde_json::json!({ "proxy": shown, "connections": 5 })).send().await.unwrap().status(), 204);
        assert_eq!(s.mgr.settings().proxy.password.as_deref(), Some("secret"));
        assert_eq!(s.mgr.settings().performance, reidm_core::Performance::Custom, "a number set by hand");
        // A proxy that can't work is refused with a reason.
        let r = s.post("/api/settings", serde_json::json!({ "proxy": { "mode": "manual", "host": "", "port": 1 } })).send().await.unwrap();
        assert_eq!(r.status(), 400);
        assert!(r.text().await.unwrap().contains("server name"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downloads_move_in_the_queue_one_or_many() {
        let s = server("move", 0).await;
        for n in 1..=4 {
            s.mgr.add_with(Request { url: format!("http://127.0.0.1:9/{n}.bin"), ..Default::default() }, AddOptions { later: true, ..Default::default() }).unwrap();
        }
        assert_eq!(s.post("/api/downloads/4/move", serde_json::json!({ "to": "top" })).send().await.unwrap().status(), 204);
        let v: serde_json::Value = s.post("/api/downloads/bulk", serde_json::json!({ "action": "move", "to": "bottom", "ids": [1, 2, 99] })).send().await.unwrap().json().await.unwrap();
        assert_eq!((v["ok"].as_u64(), v["failed"].as_u64()), (Some(2), Some(1)), "{v}");
        assert_eq!(s.mgr.queue_order(), [4, 3, 1, 2], "a block keeps its own order");
        let r = s.post("/api/downloads/1/move", serde_json::json!({ "to": "sideways" })).send().await.unwrap();
        assert_eq!(r.status(), 400);
        assert_eq!(s.post("/api/downloads/77/move", serde_json::json!({ "to": "up" })).send().await.unwrap().status(), 404);
        // Rows carry the order, so the window can sort by it.
        let rows: serde_json::Value = s.get("/api/downloads/rows").send().await.unwrap().json().await.unwrap();
        let order = |id: u64| rows["rows"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap()["order"].as_u64().unwrap();
        assert!(order(4) < order(3) && order(3) < order(1) && order(1) < order(2));
        assert!(rows["rows"][0]["checksum_ok"].is_null());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_finished_downloads_checksum_is_worked_out_on_demand() {
        let dir = std::env::temp_dir().join(format!("reidm-web-verify-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("abc.txt");
        std::fs::write(&f, b"abc").unwrap();
        let list = serde_json::json!({ "next_id": 2, "items": { "1": {
            "id": 1, "request": { "url": "https://example.com/abc.txt", "referer": null, "cookie": null, "user_agent": null, "filename": null, "dir": null },
            "path": f, "size": 3, "downloaded": 3, "status": { "state": "Done" }, "segments": [], "resumable": true } } });
        let s = server_with("verify", 0, Some(&serde_json::to_vec(&list).unwrap())).await;
        let v: serde_json::Value = s.post("/api/downloads/1/verify", serde_json::json!({ "algo": "sha1" })).send().await.unwrap().json().await.unwrap();
        assert_eq!(v, serde_json::json!({ "algo": "sha1", "value": "a9993e364706816aba3e25717850c26c9cd0d89d" }));
        let v: serde_json::Value = s.post("/api/downloads/1/verify", serde_json::json!({})).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["algo"], "sha256", "the default");
        assert_eq!(s.post("/api/downloads/1/verify", serde_json::json!({ "algo": "crc" })).send().await.unwrap().status(), 400);
        assert_eq!(s.post("/api/downloads/5/verify", serde_json::json!({})).send().await.unwrap().status(), 404);
        // Added with a checksum: kept, checked as given.
        let sum = serde_json::json!({ "algo": "md5", "value": "900150983CD24FB0D6963F7D28E17F72" });
        let r = s.post("/api/downloads", serde_json::json!({ "url": "http://127.0.0.1:9/x.bin", "later": true, "checksum": sum })).send().await.unwrap();
        assert_eq!(r.status(), 200);
        let id = r.json::<serde_json::Value>().await.unwrap()["id"].as_u64().unwrap();
        assert_eq!(s.mgr.get(id).unwrap().request.checksum.unwrap().value, "900150983cd24fb0d6963f7d28e17f72");
        let bad = s.post("/api/downloads", serde_json::json!({ "url": "http://127.0.0.1:9/y.bin", "checksum": { "algo": "md5", "value": "12" } })).send().await.unwrap();
        assert_eq!(bad.status(), 400);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refresh_aimed_at_one_download_refreshes_it_or_is_refused() {
        let s = server("refresh-id", 0).await;
        let (port, _) = files(Duration::ZERO);
        // Checked, so its size is known; not started.
        let id = s.mgr.add_checked(Request { url: format!("http://127.0.0.1:{port}/a.bin?old"), ..Default::default() }, AddOptions { later: true, ..Default::default() }).await.unwrap().id;
        let ping = || async { reqwest::get(format!("{}/ext/ping", s.base)).await.unwrap().json::<serde_json::Value>().await.unwrap() };
        assert!(ping().await["refresh_armed"].is_null());

        // Not armed: refused, and nothing is added.
        let r = ext(&s, serde_json::json!({ "url": format!("http://127.0.0.1:{port}/a.bin"), "refresh_id": id, "verify": true })).send().await.unwrap();
        assert_eq!(r.status(), 400);
        assert!(r.json::<serde_json::Value>().await.unwrap()["error"].as_str().unwrap().contains("no longer waiting"));
        assert_eq!(s.mgr.list().len(), 1, "never added as a new download");

        s.mgr.arm_refresh(id).unwrap();
        let armed = ping().await["refresh_armed"].clone();
        assert_eq!(armed["id"], id);
        assert_eq!(armed["name"], "a.bin");
        // Another file: refused, nothing added, and still waiting for the right one.
        let r = ext(&s, serde_json::json!({ "url": format!("http://127.0.0.1:{port}/b.bin"), "refresh_id": id, "verify": true })).send().await.unwrap();
        assert_eq!(r.status(), 400);
        assert!(r.json::<serde_json::Value>().await.unwrap()["error"].as_str().unwrap().contains("different file"));
        assert_eq!(s.mgr.list().len(), 1);
        assert_eq!(s.mgr.refresh_armed(), Some(id));
        // The same file: refreshed.
        let v: serde_json::Value = ext(&s, serde_json::json!({ "url": format!("http://127.0.0.1:{port}/a.bin?new"), "refresh_id": id })).send().await.unwrap().json().await.unwrap();
        assert_eq!((v["outcome"].as_str(), v["id"].as_u64()), (Some("refreshed"), Some(id)), "{v}");
        assert!(s.mgr.get(id).unwrap().request.url.ends_with("?new"));
        assert_eq!(s.mgr.refresh_armed(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_download_can_say_it_is_not_a_refresh() {
        let s = server("no-refresh", 0).await;
        let (port, _) = files(Duration::ZERO);
        // Never checked: its size is unknown, so any link would pass for it.
        let id = s.mgr.add_with(Request { url: "http://127.0.0.1:9/old.bin".into(), ..Default::default() }, AddOptions { later: true, ..Default::default() }).unwrap().id;
        s.mgr.arm_refresh(id).unwrap();
        let v: serde_json::Value = ext(&s, serde_json::json!({ "url": format!("http://127.0.0.1:{port}/a.bin"), "no_refresh": true, "later": true })).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["outcome"], "new", "{v}");
        assert_eq!(s.mgr.refresh_armed(), Some(id), "still waiting");
        assert!(s.mgr.get(id).unwrap().request.url.ends_with("/old.bin"));
        // Without it (an older extension), the link goes to the armed download as before.
        let v: serde_json::Value = ext(&s, serde_json::json!({ "url": format!("http://127.0.0.1:{port}/b.bin"), "later": true })).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["outcome"], "refreshed", "{v}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_repeated_request_id_gets_the_first_answer_even_while_it_is_still_being_checked() {
        let s = server("request-id", 0).await;
        let (port, hits) = files(Duration::from_millis(600));
        let body = serde_json::json!({ "url": format!("http://127.0.0.1:{port}/a.bin"), "verify": true, "later": true, "request_id": "dl-41" });
        // The extension gave up waiting on the first and sent it again while it was checked.
        let (a, b) = tokio::join!(ext(&s, body.clone()).send(), async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            ext(&s, body.clone()).send().await
        });
        let (a, b): (serde_json::Value, serde_json::Value) = (a.unwrap().json().await.unwrap(), b.unwrap().json().await.unwrap());
        assert_eq!(a, b);
        assert_eq!(a["outcome"], "new");
        assert_eq!(hits.load(Ordering::SeqCst), 1, "checked once, not twice");
        assert_eq!(s.mgr.list().len(), 1);
        // Later repeats too.
        let c: serde_json::Value = ext(&s, body.clone()).send().await.unwrap().json().await.unwrap();
        assert_eq!(c, a);
        // A refusal is an answer, repeated as such.
        let bad = serde_json::json!({ "url": "ftp://nowhere/x", "request_id": "dl-42" });
        let r1 = ext(&s, bad.clone()).send().await.unwrap();
        assert_eq!(r1.status(), 400);
        let r1 = r1.text().await.unwrap();
        let r2 = ext(&s, bad).send().await.unwrap();
        assert_eq!((r2.status().as_u16(), r2.text().await.unwrap()), (400, r1));
        // A new id is a new request; a silly one is refused.
        let other: serde_json::Value = ext(&s, serde_json::json!({ "url": format!("http://127.0.0.1:{port}/b.bin"), "later": true, "request_id": "dl-43" })).send().await.unwrap().json().await.unwrap();
        assert_eq!(other["outcome"], "new");
        assert_eq!(ext(&s, serde_json::json!({ "url": "http://x/y", "request_id": "x".repeat(65) })).send().await.unwrap().status(), 400);
        // The cache stays bounded.
        {
            let mut m = s.app.ext_answers.lock().unwrap();
            for i in 0..MAX_REQUEST_IDS {
                m.insert(format!("old-{i}"), (Instant::now(), Answer1::default()));
            }
        }
        ext(&s, serde_json::json!({ "url": "ftp://x/y", "request_id": "newest" })).send().await.unwrap();
        assert!(s.app.ext_answers.lock().unwrap().len() <= MAX_REQUEST_IDS);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn when_done_is_set_shown_and_cancelled() {
        let s = server("when-done", 0).await;
        let v: serde_json::Value = s.get("/api/when-done").send().await.unwrap().json().await.unwrap();
        assert_eq!(v, serde_json::json!({ "action": "none", "fires_at": null }));
        let v: serde_json::Value = s.post("/api/when-done", serde_json::json!({ "action": "hibernate" })).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["action"], "hibernate");
        let st: serde_json::Value = s.get("/api/status").send().await.unwrap().json().await.unwrap();
        assert_eq!(st["when_done"]["action"], "hibernate");
        assert_eq!(st["exiting"], false);
        let free = st["free_space"].as_array().unwrap();
        assert_eq!(free.len(), 2, "the download folder and the temporary folder: {st}");
        assert!(free.iter().all(|f| f["free"].as_u64().is_some_and(|n| n > 0)));
        assert_eq!(s.post("/api/when-done", serde_json::json!({ "action": "reboot" })).send().await.unwrap().status(), 400);
        let v: serde_json::Value = s.post("/api/when-done/cancel", serde_json::json!({})).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["action"], "none");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exit_when_done_tells_the_window_then_stops() {
        let s = server("exit", 0).await;
        s.app.begin_exit();
        let st: serde_json::Value = s.get("/api/status").send().await.unwrap().json().await.unwrap();
        assert_eq!(st["exiting"], true);
        for _ in 0..100 {
            if !s.dir.join("api-token").exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the service did not stop");
    }

    #[test]
    fn if_none_match_lists() {
        let mut h = HeaderMap::new();
        h.insert(header::IF_NONE_MATCH, "\"a\", W/\"b\"".parse().unwrap());
        assert!(etag_matches(&h, "\"a\"") && etag_matches(&h, "\"b\"") && !etag_matches(&h, "\"c\""));
    }
}
