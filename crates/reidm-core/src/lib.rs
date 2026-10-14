//! Segmented, resumable download engine used by the reIDM daemon and CLI.
//!
//! A download is fetched over several connections, each owning a byte range (a *segment*).
//! When a connection finishes its range early it takes over half of the largest range another
//! connection is still working on — IDM's dynamic segmentation — so one slow connection does
//! not decide how long the whole file takes.

pub mod grabber;
mod limiter;
pub mod lists;
pub mod schedule;
pub mod update;

pub use limiter::Limiter;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

const MIN_SPLIT: u64 = 1 << 20; // don't split files under 1 MiB
/// Dynamic segmentation never hands a connection less than this: below it, the cost of a new
/// request is more than the time the extra connection would save.
const MIN_PIECE: u64 = 256 << 10;
const MAX_RETRIES: u32 = 5;
/// How often a segment checks for cancellation while waiting on the network.
const IDLE_TICK: Duration = Duration::from_secs(1);
/// Give up on a server that stops sending entirely.
const IDLE_LIMIT: Duration = Duration::from_secs(60);
/// How long "use the next link the browser sends as this download's new address" stays armed.
const REFRESH_WINDOW: Duration = Duration::from_secs(180);
/// How long a browser hand-off may spend checking the address. The browser is holding its own
/// copy meanwhile, so an answer has to come back promptly either way.
const PROBE_LIMIT: Duration = Duration::from_secs(12);

/// What the user (or browser extension) asks us to fetch.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Request {
    pub url: String,
    pub referer: Option<String>,
    pub cookie: Option<String>,
    pub user_agent: Option<String>,
    /// Suggested file name (e.g. from the browser). Sanitised before use.
    pub filename: Option<String>,
    pub dir: Option<PathBuf>,
}

/// How to add a download, beyond what to fetch.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct AddOptions {
    /// Put it in the list without starting it — IDM's *Download Later*. It joins the scheduled
    /// queue, which the scheduler (or *Start queue*) starts.
    #[serde(default)]
    pub later: bool,
    /// For [`Manager::add_checked`]: refuse an address that answers with a web page. A link
    /// that looked like a file but leads to a login wall or a "your download will start" page
    /// is something for the browser to open, not for reIDM to save.
    #[serde(default)]
    pub refuse_html: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", content = "error")]
pub enum Status {
    Queued,
    Running,
    Paused,
    Done,
    Failed(String),
}

/// What `add` did with a URL that was already known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// A new entry was created.
    New,
    /// The URL was already listed; nothing new was created. If it had stopped, it is running again.
    Existing,
    /// The URL was already downloaded and the file is still on disk.
    AlreadyComplete,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AddOutcome {
    pub id: u64,
    pub outcome: Outcome,
}

/// The file behind the address is not the one this download started with. Resuming would splice
/// two different files together, so it is never retried; the download has to start again.
#[derive(Debug)]
pub struct ChangedOnServer(pub String);

impl std::fmt::Display for ChangedOnServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the file changed on the server since this download began ({}); restart the download to fetch the new version", self.0)
    }
}

impl std::error::Error for ChangedOnServer {}

/// A stable identity for a URL, used to recognise a link we have already been given.
///
/// The fragment is dropped (it is not sent to the server) and the query is sorted, so
/// `?b=2&a=1` and `?a=1&b=2` are the same download. Everything else is kept: two URLs that
/// differ in host, path or query values really are different files.
pub fn url_key(u: &url::Url) -> String {
    let mut q: Vec<(String, String)> = u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    q.sort();
    let query = if q.is_empty() {
        String::new()
    } else {
        let mut s = String::new();
        for (i, (k, v)) in q.iter().enumerate() {
            s.push(if i == 0 { '?' } else { '&' });
            s.push_str(k);
            s.push('=');
            s.push_str(v);
        }
        s
    };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    format!("{}{host}{port}{}{query}", u.scheme(), u.path())
}

fn url_key_of(raw: &str) -> Option<String> {
    url::Url::parse(raw).ok().map(|u| url_key(&u))
}

/// One byte range of a download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub start: u64,
    /// Inclusive; `u64::MAX` when the length is unknown.
    pub end: u64,
    pub done: u64,
    #[serde(default)]
    pub finished: bool,
    /// A connection is transferring this range right now. Only meaningful while the download
    /// runs; it is cleared whenever the list is loaded.
    #[serde(default)]
    pub active: bool,
    /// Bytes promised to a write that is in flight, just past `done`. A split must not hand
    /// these to another connection.
    #[serde(skip)]
    claimed: u64,
}

impl Segment {
    fn new(start: u64, end: u64) -> Self {
        Segment { start, end, done: 0, finished: false, active: false, claimed: 0 }
    }

    pub fn complete(&self) -> bool {
        self.finished || (self.end != u64::MAX && self.done > self.end - self.start)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub request: Request,
    pub path: PathBuf,
    pub size: Option<u64>,
    pub downloaded: u64,
    pub status: Status,
    segments: Vec<Segment>,
    resumable: bool,
    /// When the download was queued, as unix seconds.
    #[serde(default)]
    pub added: u64,
    /// When a transfer last ran, as unix seconds. Shown as IDM's "Last Try" column.
    #[serde(default)]
    pub last_try: Option<u64>,
    /// When the download completed, as unix seconds.
    #[serde(default)]
    pub finished_at: Option<u64>,
    /// The server's strong `ETag` from the first request. Sent as `If-Range` when resuming, so a
    /// file that changed in the meantime is noticed instead of being stitched onto the old bytes.
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    /// This download's own speed cap in bytes per second, on top of the global one.
    #[serde(default)]
    pub speed_limit: Option<u64>,
    /// Part of the scheduled queue (*Download Later*).
    #[serde(default)]
    pub scheduled: bool,
}

impl Item {
    /// The byte ranges and how far each has got, for a segment map.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Whether the server serves byte ranges, i.e. whether this download can be resumed.
    pub fn resumable(&self) -> bool {
        self.resumable
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The shared state of one running download: which ranges exist, and how many connections are
/// still working on them.
#[derive(Default)]
struct Plan {
    segs: Vec<Segment>,
    /// Connections still running. The last one standing reports a failure instead of handing
    /// its range back, because there is nobody left to hand it to.
    workers: usize,
}

/// The handles a running download shares between its connections and the rest of the manager.
#[derive(Clone)]
struct Live {
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    plan: Arc<Mutex<Plan>>,
    /// This download's own limit; the global one lives on the manager.
    limiter: Arc<Limiter>,
}

/// What every connection of one run needs, built once per run.
struct RunCtx {
    id: u64,
    url: String,
    headers: HeaderMap,
    part: PathBuf,
    resumable: bool,
    size: Option<u64>,
    /// `If-Range` value: the strong `ETag` from the first response, when there was one.
    validator: Option<String>,
    /// Ranges can only be split when the server serves ranges of a file of known length.
    splittable: bool,
    live: Live,
}

#[derive(Serialize, Deserialize, Default)]
struct Db {
    next_id: u64,
    items: BTreeMap<u64, Item>,
}

fn default_max_concurrent() -> usize {
    4
}

/// User-adjustable options, persisted next to the download list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub default_dir: PathBuf,
    pub connections: usize,
    /// How many downloads may transfer at once. The rest wait as `Queued`, which is what
    /// makes adding a hundred links at once reasonable.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    /// The global speed cap in bytes per second, applied while `limit_enabled` is on. Kept when
    /// the limiter is switched off, so switching it back on restores the same cap.
    #[serde(default)]
    pub speed_limit: u64,
    #[serde(default)]
    pub limit_enabled: bool,
    #[serde(default)]
    pub schedule: schedule::Schedule,
}

impl Settings {
    /// The cap actually in force, in bytes per second; 0 means none.
    pub fn effective_limit(&self) -> u64 {
        if self.limit_enabled {
            self.speed_limit
        } else {
            0
        }
    }
}

/// A partial update to [`Settings`]: `None` leaves a field as it is.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SettingsPatch {
    #[serde(default)]
    pub default_dir: Option<PathBuf>,
    #[serde(default)]
    pub connections: Option<usize>,
    #[serde(default)]
    pub max_concurrent: Option<usize>,
    #[serde(default)]
    pub speed_limit: Option<u64>,
    #[serde(default)]
    pub limit_enabled: Option<bool>,
    #[serde(default)]
    pub schedule: Option<schedule::Schedule>,
}

struct Inner {
    db: Mutex<Db>,
    live: Mutex<BTreeMap<u64, Live>>,
    db_path: PathBuf,
    settings_path: PathBuf,
    settings: Mutex<Settings>,
    http: reqwest::Client,
    /// The global speed limit, shared by every connection of every download.
    limiter: Arc<Limiter>,
    /// A download waiting for the browser to supply a fresh address, and until when.
    refresh: Mutex<Option<(u64, Instant)>>,
}

#[derive(Clone)]
pub struct Manager(Arc<Inner>);

pub fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "Zektopic", "reIDM")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".reidm"))
}

pub fn default_download_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Windows resolves these to devices whatever the extension, so a download named `NUL` would
/// silently write nowhere and still report success.
fn is_reserved_device(stem: &str) -> bool {
    let up = stem.to_ascii_uppercase();
    if matches!(up.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let b = up.as_bytes();
    b.len() == 4 && (up.starts_with("COM") || up.starts_with("LPT")) && b[3].is_ascii_digit()
}

/// Strip path separators and control characters so a hostile server/page cannot escape `dir`.
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        return "download".into();
    }
    match cleaned.split('.').next() {
        Some(stem) if is_reserved_device(stem) => format!("_{cleaned}"),
        _ => cleaned,
    }
}

/// Split a `Content-Disposition` value on `;`, ignoring separators inside a quoted string, so a
/// name like `"a;b.zip"` keeps its extension.
fn split_params(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in v.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                cur.push(ch);
            }
            ';' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    out.push(cur);
    out
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    match s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        Some(inner) => inner.replace("\\\"", "\""),
        None => s.to_string(),
    }
}

fn filename_from_disposition(v: &str) -> Option<String> {
    // RFC 6266: `filename*` (a percent-encoded, optionally language-tagged ext-value) takes
    // precedence over the plain `filename` parameter.
    let mut plain = None;
    for part in split_params(v) {
        let p = part.trim();
        if let Some(rest) = p.strip_prefix("filename*=") {
            let rest = unquote(rest);
            // charset ' language ' percent-encoded-value
            let mut it = rest.splitn(3, '\'');
            if let (Some(_charset), Some(_lang), Some(enc)) = (it.next(), it.next(), it.next()) {
                return Some(percent_encoding::percent_decode_str(enc).decode_utf8_lossy().into_owned());
            }
        } else if let Some(rest) = p.strip_prefix("filename=") {
            plain = Some(unquote(rest));
        }
    }
    plain
}

fn filename_from_url(u: &url::Url) -> String {
    let seg = u.path_segments().and_then(|mut s| s.next_back()).unwrap_or("");
    let decoded = percent_encoding::percent_decode_str(seg).decode_utf8_lossy().into_owned();
    if decoded.is_empty() { "download".into() } else { decoded }
}

/// `bytes 100-199/1000` → `(100, Some(1000))`; `bytes 100-199/*` → `(100, None)`.
fn parse_content_range(v: &str) -> Option<(u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes")?.trim();
    let (range, total) = rest.split_once('/')?;
    let (a, _) = range.split_once('-')?;
    Some((a.trim().parse().ok()?, total.trim().parse().ok()))
}

/// Pick a path that is not already on disk *and* not already claimed by another download.
///
/// The second half matters: the `.reidm` part file does not appear until the probe finishes, so
/// two downloads started a second apart would otherwise both pick `foo.zip` and then write into
/// the same part file, corrupting each other.
fn unique_path(dir: &Path, name: &str, claimed: impl Fn(&Path) -> bool) -> PathBuf {
    let free = |p: &Path| !p.exists() && !part_path(p).exists() && !claimed(p);
    let mut p = dir.join(name);
    if free(&p) {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for n in 1.. {
        p = dir.join(format!("{stem} ({n}){ext}"));
        if free(&p) {
            break;
        }
    }
    p
}

fn part_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".reidm");
    PathBuf::from(s)
}

/// The ranges a fresh download starts with: one per connection when the server serves ranges
/// of a file large enough to be worth splitting, otherwise a single one.
fn initial_segments(size: Option<u64>, resumable: bool, connections: u64) -> Vec<Segment> {
    match (size, resumable) {
        // Nothing to fetch: the file is created empty and the download completes.
        (Some(0), _) => vec![],
        (Some(sz), true) if sz >= MIN_SPLIT => {
            let n = connections.max(1);
            let chunk = sz.div_ceil(n);
            (0..n).map(|i| Segment::new(i * chunk, ((i + 1) * chunk - 1).min(sz - 1))).filter(|s| s.start < sz).collect()
        }
        (Some(sz), _) => vec![Segment::new(0, sz.saturating_sub(1))],
        (None, _) => vec![Segment::new(0, u64::MAX)],
    }
}

/// Give an idle connection something to do: first a range nobody is working on, then — for a
/// splittable download — the far half of the largest range another connection still has left.
fn claim(segs: &mut Vec<Segment>, splittable: bool) -> Option<usize> {
    if let Some(i) = segs.iter().position(|s| !s.active && !s.complete()) {
        segs[i].active = true;
        return Some(i);
    }
    if !splittable {
        return None;
    }
    let (i, pos, rem) = segs
        .iter()
        .enumerate()
        .filter(|(_, s)| s.active && s.end != u64::MAX && !s.complete())
        .filter_map(|(i, s)| {
            // Everything up to `pos` is written or being written by its owner.
            let pos = s.start + s.done + s.claimed;
            (s.end + 1).checked_sub(pos).map(|rem| (i, pos, rem))
        })
        .max_by_key(|&(_, _, rem)| rem)?;
    if rem < 2 * MIN_PIECE {
        return None;
    }
    let mid = pos + rem / 2;
    let mut piece = Segment::new(mid, segs[i].end);
    piece.active = true;
    segs[i].end = mid - 1;
    segs.push(piece);
    Some(segs.len() - 1)
}

/// Errors that no amount of retrying, and no other connection, can fix.
fn is_fatal(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ChangedOnServer>().is_some() || e.downcast_ref::<std::io::Error>().is_some()
}

/// `429 Too Many Requests` / `503 Service Unavailable`: how servers that cap connections per
/// client usually refuse an extra one.
fn is_busy(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<reqwest::Error>().and_then(reqwest::Error::status).map(|s| s.as_u16()),
        Some(429) | Some(503)
    )
}

/// Sleep, but wake early when `cancel` is set.
async fn nap(d: Duration, cancel: &AtomicBool) {
    let until = Instant::now() + d;
    while !cancel.load(Ordering::SeqCst) {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        tokio::time::sleep(left.min(Duration::from_millis(250))).await;
    }
}

/// What a first request tells us about a URL.
struct Probe {
    size: Option<u64>,
    resumable: bool,
    filename: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
    /// The media type without parameters, lower-cased.
    content_type: Option<String>,
}

impl Manager {
    /// Open with the default concurrency limit; see [`Manager::open_with`].
    pub fn open(data_dir: &Path, default_dir: PathBuf, connections: usize) -> Result<Self> {
        Self::open_with(data_dir, default_dir, connections, default_max_concurrent())
    }

    /// `max_concurrent` is only the default for a first run; a saved `settings.json` wins.
    pub fn open_with(data_dir: &Path, default_dir: PathBuf, connections: usize, max_concurrent: usize) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("downloads.json");
        let mut db: Db = match std::fs::read(&db_path) {
            Ok(b) => serde_json::from_slice(&b).context("corrupt downloads.json")?,
            Err(_) => Db { next_id: 1, ..Default::default() },
        };
        if db.next_id == 0 {
            db.next_id = 1;
        }
        for it in db.items.values_mut() {
            // anything that was running when we last exited is now paused
            if it.status == Status::Running || it.status == Status::Queued {
                it.status = Status::Paused;
            }
            // and nothing is transferring
            for s in &mut it.segments {
                s.active = false;
            }
        }
        // Saved options win over the values passed on the command line.
        let settings_path = data_dir.join("settings.json");
        let saved: Option<Settings> = std::fs::read(&settings_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .filter(|s| s.connections > 0);
        let mut settings = saved.unwrap_or(Settings {
            default_dir: default_dir.clone(),
            connections,
            max_concurrent,
            speed_limit: 0,
            limit_enabled: false,
            schedule: Default::default(),
        });
        settings.connections = settings.connections.clamp(1, 32);
        settings.max_concurrent = settings.max_concurrent.clamp(1, 64);

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(10))
            // The bytes on disk must be the bytes the server stores. With transparent
            // decompression a misconfigured server's `.gz` would be saved decompressed, and a
            // compressed response to a range request cannot be decoded at all.
            .no_gzip()
            .no_brotli()
            .build()?;
        let limiter = Arc::new(Limiter::new(settings.effective_limit()));
        Ok(Self(Arc::new(Inner {
            db: Mutex::new(db),
            live: Mutex::new(BTreeMap::new()),
            db_path,
            settings_path,
            settings: Mutex::new(settings),
            http,
            limiter,
            refresh: Mutex::new(None),
        })))
    }

    pub fn settings(&self) -> Settings {
        self.0.settings.lock().unwrap().clone()
    }

    /// Update the download folder and/or connection count. Persisted immediately.
    pub fn set_settings(&self, dir: Option<PathBuf>, connections: Option<usize>, max_concurrent: Option<usize>) -> Result<()> {
        self.update_settings(SettingsPatch { default_dir: dir, connections, max_concurrent, ..Default::default() })
    }

    /// Apply a partial settings update. Persisted immediately; a new speed limit applies to
    /// transfers already running.
    pub fn update_settings(&self, patch: SettingsPatch) -> Result<()> {
        if let Some(s) = &patch.schedule {
            s.validate()?;
        }
        if let Some(d) = &patch.default_dir {
            std::fs::create_dir_all(d).with_context(|| format!("cannot use {} as the download folder", d.display()))?;
        }
        let limit = {
            let mut s = self.0.settings.lock().unwrap();
            if let Some(d) = patch.default_dir {
                s.default_dir = d;
            }
            if let Some(c) = patch.connections {
                s.connections = c.clamp(1, 32);
            }
            if let Some(m) = patch.max_concurrent {
                s.max_concurrent = m.clamp(1, 64);
            }
            if let Some(v) = patch.speed_limit {
                s.speed_limit = v;
            }
            if let Some(v) = patch.limit_enabled {
                s.limit_enabled = v;
            }
            if let Some(v) = patch.schedule {
                s.schedule = v;
            }
            s.effective_limit()
        };
        self.0.limiter.set_rate(limit);
        self.save_settings();
        // Raising the limit should let waiting downloads start now, not at the next completion.
        self.pump();
        Ok(())
    }

    fn save_settings(&self) {
        let s = self.0.settings.lock().unwrap();
        if let Ok(b) = serde_json::to_vec_pretty(&*s) {
            let tmp = self.0.settings_path.with_extension("json.tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, &self.0.settings_path);
            }
        }
    }

    fn save(&self) {
        let db = self.0.db.lock().unwrap();
        if let Ok(b) = serde_json::to_vec_pretty(&*db) {
            let tmp = self.0.db_path.with_extension("json.tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, &self.0.db_path);
            }
        }
    }

    /// Directory holding `downloads.json` (and the daemon's API token, if it is serving a UI).
    pub fn data_dir(&self) -> &Path {
        self.0.db_path.parent().unwrap_or(Path::new("."))
    }

    /// Every lock in this type must be taken `db` before `live`, and both before a download's
    /// own plan. Taking them the other way round would deadlock against `resume`/`start`, which
    /// hold `db` while taking `live`.
    pub fn list(&self) -> Vec<Item> {
        let mut items: Vec<Item> = self.0.db.lock().unwrap().items.values().cloned().collect();
        let live = self.0.live.lock().unwrap();
        for it in &mut items {
            if let Some(l) = live.get(&it.id) {
                it.downloaded = l.progress.load(Ordering::Relaxed);
                // The live ranges, not the last checkpoint, so a segment map moves smoothly.
                let p = l.plan.lock().unwrap();
                if !p.segs.is_empty() {
                    it.segments = p.segs.clone();
                }
            }
        }
        items
    }

    pub fn get(&self, id: u64) -> Option<Item> {
        self.list().into_iter().find(|i| i.id == id)
    }

    /// Queue a download. Returns its id.
    ///
    /// Adding the same URL twice does not make a second copy: see [`Manager::add_detailed`].
    pub fn add(&self, req: Request) -> Result<u64> {
        Ok(self.add_detailed(req)?.id)
    }

    /// Queue a download and say what actually happened to it.
    ///
    /// Re-adding a URL resumes the existing entry rather than starting a duplicate, which is what
    /// makes "paste the link again" a sensible way to carry on an interrupted download.
    pub fn add_detailed(&self, req: Request) -> Result<AddOutcome> {
        self.add_with(req, AddOptions::default())
    }

    /// [`Manager::add_detailed`], with control over whether the download starts now.
    pub fn add_with(&self, req: Request, opts: AddOptions) -> Result<AddOutcome> {
        self.add_inner(req, opts, None)
    }

    /// Add a download only once the address has been shown to work with the cookies, referer and
    /// user agent supplied — the check a browser hand-off needs. The browser is holding its own
    /// copy of the download while this runs; if reIDM cannot fetch the file (a one-time link the
    /// browser has already used, a header reIDM was not given), the error goes back and the
    /// browser keeps it, instead of the download being lost.
    ///
    /// What the check learns (size, name, range support) is kept, so the transfer does not ask
    /// the server a second time.
    pub async fn add_checked(&self, req: Request, opts: AddOptions) -> Result<AddOutcome> {
        let u = url::Url::parse(&req.url).context("invalid URL")?;
        if !matches!(u.scheme(), "http" | "https") {
            bail!("unsupported scheme {:?} (only http/https)", u.scheme());
        }
        let key = url_key(&u);
        let known = self.0.db.lock().unwrap().items.values().any(|i| url_key_of(&i.request.url).as_deref() == Some(key.as_str()));
        if known {
            // Already in the list: re-adding resumes it, which needs no new check.
            return self.add_with(req, opts);
        }
        let p = match tokio::time::timeout(PROBE_LIMIT, self.probe(&req.url, &Self::headers(&req))).await {
            Ok(r) => r?,
            Err(_) => bail!("the server did not answer within {}s", PROBE_LIMIT.as_secs()),
        };
        if opts.refuse_html && p.content_type.as_deref().is_some_and(|t| t.eq_ignore_ascii_case("text/html") || t.eq_ignore_ascii_case("application/xhtml+xml")) {
            let named_html = url_key(&u).rsplit('/').next().is_some_and(|n| {
                let n = n.split('?').next().unwrap_or(n).to_ascii_lowercase();
                n.ends_with(".html") || n.ends_with(".htm")
            });
            if !named_html {
                bail!("the link answers with a web page, not a file");
            }
        }
        self.add_inner(req, opts, Some(p))
    }

    fn add_inner(&self, req: Request, opts: AddOptions, probe: Option<Probe>) -> Result<AddOutcome> {
        let u = url::Url::parse(&req.url).context("invalid URL")?;
        if !matches!(u.scheme(), "http" | "https") {
            bail!("unsupported scheme {:?} (only http/https)", u.scheme());
        }
        let key = url_key(&u);
        let dir = req.dir.clone().unwrap_or_else(|| self.0.settings.lock().unwrap().default_dir.clone());
        std::fs::create_dir_all(&dir)?;

        let id = {
            let mut db = self.0.db.lock().unwrap();

            // Already in the list?
            if let Some(existing) = db.items.values().find(|i| url_key_of(&i.request.url).as_deref() == Some(key.as_str())) {
                let (id, status) = (existing.id, existing.status.clone());
                // `req.filename` may carry a better name than the one we guessed last time.
                if let Some(name) = req.filename.as_deref().map(sanitize_filename) {
                    if let Some(slot) = db.items.get_mut(&id) {
                        if slot.request.filename.is_none() {
                            slot.request.filename = Some(name);
                        }
                    }
                }
                let live = self.0.live.lock().unwrap().contains_key(&id);
                if status == Status::Done && !live {
                    return Ok(AddOutcome { id, outcome: Outcome::AlreadyComplete });
                }
                // Anything that had stopped is picked up again: re-adding the link is how you
                // carry on an interrupted download. *Download Later* leaves it where it is.
                if !opts.later && !live && matches!(status, Status::Paused | Status::Failed(_)) {
                    if let Some(slot) = db.items.get_mut(&id) {
                        slot.status = Status::Queued;
                    }
                    drop(db);
                    self.save();
                    self.pump();
                }
                return Ok(AddOutcome { id, outcome: Outcome::Existing });
            }

            let id = db.next_id;
            db.next_id += 1;
            // The caller's name first (the browser's choice), then the server's, then the URL's.
            let hint = req
                .filename
                .as_deref()
                .or(probe.as_ref().and_then(|p| p.filename.as_deref()))
                .map(sanitize_filename)
                .unwrap_or_else(|| sanitize_filename(&filename_from_url(&u)));
            let claimed: HashSet<PathBuf> = db.items.values().map(|i| i.path.clone()).collect();
            let path = unique_path(&dir, &hint, |p| claimed.contains(p));
            let mut item = Item {
                id,
                request: req,
                path,
                size: None,
                downloaded: 0,
                status: if opts.later { Status::Paused } else { Status::Queued },
                segments: vec![],
                resumable: false,
                added: now_secs(),
                last_try: None,
                finished_at: None,
                etag: None,
                last_modified: None,
                speed_limit: None,
                scheduled: opts.later,
            };
            if let Some(p) = probe {
                let n = self.0.settings.lock().unwrap().connections as u64;
                item.segments = initial_segments(p.size, p.resumable, n);
                item.size = p.size;
                item.resumable = p.resumable;
                item.etag = p.etag;
                item.last_modified = p.last_modified;
            }
            db.items.insert(id, item);
            id
        };
        self.save();
        self.pump();
        Ok(AddOutcome { id, outcome: Outcome::New })
    }

    /// Queue many requests, skipping duplicates, and report per-URL what happened.
    pub fn add_many(&self, reqs: Vec<Request>) -> Vec<(String, Result<AddOutcome>)> {
        self.add_many_with(reqs, AddOptions::default())
    }

    pub fn add_many_with(&self, reqs: Vec<Request>, opts: AddOptions) -> Vec<(String, Result<AddOutcome>)> {
        reqs.into_iter()
            .map(|r| {
                let url = r.url.clone();
                (url, self.add_with(r, opts))
            })
            .collect()
    }

    /// Start as many queued downloads as the concurrency limit allows.
    ///
    /// Called after anything that changes what is running: a new download, a pause, a removal, or
    /// a transfer finishing — often from several threads at once. A download is therefore marked
    /// live under the same locks that chose it: choosing under the lock and marking it live
    /// afterwards let two concurrent calls both pick the same download and run it twice, two
    /// transfers writing one part file until one renamed it away from under the other.
    pub fn pump(&self) {
        let limit = self.0.settings.lock().unwrap().max_concurrent.max(1);
        let mut to_start = Vec::new();
        {
            // db before live, like every other lock site in this type.
            let db = self.0.db.lock().unwrap();
            let mut live = self.0.live.lock().unwrap();
            let free = limit.saturating_sub(live.len());
            for it in db.items.values() {
                if to_start.len() >= free {
                    break;
                }
                // A queued item that is not live is exactly one that is waiting for a slot.
                if it.status == Status::Queued && !live.contains_key(&it.id) {
                    let l = Live {
                        cancel: Arc::new(AtomicBool::new(false)),
                        progress: Arc::new(AtomicU64::new(it.downloaded)),
                        plan: Arc::new(Mutex::new(Plan::default())),
                        limiter: Arc::new(Limiter::new(it.speed_limit.unwrap_or(0))),
                    };
                    live.insert(it.id, l.clone());
                    to_start.push((it.id, l));
                }
            }
        }
        for (id, live) in to_start {
            self.launch(id, live);
        }
    }

    pub fn resume(&self, id: u64) -> Result<()> {
        {
            let mut db = self.0.db.lock().unwrap();
            let it = db.items.get_mut(&id).ok_or_else(|| anyhow!("no such download {id}"))?;
            if it.status == Status::Done {
                bail!("already complete");
            }
            if self.0.live.lock().unwrap().contains_key(&id) {
                return Ok(());
            }
            it.status = Status::Queued;
        }
        // Through the scheduler, so resuming twenty paused rows does not start twenty transfers.
        self.pump();
        Ok(())
    }

    /// Re-queue a finished download from its original URL, replacing what is on disk.
    pub fn redownload(&self, id: u64) -> Result<()> {
        {
            let mut db = self.0.db.lock().unwrap();
            let it = db.items.get_mut(&id).ok_or_else(|| anyhow!("no such download {id}"))?;
            it.segments.clear();
            it.downloaded = 0;
            it.size = None;
            it.resumable = false;
            it.etag = None;
            it.last_modified = None;
            it.finished_at = None;
            it.status = Status::Queued;
            let _ = std::fs::remove_file(part_path(&it.path));
        }
        self.save();
        self.pump();
        Ok(())
    }

    /// Stop a download if it is running, and start it again from the first byte.
    pub async fn restart(&self, id: u64) -> Result<()> {
        if !self.0.db.lock().unwrap().items.contains_key(&id) {
            bail!("no such download {id}");
        }
        self.stop_and_wait(id).await;
        self.redownload(id)
    }

    pub fn pause(&self, id: u64) {
        if let Some(l) = self.0.live.lock().unwrap().get(&id) {
            l.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Pause, and make sure nothing will start it again behind our back, before changing what a
    /// download *is*. Returns once its connections have stopped.
    async fn stop_and_wait(&self, id: u64) {
        {
            let mut db = self.0.db.lock().unwrap();
            let live = self.0.live.lock().unwrap();
            if let Some(it) = db.items.get_mut(&id) {
                if it.status == Status::Queued && !live.contains_key(&id) {
                    it.status = Status::Paused;
                }
            }
        }
        self.pause(id);
        for _ in 0..150 {
            if !self.0.live.lock().unwrap().contains_key(&id) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub fn remove(&self, id: u64, delete_files: bool) {
        self.pause(id);
        let removed = self.0.db.lock().unwrap().items.remove(&id);
        if let Some(it) = removed {
            // The part file is reIDM's own scratch space, so it always goes; leaving it behind
            // would make every later download of the same name pick "name (1).zip".
            let _ = std::fs::remove_file(part_path(&it.path));
            if delete_files {
                // "remove and delete the file" must also work for finished downloads, which are
                // the ones people actually delete.
                let _ = std::fs::remove_file(&it.path);
            }
        }
        self.save();
        self.pump();
    }

    /// Cap one download's speed (bytes per second), or lift its own cap with `None`. Takes
    /// effect immediately if it is running; the global limit still applies on top.
    pub fn set_speed_limit(&self, id: u64, limit: Option<u64>) -> Result<()> {
        let limit = limit.filter(|l| *l > 0);
        {
            let mut db = self.0.db.lock().unwrap();
            let it = db.items.get_mut(&id).ok_or_else(|| anyhow!("no such download {id}"))?;
            it.speed_limit = limit;
            if let Some(l) = self.0.live.lock().unwrap().get(&id) {
                l.limiter.set_rate(limit.unwrap_or(0));
            }
        }
        self.save();
        Ok(())
    }

    /// Move a download into or out of the scheduled queue.
    pub fn set_scheduled(&self, id: u64, scheduled: bool) -> Result<()> {
        {
            let mut db = self.0.db.lock().unwrap();
            let it = db.items.get_mut(&id).ok_or_else(|| anyhow!("no such download {id}"))?;
            if scheduled && it.status == Status::Done {
                bail!("that download is already complete");
            }
            it.scheduled = scheduled;
        }
        self.save();
        Ok(())
    }

    /// Start every stopped download in the scheduled queue (*Start queue*). They still go
    /// through the concurrency limit. Returns how many were started.
    pub fn start_queue(&self) -> usize {
        let n = {
            let mut db = self.0.db.lock().unwrap();
            let live = self.0.live.lock().unwrap();
            let mut n = 0;
            for it in db.items.values_mut() {
                if it.scheduled && !live.contains_key(&it.id) && matches!(it.status, Status::Paused | Status::Failed(_)) {
                    it.status = Status::Queued;
                    n += 1;
                }
            }
            n
        };
        self.save();
        self.pump();
        n
    }

    /// Stop every download in the scheduled queue (*Stop queue*). Returns how many were stopped.
    pub fn stop_queue(&self) -> usize {
        let running: Vec<u64> = {
            let mut db = self.0.db.lock().unwrap();
            let live = self.0.live.lock().unwrap();
            let mut running = Vec::new();
            for it in db.items.values_mut().filter(|i| i.scheduled) {
                if live.contains_key(&it.id) {
                    running.push(it.id);
                } else if it.status == Status::Queued {
                    it.status = Status::Paused;
                }
            }
            running
        };
        for id in &running {
            self.pause(*id);
        }
        self.save();
        running.len()
    }

    /// Run the scheduler for as long as the process lives. Call once, from the daemon.
    pub fn spawn_scheduler(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut prev = chrono::Local::now().naive_local();
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                let now = chrono::Local::now().naive_local();
                let sched = me.settings().schedule;
                match sched.due(prev, now) {
                    Some(schedule::Action::Start) => {
                        let n = me.start_queue();
                        tracing::info!("scheduler: started the queue ({n} download(s))");
                    }
                    Some(schedule::Action::Stop) => {
                        let n = me.stop_queue();
                        tracing::info!("scheduler: stopped the queue ({n} running)");
                    }
                    None => {}
                }
                prev = now;
            }
        });
    }

    // ── refreshing an address ────────────────────────────────────────────

    /// Use the next download the browser hands over as `id`'s new address, for the next
    /// three minutes. This is IDM's *Refresh download address*: open the page again, click the
    /// link again, and the expired download carries on from where it was.
    pub fn arm_refresh(&self, id: u64) -> Result<()> {
        let db = self.0.db.lock().unwrap();
        let it = db.items.get(&id).ok_or_else(|| anyhow!("no such download {id}"))?;
        if it.status == Status::Done {
            bail!("that download is already complete");
        }
        *self.0.refresh.lock().unwrap() = Some((id, Instant::now() + REFRESH_WINDOW));
        Ok(())
    }

    pub fn disarm_refresh(&self) {
        *self.0.refresh.lock().unwrap() = None;
    }

    /// The download waiting for a new address, if the window is still open.
    pub fn refresh_armed(&self) -> Option<u64> {
        match *self.0.refresh.lock().unwrap() {
            Some((id, until)) if Instant::now() < until => Some(id),
            _ => None,
        }
    }

    /// Consume the armed refresh, if any. Called by whatever receives the browser's next link.
    pub fn take_refresh_target(&self) -> Option<u64> {
        let mut g = self.0.refresh.lock().unwrap();
        let target = match *g {
            Some((id, until)) if Instant::now() < until => Some(id),
            _ => None,
        };
        *g = None;
        target
    }

    /// Point an unfinished download at a new address — the old one expired, or a mirror is
    /// faster — and carry on from where it was.
    ///
    /// The new address is checked first: it must serve a file of the same size, and it must
    /// serve ranges if bytes have already been downloaded. Anything else would quietly produce a
    /// file made of two different downloads.
    pub async fn refresh_address(&self, id: u64, new: Request) -> Result<()> {
        let u = url::Url::parse(new.url.trim()).context("invalid URL")?;
        if !matches!(u.scheme(), "http" | "https") {
            bail!("unsupported scheme {:?} (only http/https)", u.scheme());
        }
        let key = url_key(&u);
        {
            let db = self.0.db.lock().unwrap();
            let it = db.items.get(&id).ok_or_else(|| anyhow!("no such download {id}"))?;
            if it.status == Status::Done {
                bail!("that download is already complete");
            }
            if let Some(other) = db.items.values().find(|i| i.id != id && url_key_of(&i.request.url).as_deref() == Some(key.as_str())) {
                bail!("that address is already in the list as download #{}", other.id);
            }
        }

        // Its connections are still using the old address.
        self.stop_and_wait(id).await;
        let item = self.get(id).ok_or_else(|| anyhow!("no such download {id}"))?;
        let mut req = item.request.clone();
        req.url = u.to_string();
        if new.referer.is_some() {
            req.referer = new.referer;
        }
        if new.cookie.is_some() {
            req.cookie = new.cookie;
        }
        if new.user_agent.is_some() {
            req.user_agent = new.user_agent;
        }
        let p = self.probe(&req.url, &Self::headers(&req)).await.context("the new address could not be reached")?;

        let progressed = item.segments.iter().any(|s| s.done > 0 || s.finished);
        match (item.size, p.size) {
            (Some(was), Some(now)) if was != now => {
                bail!("the new address is a different file: it is {now} bytes, the download is {was} bytes")
            }
            (Some(_), None) if progressed => bail!("the new address does not say how large its file is, so it cannot be checked against this download"),
            _ => {}
        }
        if progressed && item.resumable && !p.resumable {
            bail!("the new address cannot continue a partial download (it does not serve byte ranges); restart the download instead");
        }

        {
            let mut db = self.0.db.lock().unwrap();
            let it = db.items.get_mut(&id).ok_or_else(|| anyhow!("download {id} was removed"))?;
            it.request = req;
            // The validators of the new server, not the old one's: a mirror has its own ETags.
            it.etag = p.etag;
            it.last_modified = p.last_modified;
            if it.segments.is_empty() {
                // Never probed: let the next run do it from scratch with the new address.
                it.size = None;
            }
            it.status = Status::Queued;
        }
        self.save();
        self.pump();
        Ok(())
    }

    /// Run a download whose slot [`Manager::pump`] has already reserved in `live`.
    fn launch(&self, id: u64, live: Live) {
        let me = self.clone();
        tokio::spawn(async move {
            me.set_status(id, Status::Running);
            let res = me.run(id, live.clone()).await;
            let st = match res {
                Ok(true) => Status::Done,
                Ok(false) => Status::Paused,
                Err(e) => {
                    tracing::warn!("download {id} failed: {e:#}");
                    Status::Failed(format!("{e:#}"))
                }
            };
            {
                let mut db = me.0.db.lock().unwrap();
                if let Some(it) = db.items.get_mut(&id) {
                    it.downloaded = live.progress.load(Ordering::Relaxed);
                    if st == Status::Done {
                        it.finished_at = Some(now_secs());
                        // A finished download leaves the scheduled queue, as in IDM.
                        it.scheduled = false;
                    }
                    it.status = st;
                }
            }
            me.0.live.lock().unwrap().remove(&id);
            me.save();
            me.pump();
        });
    }

    fn set_status(&self, id: u64, st: Status) {
        let mut db = self.0.db.lock().unwrap();
        if let Some(it) = db.items.get_mut(&id) {
            if st == Status::Running {
                it.last_try = Some(now_secs());
            }
            it.status = st;
        }
    }

    fn headers(req: &Request) -> HeaderMap {
        let mut h = HeaderMap::new();
        let mut put = |k: header::HeaderName, v: &Option<String>| {
            if let Some(Ok(hv)) = v.as_deref().map(HeaderValue::from_str) {
                h.insert(k, hv);
            }
        };
        put(header::REFERER, &req.referer);
        put(header::COOKIE, &req.cookie);
        put(header::USER_AGENT, &req.user_agent);
        h
    }

    /// One 1-byte ranged GET tells us size, range support, name and validators in one request.
    async fn probe(&self, url: &str, headers: &HeaderMap) -> Result<Probe> {
        let r = self.0.http.get(url).headers(headers.clone()).header(header::RANGE, "bytes=0-0").send().await?;
        // `416` is how a server reports "that range does not exist", which for `bytes=0-0`
        // means the resource is empty. Treating it as an error made zero-byte files
        // undownloadable.
        let empty = r.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE;
        let r = if empty { r } else { r.error_for_status()? };
        let partial = !empty && r.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        let h = r.headers();
        let text = |k: header::HeaderName| h.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
        let size = if empty {
            Some(0)
        } else if partial {
            text(header::CONTENT_RANGE).as_deref().and_then(parse_content_range).and_then(|(_, total)| total)
        } else {
            r.content_length()
        };
        // A weak ETag (`W/"…"`) may not be used in `If-Range`, so it is not kept.
        let etag = text(header::ETAG).filter(|e| !e.starts_with("W/") && !e.is_empty());
        let filename = text(header::CONTENT_DISPOSITION).as_deref().and_then(filename_from_disposition);
        let last_modified = text(header::LAST_MODIFIED);
        let content_type = text(header::CONTENT_TYPE).map(|t| t.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
        // Dropping the response closes the connection, so a server that ignored the range and
        // started sending the whole file is not read any further.
        drop(r);
        Ok(Probe { size, resumable: partial && size.is_some(), filename, etag, last_modified, content_type })
    }

    /// Returns Ok(true) when finished, Ok(false) when paused/cancelled.
    async fn run(&self, id: u64, live: Live) -> Result<bool> {
        let mut item = self.get(id).ok_or_else(|| anyhow!("vanished"))?;
        let headers = Self::headers(&item.request);

        if item.segments.is_empty() {
            let p = self.probe(&item.request.url, &headers).await?;
            if item.request.filename.is_none() {
                if let Some(n) = &p.filename {
                    let dir = item.path.parent().unwrap().to_path_buf();
                    // Other downloads must not be renamed onto the same file either.
                    let claimed: HashSet<PathBuf> = {
                        let db = self.0.db.lock().unwrap();
                        db.items.iter().filter(|(k, _)| **k != id).map(|(_, i)| i.path.clone()).collect()
                    };
                    item.path = unique_path(&dir, &sanitize_filename(n), |p| claimed.contains(p));
                }
            }
            item.size = p.size;
            item.resumable = p.resumable;
            item.etag = p.etag;
            item.last_modified = p.last_modified;
            let n = self.0.settings.lock().unwrap().connections as u64;
            item.segments = initial_segments(p.size, p.resumable, n);
            // Update in place rather than inserting: the download may have been removed while the
            // probe was in flight, and re-inserting would resurrect it.
            {
                let mut db = self.0.db.lock().unwrap();
                match db.items.get_mut(&id) {
                    Some(slot) => *slot = item.clone(),
                    None => return Ok(false),
                }
            }
            self.save();
        }

        let part = part_path(&item.path);
        {
            let f = tokio::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&part).await?;
            // The database can be ahead of the part file: it is written without fsync, and the
            // file can be deleted or truncated between runs. `set_len` would silently fill that
            // gap with zeros, so check first and begin again when the bytes are not there.
            let on_disk = f.metadata().await?.len();
            let restart = !item.resumable
                || on_disk < item.segments.iter().map(|s| s.start + s.done).max().unwrap_or(0);
            if restart && item.segments.iter().any(|s| s.done > 0 || s.finished) {
                tracing::warn!("saved progress for download {id} is not on disk; restarting it");
                for s in &mut item.segments {
                    s.done = 0;
                    s.finished = false;
                }
                if let Some(slot) = self.0.db.lock().unwrap().items.get_mut(&id) {
                    *slot = item.clone();
                }
            }
            if let Some(sz) = item.size {
                f.set_len(sz).await?;
            }
        }
        live.progress.store(item.segments.iter().map(|s| s.done).sum(), Ordering::Relaxed);

        let splittable = item.resumable && item.size.is_some_and(|s| s >= MIN_SPLIT);
        let incomplete = item.segments.iter().filter(|s| !s.complete()).count();
        let connections = self.0.settings.lock().unwrap().connections.max(1);
        let workers = match (incomplete, splittable) {
            (0, _) => 0,
            // More connections than ranges is fine: the extra ones split the largest range.
            (_, true) => connections,
            (_, false) => 1,
        };
        for s in &mut item.segments {
            s.active = false;
            s.claimed = 0;
        }
        {
            let mut p = live.plan.lock().unwrap();
            p.segs = item.segments.clone();
            p.workers = workers;
        }
        live.limiter.set_rate(item.speed_limit.unwrap_or(0));

        let ctx = Arc::new(RunCtx {
            id,
            url: item.request.url.clone(),
            headers,
            part: part.clone(),
            resumable: item.resumable,
            size: item.size,
            validator: if item.resumable { item.etag.clone() } else { None },
            splittable,
            live: live.clone(),
        });
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..workers {
            let (me, ctx) = (self.clone(), ctx.clone());
            tasks.spawn(async move { me.worker(ctx).await });
        }
        // periodic checkpoint so a crash loses little
        let ticker = {
            let (me, live) = (self.clone(), live.clone());
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    me.checkpoint(id, &live);
                }
            })
        };
        let mut first_err = None;
        while let Some(r) = tasks.join_next().await {
            if let Err(e) = r.map_err(|e| anyhow!(e)).and_then(|r| r) {
                first_err.get_or_insert(e);
                live.cancel.store(true, Ordering::SeqCst); // stop siblings
            }
        }
        ticker.abort();
        self.checkpoint(id, &live);
        if let Some(e) = first_err {
            return Err(e);
        }
        if live.cancel.load(Ordering::SeqCst) {
            return Ok(false);
        }
        if !live.plan.lock().unwrap().segs.iter().all(Segment::complete) {
            // Every connection stopped without finishing and without reporting why. It should
            // not happen; if it does, say so rather than calling it "paused".
            bail!("the transfer stopped before the file was complete; resume it to try again");
        }
        tokio::fs::rename(&part, &item.path).await?;
        Ok(true)
    }

    fn checkpoint(&self, id: u64, live: &Live) {
        if let Some(it) = self.0.db.lock().unwrap().items.get_mut(&id) {
            it.segments = live.plan.lock().unwrap().segs.clone();
            it.downloaded = live.progress.load(Ordering::Relaxed);
        }
        self.save();
    }

    /// One connection: take a range, fetch it, take another, until nothing is left to take.
    async fn worker(&self, ctx: Arc<RunCtx>) -> Result<()> {
        let plan = &ctx.live.plan;
        loop {
            let idx = {
                let mut p = plan.lock().unwrap();
                let next = if ctx.live.cancel.load(Ordering::SeqCst) { None } else { claim(&mut p.segs, ctx.splittable) };
                match next {
                    Some(i) => i,
                    None => {
                        p.workers -= 1;
                        return Ok(());
                    }
                }
            };
            let r = self.fetch_segment(idx, &ctx).await;
            let mut p = plan.lock().unwrap();
            p.segs[idx].active = false;
            p.segs[idx].claimed = 0;
            if let Err(e) = r {
                p.workers -= 1;
                // While other connections are still working, a refused or broken one hands its
                // range back and bows out: many servers cap connections per client, and carrying
                // on with fewer — what IDM does — beats failing the whole download.
                if p.workers > 0 && !is_fatal(&e) && !ctx.live.cancel.load(Ordering::SeqCst) {
                    tracing::info!("download {}: dropping a connection ({e:#}); {} still working", ctx.id, p.workers);
                    return Ok(());
                }
                return Err(e);
            }
        }
    }

    /// Fetch range `idx` until it is complete, retrying transient failures.
    async fn fetch_segment(&self, idx: usize, ctx: &RunCtx) -> Result<()> {
        let mut attempt = 0;
        loop {
            if ctx.live.cancel.load(Ordering::SeqCst) {
                return Ok(());
            }
            let (from, end) = {
                let mut p = ctx.live.plan.lock().unwrap();
                let s = &mut p.segs[idx];
                if s.complete() {
                    return Ok(());
                }
                // A server that ignores Range means we cannot continue a partial transfer: the
                // next request returns the file from byte 0, so any progress already recorded is
                // unusable. Reset it and rewrite the whole range rather than writing the new body
                // over the head of the old one and leaving a stale tail behind.
                if !ctx.resumable && s.done > 0 {
                    let done = std::mem::take(&mut s.done);
                    let p = &ctx.live.progress;
                    p.fetch_sub(done.min(p.load(Ordering::Relaxed)), Ordering::Relaxed);
                }
                (s.start + s.done, s.end)
            };
            match self.transfer(idx, ctx, from, end).await {
                Ok(()) => {
                    if ctx.live.cancel.load(Ordering::SeqCst) || ctx.live.plan.lock().unwrap().segs[idx].complete() {
                        return Ok(());
                    }
                }
                Err(e) => {
                    if is_fatal(&e) || ctx.live.cancel.load(Ordering::SeqCst) {
                        return Err(e);
                    }
                    // A server refusing an extra connection will keep refusing it; when others
                    // are working, give the range back at once rather than after a minute of
                    // retries.
                    if is_busy(&e) && ctx.live.plan.lock().unwrap().workers > 1 {
                        return Err(e);
                    }
                    attempt += 1;
                    if attempt > MAX_RETRIES {
                        return Err(e);
                    }
                    tracing::debug!("download {} range {idx} retry {attempt}: {e:#}", ctx.id);
                    nap(Duration::from_millis(500 * 2u64.pow(attempt)), &ctx.live.cancel).await;
                }
            }
        }
    }

    /// One request for range `idx`, starting at `from`. Writes until the range is complete (its
    /// end may move closer while this runs, when another connection takes the far half), the
    /// server stops sending, or the download is cancelled.
    async fn transfer(&self, idx: usize, ctx: &RunCtx, from: u64, end: u64) -> Result<()> {
        let open_ended = end == u64::MAX;
        let mut rb = self.0.http.get(&ctx.url).headers(ctx.headers.clone());
        if ctx.resumable {
            rb = rb.header(header::RANGE, format!("bytes={from}-{end}"));
            if let Some(v) = &ctx.validator {
                rb = rb.header(header::IF_RANGE, v.as_str());
            }
        }
        let resp = rb.send().await?;
        let resp = if ctx.resumable {
            // With `If-Range`, a full `200` instead of `206` is the server saying the file is no
            // longer the one the validator describes.
            if resp.status() == reqwest::StatusCode::OK && ctx.validator.is_some() {
                return Err(ChangedOnServer("the server no longer recognises the version being resumed".into()).into());
            }
            let resp = resp.error_for_status()?;
            if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                bail!("server stopped honouring Range (HTTP {})", resp.status());
            }
            let cr = resp.headers().get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(parse_content_range);
            if let Some((start, total)) = cr {
                if let (Some(total), Some(size)) = (total, ctx.size) {
                    if total != size {
                        return Err(ChangedOnServer(format!("it is now {total} bytes, it was {size}")).into());
                    }
                }
                // Writing bytes that start somewhere else at `from` would corrupt the file.
                if start != from {
                    bail!("the server sent bytes from {start} when asked for {from}");
                }
            }
            resp
        } else {
            resp.error_for_status()?
        };

        let mut f = tokio::fs::OpenOptions::new().write(true).open(&ctx.part).await?;
        f.seek(std::io::SeekFrom::Start(from)).await?;
        let mut stream = resp.bytes_stream();
        let cancel = &ctx.live.cancel;
        // Read on a short tick: it lets Pause take effect while a transfer is stalled, and gives
        // up on a server that accepted the request and then went silent instead of holding the
        // slot forever.
        let mut idle = Duration::ZERO;
        let mut ended = false;
        'read: loop {
            if cancel.load(Ordering::SeqCst) {
                f.flush().await?;
                return Ok(());
            }
            let chunk = match tokio::time::timeout(IDLE_TICK, stream.next()).await {
                Err(_) => {
                    idle += IDLE_TICK;
                    if idle >= IDLE_LIMIT {
                        bail!("no data received for {}s", IDLE_LIMIT.as_secs());
                    }
                    continue;
                }
                Ok(None) => {
                    ended = true;
                    break;
                }
                Ok(Some(c)) => {
                    idle = Duration::ZERO;
                    c?
                }
            };
            let mut off = 0;
            while off < chunk.len() {
                let want = (chunk.len() - off) as u64;
                // Pace first, then claim: the claim should cover a write, not a wait.
                ctx.live.limiter.take(want, cancel).await;
                self.0.limiter.take(want, cancel).await;
                if cancel.load(Ordering::SeqCst) {
                    f.flush().await?;
                    return Ok(());
                }
                // How much of this chunk is still ours: another connection may have taken the
                // far end of the range since the request went out.
                let take = {
                    let mut p = ctx.live.plan.lock().unwrap();
                    let s = &mut p.segs[idx];
                    let pos = s.start + s.done;
                    let room = if s.end == u64::MAX { u64::MAX } else { (s.end + 1).saturating_sub(pos) };
                    let t = room.min(want);
                    s.claimed = t;
                    t as usize
                };
                if take > 0 {
                    f.write_all(&chunk[off..off + take]).await?;
                    {
                        let mut p = ctx.live.plan.lock().unwrap();
                        let s = &mut p.segs[idx];
                        s.done += take as u64;
                        s.claimed = 0;
                    }
                    ctx.live.progress.fetch_add(take as u64, Ordering::Relaxed);
                    off += take;
                }
                if (take as u64) < want {
                    // The range is complete; the rest of this response belongs to someone else.
                    break 'read;
                }
            }
        }
        f.flush().await?;
        let mut p = ctx.live.plan.lock().unwrap();
        let s = &mut p.segs[idx];
        if open_ended {
            if ended {
                s.finished = true;
            }
        } else if !s.complete() {
            bail!("connection closed early ({} bytes missing)", s.end + 1 - (s.start + s.done));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a:b*c?.zip"), "a_b_c_.zip");
        assert_eq!(sanitize_filename("..."), "download");
        assert_eq!(sanitize_filename("C:\\x\\y.exe"), "y.exe");
        assert_eq!(sanitize_filename(""), "download");
    }

    #[test]
    fn sanitize_windows_devices() {
        // Writing to these silently discards the data on Windows.
        assert_eq!(sanitize_filename("NUL"), "_NUL");
        assert_eq!(sanitize_filename("con.txt"), "_con.txt");
        assert_eq!(sanitize_filename("LPT1"), "_LPT1");
        assert_eq!(sanitize_filename("com9.log"), "_com9.log");
        // Near misses stay untouched.
        assert_eq!(sanitize_filename("console.txt"), "console.txt");
        assert_eq!(sanitize_filename("COM10.txt"), "COM10.txt");
    }

    #[test]
    fn disposition() {
        assert_eq!(filename_from_disposition("attachment; filename=\"a b.zip\"").as_deref(), Some("a b.zip"));
        assert_eq!(filename_from_disposition("attachment; filename*=UTF-8''%E2%82%AC.txt").as_deref(), Some("€.txt"));
        // A semicolon inside the quoted name must not split the parameter.
        assert_eq!(filename_from_disposition("attachment; filename=\"a;b.zip\"").as_deref(), Some("a;b.zip"));
        // RFC 5987 with a language tag, and filename* winning over filename.
        assert_eq!(filename_from_disposition("attachment; filename*=UTF-8'en'caf%C3%A9.txt").as_deref(), Some("café.txt"));
        assert_eq!(
            filename_from_disposition("attachment; filename=\"plain.txt\"; filename*=UTF-8''better.txt").as_deref(),
            Some("better.txt")
        );
        assert_eq!(filename_from_disposition("attachment").as_deref(), None);
    }

    #[test]
    fn url_identity_ignores_fragment_and_query_order() {
        let k = |u: &str| url_key(&url::Url::parse(u).unwrap());
        assert_eq!(k("https://a.example/f.zip?a=1&b=2"), k("https://a.example/f.zip?b=2&a=1"));
        assert_eq!(k("https://a.example/f.zip#part2"), k("https://a.example/f.zip"));
        assert_eq!(k("https://A.example/f.zip"), k("https://a.example/f.zip"), "host is case-insensitive");
        // Genuinely different downloads must not collide.
        assert_ne!(k("https://a.example/f.zip?a=1"), k("https://a.example/f.zip?a=2"));
        assert_ne!(k("https://a.example/f.zip"), k("https://b.example/f.zip"));
        assert_ne!(k("https://a.example/f.zip"), k("https://a.example/g.zip"));
        assert_ne!(k("http://a.example/f.zip"), k("https://a.example/f.zip"));
    }

    #[test]
    fn segment_completion_is_inclusive() {
        let full = Segment { done: 10, ..Segment::new(10, 19) };
        assert!(full.complete(), "end is inclusive, so 10..=19 is 10 bytes");
        let short = Segment { done: 9, ..Segment::new(10, 19) };
        assert!(!short.complete());
        let unknown = Segment { done: 5, ..Segment::new(0, u64::MAX) };
        assert!(!unknown.complete());
        let finished = Segment { done: 5, finished: true, ..Segment::new(0, u64::MAX) };
        assert!(finished.complete());
    }

    #[test]
    fn content_range_parses() {
        assert_eq!(parse_content_range("bytes 0-0/1234"), Some((0, Some(1234))));
        assert_eq!(parse_content_range("bytes 100-199/*"), Some((100, None)));
        assert_eq!(parse_content_range("bytes */0"), None);
        assert_eq!(parse_content_range("items 1-2/3"), None);
    }

    /// The bytes every range covers, in order, so tests can check nothing overlaps or is lost.
    fn coverage(segs: &[Segment]) -> Vec<(u64, u64)> {
        let mut v: Vec<(u64, u64)> = segs.iter().map(|s| (s.start, s.end)).collect();
        v.sort();
        v
    }

    #[test]
    fn an_idle_connection_takes_an_unowned_range_first() {
        let mut segs = initial_segments(Some(8 << 20), true, 4);
        segs[0].active = true;
        assert_eq!(claim(&mut segs, true), Some(1));
        assert!(segs[1].active);
        assert_eq!(segs.len(), 4, "nothing was split");
    }

    #[test]
    fn splitting_halves_the_largest_remaining_range_without_overlap() {
        let mut segs = initial_segments(Some(8 << 20), true, 2);
        for s in &mut segs {
            s.active = true;
        }
        segs[0].done = 4 << 20; // finished
        segs[1].done = 1 << 20; // 3 MiB to go
        segs[1].claimed = 64 << 10; // and a write in flight
        segs[0].active = false;
        let n = claim(&mut segs, true).expect("a split");
        assert_eq!(n, 2);
        let victim = &segs[1];
        let piece = &segs[2];
        // The in-flight write stays with its owner.
        assert!(victim.end >= victim.start + victim.done + victim.claimed - 1);
        assert_eq!(piece.start, victim.end + 1);
        assert_eq!(piece.end, (8 << 20) - 1);
        // The two halves are within a byte of each other.
        let left = victim.end + 1 - (victim.start + victim.done + victim.claimed);
        let right = piece.end + 1 - piece.start;
        assert!(left.abs_diff(right) <= 1, "{left} vs {right}");
        // Still exactly one cover of the file.
        let cov = coverage(&segs);
        assert_eq!(cov.first().unwrap().0, 0);
        for w in cov.windows(2) {
            assert_eq!(w[0].1 + 1, w[1].0);
        }
        assert_eq!(cov.last().unwrap().1, (8 << 20) - 1);
    }

    #[test]
    fn small_remainders_and_unsplittable_downloads_are_left_alone() {
        let mut segs = initial_segments(Some(4 << 20), true, 1);
        segs[0].active = true;
        segs[0].done = (4 << 20) - MIN_PIECE; // less than two pieces left
        assert_eq!(claim(&mut segs, true), None);
        let mut one = initial_segments(Some(4 << 20), false, 8);
        one[0].active = true;
        assert_eq!(claim(&mut one, false), None, "a server without ranges cannot be split");
    }
}
