//! Segmented, resumable download engine used by the reIDM daemon and CLI.
//!
//! A download is fetched over several connections, each owning a byte range (a *segment*).
//! When a connection finishes its range early it takes over half of the largest range another
//! connection is still working on — IDM's dynamic segmentation — so one slow connection does
//! not decide how long the whole file takes.

pub mod category;
pub mod checksum;
mod disk;
pub mod grabber;
mod limiter;
pub mod lists;
pub mod media;
pub mod power;
pub mod queue;
pub mod schedule;
mod settings;
mod space;
mod store;
pub mod torrent;
pub mod update;

pub use category::Category;
pub use checksum::Checksum;
pub use power::WhenDone;
pub use queue::Move;
pub use limiter::Limiter;
pub use media::{valid_format, Media};
pub use settings::{Performance, Profile, Proxy, ProxyKind, ProxyMode, Settings, SettingsPatch};
pub use torrent::{TorrentFile, TorrentInfo, TorrentRequest, TorrentStatus};

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use store::{Db, DbCell};

const MIN_SPLIT: u64 = 1 << 20; // don't split files under 1 MiB
/// Dynamic segmentation never hands a connection less than this: below it, the cost of a new
/// request is more than the time the extra connection would save.
const MIN_PIECE: u64 = 256 << 10;
// How long a server may stay silent or take to answer, how often a connection retries, and how
// much it collects before writing (256 KiB by default: writing every 8-16 KiB network read cost a
// thread hand-off each, about a core at 1 GB/s, and many small interleaved writes make a hard
// disk seek) are settings: see `Settings::idle_timeout_secs`, `connect_timeout_secs`, `retries`
// and `write_buffer_kib`. A request that is never answered must time out at all: one sent over a
// connection that died (a Wi-Fi switch, a NAT timeout) otherwise waits for the system to give up
// on it, which takes a quarter of an hour.
/// A connection writes what it holds after this long, however little: a slow download should
/// still show progress.
const WRITE_AT_LEAST_EVERY: Duration = Duration::from_secs(1);
/// How often a running download's progress is made durable and recorded.
const CHECKPOINT_EVERY: Duration = Duration::from_secs(3);
/// How often a download that has lost connections (a flaky line, a server that refused one
/// for a while) tries to get back up to its full number.
const TOP_UP_EVERY: Duration = Duration::from_secs(30);
/// How long to wait for a download to stop before changing what it is.
const STOP_LIMIT: Duration = Duration::from_secs(15);
/// The longest file name reIDM gives a download, in bytes. File systems allow 255, and the part
/// file adds its id and `.reidm` to the name.
const MAX_NAME_BYTES: usize = 200;
/// How long "use the next link the browser sends as this download's new address" stays armed.
const REFRESH_WINDOW: Duration = Duration::from_secs(180);
/// How often downloads stopped for want of space look again.
const SPACE_RETRY_EVERY: Duration = Duration::from_secs(30);
/// How often a video or torrent download (whose writes reIDM doesn't see) checks the space left.
const SPACE_CHECK_EVERY: Duration = Duration::from_secs(10);
/// Queue positions start here, so that *Move to top* has room below them for a very long time
/// before the list has to be numbered again.
const ORDER_BASE: u64 = 1 << 32;
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
    /// Fetch through yt-dlp: `url` is then the page with the video, and this says which
    /// format. See [`media`](crate::media).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<Media>,
    /// A torrent: `url` is then a magnet link or the address of a `.torrent` file. See
    /// [`torrent`](crate::torrent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent: Option<TorrentRequest>,
    /// What the finished file's checksum should be. Checked once it is in place; a file that
    /// differs is kept, and the download fails saying so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<Checksum>,
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
/// The fragment is dropped (it is not sent to the server) and the query's parameters are sorted
/// by name, so `?b=2&a=1` and `?a=1&b=2` are the same download. Everything else is kept: two
/// URLs that differ in host, path or query values really are different files. Repeated
/// parameters keep their order (`?id=2&id=1` is not `?id=1&id=2`), and values are encoded again
/// after decoding, so an encoded `&` in a value can't pass for a separator.
pub fn url_key(u: &url::Url) -> String {
    // A magnet link is its torrent, whatever trackers and name ride along.
    if u.scheme() == "magnet" {
        if let Some((_, xt)) = u.query_pairs().find(|(k, _)| k == "xt") {
            // Hex or base32, the same hash is the same torrent.
            return torrent::btih_hex(&xt).map_or_else(|| format!("magnet:{}", xt.to_ascii_lowercase()), |h| format!("magnet:urn:btih:{h}"));
        }
    }
    let mut q: Vec<(String, String)> = u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    q.sort_by(|a, b| a.0.cmp(&b.0));
    let query = if q.is_empty() { String::new() } else { format!("?{}", url::form_urlencoded::Serializer::new(String::new()).extend_pairs(q).finish()) };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    format!("{}{host}{port}{}{query}", u.scheme(), u.path())
}

/// Whether `i`, already known to have the right address, is the download `media` describes.
/// The same page in two formats is two downloads.
fn same_media(i: &Item, media: Option<&Media>) -> bool {
    i.request.media.as_ref().map(|m| &m.format) == media.map(|m| &m.format)
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
    /// What a running download is busy with when its progress can't say it: a video download
    /// fetching yt-dlp, looking up the video, joining video and audio. Cleared when it stops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// A torrent's name, files, peers and what it has shared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent: Option<TorrentStatus>,
    /// Where the unfinished data is, once there is some. Recorded rather than worked out again
    /// each time, so that changing the temporary folder (or losing the settings) does not start
    /// every unfinished download again from nothing and leave its old data behind. Lists from
    /// before 0.8 don't have it; their downloads find their part file where it always was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    part: Option<PathBuf>,
    /// The file at `path` is this download's own earlier copy, which finishing may replace:
    /// set when a finished download is downloaded again. Anything else already at `path` when a
    /// download finishes is kept, and the download takes another name.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    replace: bool,
    /// Its place in the queue: of the downloads waiting, the lowest starts first. Lists from
    /// before 0.8 have none; they are given one in id order when loaded.
    #[serde(default)]
    pub order: u64,
    /// Whether the finished file matched [`Request::checksum`]; `None` when there was none to
    /// check, or it has not been checked yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum_ok: Option<bool>,
    /// Stopped because the disk was too full to carry on; started again by itself once there
    /// is room (see [`Manager::spawn_space_watch`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub waiting_for_space: bool,
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

/// A running download's stop signal: a flag for code that checks it, and a wake-up for code
/// that is waiting — on a server, on the speed limit — so that Pause takes effect at once
/// whatever the download is doing.
#[derive(Default)]
struct Cancel {
    flag: AtomicBool,
    wake: tokio::sync::Notify,
}

/// The flag itself, for code that only looks at it now and then (`cancel.load(…)`). Stopping a
/// download goes through [`Cancel::set`], which also wakes whatever is waiting.
impl std::ops::Deref for Cancel {
    type Target = AtomicBool;
    fn deref(&self) -> &AtomicBool {
        &self.flag
    }
}

impl Cancel {
    fn set(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    /// Completes once the download has been told to stop.
    async fn cancelled(&self) {
        loop {
            let woken = self.wake.notified();
            tokio::pin!(woken);
            // Registered before the flag is looked at, so a `set` in between is not missed.
            woken.as_mut().enable();
            if self.flag.load(Ordering::SeqCst) {
                return;
            }
            woken.await;
        }
    }
}

/// How many bytes a running download has. Every change moves the list's revision on, so a
/// window asking "has anything changed?" sees progress too.
struct Progress {
    bytes: AtomicU64,
    rev: Arc<AtomicU64>,
}

impl Progress {
    fn load(&self, o: Ordering) -> u64 {
        self.bytes.load(o)
    }

    fn store(&self, v: u64, o: Ordering) {
        if self.bytes.swap(v, o) != v {
            self.touch();
        }
    }

    fn add(&self, n: u64) {
        self.bytes.fetch_add(n, Ordering::Relaxed);
        self.touch();
    }

    fn sub(&self, n: u64) {
        // A compare-and-swap loop rather than `fetch_update`, which newer Rust deprecates under a
        // new name (`try_update`) that older toolchains don't have.
        let mut cur = self.bytes.load(Ordering::Relaxed);
        while let Err(seen) = self.bytes.compare_exchange_weak(cur, cur.saturating_sub(n), Ordering::Relaxed, Ordering::Relaxed) {
            cur = seen;
        }
        self.touch();
    }

    /// Something else about the download that a list shows has changed (which ranges are being
    /// worked on).
    fn touch(&self) {
        self.rev.fetch_add(1, Ordering::Relaxed);
    }
}

/// The handles a running download shares between its connections and the rest of the manager.
#[derive(Clone)]
struct Live {
    cancel: Arc<Cancel>,
    progress: Arc<Progress>,
    plan: Arc<Mutex<Plan>>,
    /// This download's own limit; the global one lives on the manager.
    limiter: Arc<Limiter>,
    /// Resume was pressed while this run was stopping: queue it again once it has stopped,
    /// instead of leaving it paused as if the click had not happened.
    requeue: Arc<AtomicBool>,
    /// Removed while running: the row as it was, and whether to delete its files. Its files are
    /// deleted once its connections have stopped writing them.
    removed: Arc<Mutex<Option<(Item, bool)>>>,
    /// Stopped because the disk is too full to carry on, and why: it then waits for room
    /// instead of for the user.
    no_space: Arc<Mutex<Option<String>>>,
}

impl Live {
    fn new(downloaded: u64, speed_limit: Option<u64>, rev: &Arc<AtomicU64>) -> Self {
        Live {
            cancel: Arc::default(),
            progress: Arc::new(Progress { bytes: AtomicU64::new(downloaded), rev: rev.clone() }),
            plan: Arc::default(),
            limiter: Arc::new(Limiter::new(speed_limit.unwrap_or(0))),
            requeue: Arc::default(),
            removed: Arc::default(),
            no_space: Arc::default(),
        }
    }

    /// Stop this run for want of space, saying why.
    fn stop_for_space(&self, why: String) {
        *self.no_space.lock().unwrap() = Some(why);
        self.cancel.set();
    }
}

/// What every connection of one run needs, built once per run.
struct RunCtx {
    id: u64,
    url: String,
    headers: HeaderMap,
    part: PathBuf,
    /// The part file, shared by every connection: each writes at its own offset.
    file: Arc<std::fs::File>,
    resumable: bool,
    size: Option<u64>,
    /// `If-Range` value: the strong `ETag` from the first response, when there was one.
    validator: Option<String>,
    /// The first response's `Last-Modified`: without an `ETag`, a different date on a later
    /// response is how a changed file shows.
    last_modified: Option<String>,
    /// Ranges can only be split when the server serves ranges of a file of known length.
    splittable: bool,
    live: Live,
    /// The settings when the run began: retries, time limits, the write buffer.
    opts: Settings,
}

pub(crate) fn yes() -> bool {
    true
}

struct Inner {
    db: Arc<DbCell>,
    /// Moves on with every change a list of downloads would show; see [`Manager::revision`].
    rev: Arc<AtomicU64>,
    /// What went wrong loading the list or settings, for the user to be told.
    warnings: Mutex<Vec<String>>,
    live: Mutex<BTreeMap<u64, Live>>,
    db_path: PathBuf,
    settings_path: PathBuf,
    settings: Mutex<Settings>,
    /// Built from the settings (proxy, time limits), and built again when they change. A
    /// running download takes the new one for its next connection.
    http: RwLock<reqwest::Client>,
    /// What to do when every download is done, and its countdown.
    when_done: Mutex<power::Countdown>,
    /// The global speed limit, shared by every connection of every download.
    limiter: Arc<Limiter>,
    /// A download waiting for the browser to supply a fresh address, and until when.
    refresh: Mutex<Option<(u64, Instant)>>,
    /// The BitTorrent session, started by the first torrent.
    torrents: tokio::sync::OnceCell<Arc<librqbit::Session>>,
    #[cfg(test)]
    faults: Arc<tests::Faults>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // The saving thread writes what is pending, then ends.
        self.db.close();
    }
}

#[derive(Clone)]
pub struct Manager(Arc<Inner>);

/// Where the list, settings and token live. `REIDM_DATA_DIR` moves it — the only way to on
/// Windows and macOS, whose folders don't follow the environment the way `XDG_DATA_HOME` does.
pub fn default_data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("REIDM_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    directories::ProjectDirs::from("com", "Zektopic", "reIDM")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".reidm"))
}

pub fn default_download_dir() -> PathBuf {
    let dirs = directories::UserDirs::new();
    pick_download_dir(dirs.as_ref().and_then(|d| d.download_dir().map(Path::to_path_buf)), dirs.as_ref().map(|d| d.home_dir().to_path_buf()))
}

/// The system's Downloads folder; else `Downloads` in the home folder. A minimal Linux install
/// often has no `user-dirs.dirs`, and then the system names no Downloads folder at all: the
/// working directory (what this used to fall back to) is wherever the service happened to be
/// started — `/` for a login item — so downloads landed somewhere nobody would look.
fn pick_download_dir(system: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    system.or_else(|| home.map(|h| h.join("Downloads"))).unwrap_or_else(|| PathBuf::from("."))
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

/// Characters that reorder how text is displayed. In a file name they let `invoice<U+202E>fdp.exe`
/// show as `invoiceexe.pdf`: a program dressed as a document.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Cut a name to [`MAX_NAME_BYTES`], keeping its extension, so the file (and its part file,
/// whose name is longer) can be created at all.
fn shorten_name(name: String) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && e.len() <= 16 => (s, format!(".{e}")),
        _ => (name.as_str(), String::new()),
    };
    let mut cut = MAX_NAME_BYTES - ext.len();
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{ext}", stem[..cut].trim_end())
}

/// Strip path separators and control characters so a hostile server/page cannot escape `dir`.
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() || is_bidi_control(c) || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let cleaned = shorten_name(cleaned.trim().trim_matches('.').to_string());
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

/// `bytes 100-199/1000` → `(100, 199, Some(1000))`; `bytes 100-199/*` → `(100, 199, None)`.
fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes")?.trim();
    let (range, total) = rest.split_once('/')?;
    let (a, b) = range.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?, total.trim().parse().ok()))
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

/// Up to `want` names for a download that are free on disk — neither the file nor its part
/// file exists — best first. The list's own claims are checked later, under its lock; this is
/// here because it reads the folder, and on a slow or synced folder (OneDrive, iCloud, a
/// network share) one stat can take a long time: doing that under the list's lock would hold up
/// every task that needs the list, the window's polls among them.
fn free_names(dir: &Path, name: &str, want: usize) -> Vec<PathBuf> {
    let free = |p: &Path| !p.exists() && !part_path(p).exists();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    let mut out = Vec::with_capacity(want);
    let mut p = dir.join(name);
    if free(&p) {
        out.push(p);
    }
    // Enough names for the usual case (many downloads of one file name, or none), but not a
    // thousand stats for a folder where every name is taken: the caller then chooses under the
    // lock, as it always used to.
    for n in 1..=1_000 {
        if out.len() >= want {
            break;
        }
        p = dir.join(format!("{stem} ({n}){ext}"));
        if free(&p) {
            out.push(p);
        }
    }
    out
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

/// A failed write to the part file, said in a way that tells the user what to do.
fn write_error(e: std::io::Error, part: &Path) -> anyhow::Error {
    let what = match e.kind() {
        std::io::ErrorKind::StorageFull => "the disk is full: free some space, then resume the download; what it has so far is kept",
        std::io::ErrorKind::FileTooLarge => "the drive can't hold a file this large (FAT32 drives stop at 4 GB): choose a folder on another drive",
        _ => "check the disk, then resume the download; what it has so far is kept",
    };
    anyhow::Error::new(e).context(format!("cannot write the download to {} — {what}", part.display()))
}

/// `429 Too Many Requests` / `503 Service Unavailable`: how servers that cap connections per
/// client usually refuse an extra one.
fn is_busy(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<reqwest::Error>().and_then(reqwest::Error::status).map(|s| s.as_u16()),
        Some(429) | Some(503)
    )
}

/// Sleep, but wake early when the download is stopped.
async fn nap(d: Duration, cancel: &Cancel) {
    tokio::select! {
        _ = tokio::time::sleep(d) => {}
        _ = cancel.cancelled() => {}
    }
}

/// Where the scheduler's next check starts from. When the clocks go back an hour, local time
/// repeats that hour: moving the mark back with it would make a start or stop in that hour fire
/// twice, so a step back of under two hours keeps the mark where it was until the clock has
/// caught up. A bigger step back is someone setting the clock, and is followed.
fn scheduler_mark(prev: chrono::NaiveDateTime, now: chrono::NaiveDateTime) -> chrono::NaiveDateTime {
    if now < prev && prev - now < chrono::Duration::hours(2) {
        prev
    } else {
        now
    }
}

/// The client every request reIDM makes goes through, built from the settings.
fn http_client(s: &Settings) -> Result<reqwest::Client> {
    let b = reqwest::Client::builder()
        .connect_timeout(s.connect_timeout())
        // Per read, so it limits how long a server may go quiet, not how long a file may take.
        .read_timeout(s.idle_timeout())
        // A connection that died silently is noticed instead of being reused.
        .tcp_keepalive(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        // The bytes on disk must be the bytes the server stores. With transparent
        // decompression a misconfigured server's `.gz` would be saved decompressed, and a
        // compressed response to a range request cannot be decoded at all.
        .no_gzip()
        .no_brotli();
    Ok(s.proxy.configure(b)?.build()?)
}

/// A size as people read it: `1.5 GiB`.
pub(crate) fn human(n: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let (mut v, mut i) = (n as f64, 0);
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
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

/// A request checked and ready to go into the list; see [`Manager::prepare`].
struct Prepared {
    req: Request,
    /// The address's [`url_key`].
    key: String,
    dir: PathBuf,
    /// The file name it will most likely have.
    hint: String,
    probe: Option<Probe>,
    /// Names that are free on disk, best first (see [`free_names`]): chosen before the list is
    /// locked. The first one no other download has claimed is the one used.
    candidates: Vec<PathBuf>,
}

/// What a change to the list leaves to do once the list is unlocked: file system work, which
/// must not hold up everything else waiting for the list, and starting downloads.
#[derive(Default)]
struct Effects {
    pump: bool,
    delete: Vec<PathBuf>,
    delete_dirs: Vec<PathBuf>,
    /// Media downloads whose old pieces are to be found and deleted with their file. The scan
    /// reads the download folder, so it happens in `apply`, after the list is unlocked.
    leftovers_of: Vec<PathBuf>,
    /// Torrents (by info hash) to take out of the torrent engine, with their files.
    forget: Vec<String>,
}

impl Manager {
    /// Open with the default concurrency limit; see [`Manager::open_with`].
    pub fn open(data_dir: &Path, default_dir: PathBuf, connections: usize) -> Result<Self> {
        Self::open_with(data_dir, default_dir, connections, settings::default_max_concurrent())
    }

    /// `max_concurrent` is only the default for a first run; a saved `settings.json` wins.
    ///
    /// A damaged list or settings file does not stop reIDM: the previous copy is used, or an
    /// empty one, and [`Manager::warnings`] says what happened. A file that exists but can't be
    /// read at all is an error, because carrying on would save an empty list over it.
    pub fn open_with(data_dir: &Path, default_dir: PathBuf, connections: usize, max_concurrent: usize) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let mut warnings = Vec::new();
        let db_path = data_dir.join("downloads.json");
        let mut db: Db = store::load(&db_path, "download list", &mut warnings)?.unwrap_or_else(Db::fresh);
        db.next_id = db.next_id.max(db.items.keys().next_back().map_or(1, |k| k + 1));
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
        // Lists from before the queue had an order: in the order they were added.
        let mut next_order = db.items.values().map(|i| i.order).max().map_or(ORDER_BASE, |m| (m + 1).max(ORDER_BASE));
        for it in db.items.values_mut().filter(|i| i.order == 0) {
            it.order = next_order;
            next_order += 1;
        }
        db.next_order = next_order;
        // Last: it also forgets the changes above, which are the list as loaded, not changes to it.
        db.reindex();
        // Saved options win over the values passed on the command line.
        let settings_path = data_dir.join("settings.json");
        let saved: Option<Settings> = store::load::<Settings>(&settings_path, "settings", &mut warnings)?.filter(|s| s.connections > 0);
        let mut settings = saved.unwrap_or_else(|| Settings::new(default_dir.clone(), connections, max_concurrent));
        settings.normalize();
        let http = match http_client(&settings) {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("{e:#}; reIDM connects the way the system says until the proxy is corrected in Options");
                tracing::warn!("{msg}");
                warnings.push(msg);
                http_client(&Settings { proxy: Proxy::default(), ..settings.clone() })?
            }
        };
        let limiter = Arc::new(Limiter::new(settings.effective_limit()));
        let rev = Arc::new(AtomicU64::new(1));
        Ok(Self(Arc::new(Inner {
            db: DbCell::start(db, db_path.clone(), rev.clone()),
            rev,
            warnings: Mutex::new(warnings),
            live: Mutex::new(BTreeMap::new()),
            db_path,
            settings_path,
            settings: Mutex::new(settings),
            http: RwLock::new(http),
            when_done: Mutex::default(),
            limiter,
            refresh: Mutex::new(None),
            torrents: tokio::sync::OnceCell::new(),
            #[cfg(test)]
            faults: Arc::default(),
        })))
    }

    /// A number that changes whenever anything a list of downloads shows changes: a download
    /// added or removed, a status, a name, progress. Two equal readings mean nothing changed in
    /// between, so a window polling the list can skip the ones that are the same.
    pub fn revision(&self) -> u64 {
        self.0.rev.load(Ordering::Relaxed)
    }

    /// Write the list to disk now and wait (a few seconds at most) until it is there. For
    /// shutting down: changes are otherwise written within a couple of seconds. False when the
    /// write did not succeed in time.
    pub fn flush(&self) -> bool {
        let n = self.0.db.request_save(true);
        self.0.db.wait(n, Duration::from_secs(10))
    }

    /// What the user should know about the list's storage: a damaged file that was recovered
    /// from its backup (or could not be), or saves that are failing now.
    pub fn warnings(&self) -> Vec<String> {
        let mut w = self.0.warnings.lock().unwrap().clone();
        w.extend(self.0.db.save_error());
        w
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
        if let Some(d) = &patch.default_dir {
            std::fs::create_dir_all(d).with_context(|| format!("cannot use {} as the download folder", d.display()))?;
        }
        for (c, d) in patch.category_dirs.iter().flatten() {
            if !d.is_absolute() {
                bail!("the {} folder must be a full path, not {}", c.name(), d.display());
            }
            std::fs::create_dir_all(d).with_context(|| format!("cannot use {} as the {} folder", d.display(), c.name()))?;
        }
        if let Some(d) = patch.temp_dir.as_ref().filter(|d| !d.as_os_str().is_empty()) {
            if !d.is_absolute() {
                bail!("the temporary folder must be a full path, not {}", d.display());
            }
            std::fs::create_dir_all(d).with_context(|| format!("cannot use {} as the temporary folder", d.display()))?;
        }
        let limit = {
            let mut s = self.0.settings.lock().unwrap();
            // Applied to a copy first: a proxy the HTTP client can't use is refused with nothing
            // changed, rather than half applied.
            let mut next = s.clone();
            if next.apply(patch)?.client {
                *self.0.http.write().unwrap() = http_client(&next)?;
            }
            *s = next;
            s.effective_limit()
        };
        self.0.limiter.set_rate(limit);
        self.torrent_limits();
        // Raising the limit should let waiting downloads start now, not at the next completion.
        self.pump();
        self.save_settings().context("the new options apply now, but could not be saved for next time")
    }

    /// The HTTP client as the settings have it now. Cheap: clients share their insides.
    pub(crate) fn http(&self) -> reqwest::Client {
        self.0.http.read().unwrap().clone()
    }

    fn save_settings(&self) -> Result<()> {
        let s = self.0.settings.lock().unwrap().clone();
        store::write_atomic(&self.0.settings_path, &serde_json::to_vec_pretty(&s)?)
    }

    /// Have the list saved soon. Any change made through the list's lock is saved anyway; this
    /// is for code that wants to say so.
    fn save(&self) {
        self.0.db.request_save(false);
    }

    /// Have the list saved at once: for changes that must survive a crash a moment later — a
    /// download the browser has just let go of, a status the user chose.
    fn save_now(&self) {
        self.0.db.request_save(true);
    }

    /// Wait (without holding up the runtime) until what is in the list now is on disk.
    async fn persisted(&self) {
        let n = self.0.db.request_save(true);
        let db = self.0.db.clone();
        if !tokio::task::spawn_blocking(move || db.wait(n, Duration::from_secs(10))).await.unwrap_or(false) {
            tracing::warn!("the download list could not be saved in time; the download is running, but would be forgotten if reIDM stopped now");
        }
    }

    /// Directory holding `downloads.json` (and the daemon's API token, if it is serving a UI).
    /// Where unfinished downloads are kept: the user's temporary folder, or `parts` in the data
    /// folder.
    pub fn temp_dir(&self) -> PathBuf {
        let set = self.0.settings.lock().unwrap().temp_dir.clone();
        set.unwrap_or_else(|| self.data_dir().join("parts"))
    }

    /// The part file of download `it`: the one it recorded, or for a download that has not
    /// recorded one, in the temporary folder — unless a download begun before there was one
    /// already has its part file beside the final file, where it carries on.
    fn part_for(&self, it: &Item) -> PathBuf {
        if let Some(p) = &it.part {
            return p.clone();
        }
        let beside = part_path(&it.path);
        if beside.exists() {
            return beside;
        }
        let name = it.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.temp_dir().join(format!("{}-{name}.reidm", it.id))
    }

    pub fn data_dir(&self) -> &Path {
        self.0.db_path.parent().unwrap_or(Path::new("."))
    }

    /// Every lock in this type must be taken `db` before `live`, and both before a download's
    /// own plan. Taking them the other way round would deadlock against `resume`/`start`, which
    /// hold `db` while taking `live`.
    pub fn list(&self) -> Vec<Item> {
        let mut items: Vec<Item> = self.0.db.lock().unwrap().items.values().map(|i| (**i).clone()).collect();
        let live = self.0.live.lock().unwrap();
        for it in &mut items {
            if let Some(l) = live.get(&it.id) {
                Self::overlay(it, l);
            }
        }
        items
    }

    /// The live state of a running download over its last checkpoint: the bytes received so
    /// far, and the ranges as they are now, so a segment map moves smoothly.
    fn overlay(it: &mut Item, l: &Live) {
        it.downloaded = l.progress.load(Ordering::Relaxed);
        let p = l.plan.lock().unwrap();
        if !p.segs.is_empty() {
            it.segments = p.segs.clone();
        }
    }

    /// The downloads added, changed or removed since revision `rev` (as [`Manager::revision`]
    /// said it), by id; `None` when `rev` is too long ago to say, and everything should be
    /// looked at again. Running downloads are always included: their progress moves the
    /// revision without being recorded one change at a time.
    pub fn changed_since(&self, rev: u64) -> Option<Vec<u64>> {
        let mut ids = self.0.db.changed_since(rev)?;
        ids.extend(self.0.live.lock().unwrap().keys().copied());
        ids.sort_unstable();
        ids.dedup();
        Some(ids)
    }

    /// Several downloads, as [`Manager::get`] gives each, under one lock: `None` for one that
    /// is no longer in the list.
    pub fn get_many(&self, ids: &[u64]) -> Vec<(u64, Option<Item>)> {
        let db = self.0.db.lock().unwrap();
        let live = self.0.live.lock().unwrap();
        ids.iter()
            .map(|id| {
                let it = db.items.get(id).map(|i| {
                    let mut it = (**i).clone();
                    if let Some(l) = live.get(id) {
                        Self::overlay(&mut it, l);
                    }
                    it
                });
                (*id, it)
            })
            .collect()
    }

    pub fn get(&self, id: u64) -> Option<Item> {
        let mut it = (**self.0.db.lock().unwrap().items.get(&id)?).clone();
        if let Some(l) = self.0.live.lock().unwrap().get(&id) {
            Self::overlay(&mut it, l);
        }
        Some(it)
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
        // The browser lets go of its own copy once this answers, so what is added must be on
        // disk before it does: a crash a moment later must not lose the download entirely.
        let out = self.add_checked_inner(req, opts).await?;
        self.persisted().await;
        Ok(out)
    }

    async fn add_checked_inner(&self, req: Request, opts: AddOptions) -> Result<AddOutcome> {
        // A torrent is checked by the torrent engine when it starts, not with a ranged GET —
        // except a .torrent file on the web, which is fetched here with the browser's cookies
        // (a private tracker wants them) and added from what arrives.
        let named_torrent = req.filename.as_deref().is_some_and(|f| f.to_ascii_lowercase().ends_with(".torrent"));
        if req.torrent.is_none() && req.media.is_none() && req.url.starts_with("http") && (named_torrent || torrent::is_torrent_url(&req.url)) {
            let bytes = self.fetch_torrent_file(&req).await?;
            return self.add_torrent_file(&bytes, None, opts.later);
        }
        if req.torrent.is_some() || torrent::is_torrent_url(&req.url) {
            return self.add_inner(req, opts, None);
        }
        let u = url::Url::parse(&req.url).context("invalid URL")?;
        if !matches!(u.scheme(), "http" | "https") {
            bail!("unsupported scheme {:?} (only http/https)", u.scheme());
        }
        let key = url_key(&u);
        let known = self.0.db.lock().unwrap().with_key(&key).any(|i| same_media(i, req.media.as_ref()));
        // A video page is not a file to check: yt-dlp resolves it when the download runs.
        if known || req.media.is_some() {
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

    /// Check a request and work out where it goes, before the list is locked.
    fn prepare(&self, mut req: Request, probe: Option<Probe>) -> Result<Prepared> {
        let u = url::Url::parse(req.url.trim()).context("invalid URL")?;
        // A magnet link, or a .torrent file's address, is a torrent to download — not the
        // .torrent file itself.
        if req.torrent.is_none() && req.media.is_none() && torrent::is_torrent_url(&req.url) {
            req.torrent = Some(TorrentRequest::default());
        }
        let torrent_ok = req.torrent.is_some() && u.scheme() == "magnet";
        if !matches!(u.scheme(), "http" | "https") && !torrent_ok {
            bail!("unsupported scheme {:?} (only http/https, and magnet links)", u.scheme());
        }
        if let Some(c) = req.checksum.take() {
            if req.media.is_some() || req.torrent.is_some() {
                bail!("a checksum can only be checked for a single file downloaded from a web address, not a video or a torrent");
            }
            req.checksum = Some(c.checked()?);
        }
        if let Some(m) = &req.media {
            if !valid_format(&m.format) {
                bail!("not a format selector: {:?}", m.format);
            }
            if m.audio.as_deref().is_some_and(|a| !media::AUDIO_FORMATS.contains(&a)) {
                bail!("not an audio format reIDM converts to: {:?}", m.audio);
            }
        }
        let torrent_name = req.torrent.is_some().then(|| torrent::magnet_name(&req.url).unwrap_or_else(|| filename_from_url(&u).trim_end_matches(".torrent").to_string()));
        // The caller's name first (the browser's choice), then the server's, then the URL's.
        let hint = req
            .filename
            .as_deref()
            .or(probe.as_ref().and_then(|p| p.filename.as_deref()))
            .or(torrent_name.as_deref().filter(|n| !n.is_empty()))
            .map(sanitize_filename)
            .unwrap_or_else(|| match &req.media {
                // yt-dlp names it after the title once it starts; until then, the video's
                // id (`watch?v=…`) says more than the page's name ("watch").
                Some(_) => sanitize_filename(&u.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.into_owned()).unwrap_or_else(|| filename_from_url(&u))),
                None => sanitize_filename(&filename_from_url(&u)),
            });
        let dir = req.dir.clone().unwrap_or_else(|| {
            let s = self.0.settings.lock().unwrap();
            match (&req.media, Category::of(&hint)) {
                // A video page has no telling extension; it is a video unless it is music.
                (Some(_), Category::General) if s.sort_by_category => s.folder(Category::Video),
                _ => s.folder_for(&hint),
            }
        });
        // Checked here, before the list is locked (see `free_names`).
        let candidates = free_names(&dir, &hint, 8);
        Ok(Prepared { key: url_key(&u), req, dir, hint, probe, candidates })
    }

    /// Put one prepared request into the list, or recognise it as one already there. The caller
    /// holds the list and the live set; `claimed` is every path the list's downloads use.
    fn place(&self, db: &mut Db, live: &BTreeMap<u64, Live>, p: Prepared, opts: AddOptions, claimed: &mut HashSet<PathBuf>, fx: &mut Effects) -> AddOutcome {
        let Prepared { mut req, key, dir, hint, probe, candidates } = p;
        let existing = db.with_key(&key).find(|i| same_media(i, req.media.as_ref())).map(|i| i.id);
        if let Some(id) = existing {
            let running = live.get(&id);
            let it = db.items.get_mut(&id).expect("indexed items are listed");
            // `req.filename` may carry a better name than the one we guessed last time.
            if it.request.filename.is_none() {
                it.request.filename = req.filename.as_deref().map(sanitize_filename);
            }
            // As may a checksum, given the second time round.
            if it.request.checksum.is_none() && req.checksum.is_some() && it.status != Status::Done {
                it.request.checksum = req.checksum.take();
            }
            if running.is_none() {
                // The usual reason a download stopped is an expired session, and clicking the
                // link again is how the user hands over a fresh one. Same address, so these
                // are safe to take.
                let r = &mut it.request;
                for (slot, new) in [(&mut r.cookie, &mut req.cookie), (&mut r.referer, &mut req.referer), (&mut r.user_agent, &mut req.user_agent)] {
                    if new.is_some() {
                        *slot = new.take();
                    }
                }
            }
            if it.status == Status::Done && running.is_none() {
                if it.path.exists() {
                    return AddOutcome { id, outcome: Outcome::AlreadyComplete };
                }
                // Finished, but the file has gone (deleted, moved): the browser would otherwise
                // be told "already downloaded" and give up its copy, and nobody would have it.
                tracing::info!("download {id} was complete, but {} is gone; downloading it again", it.path.display());
                self.reset_item(it, fx);
                if opts.later {
                    it.status = Status::Paused;
                    it.scheduled = true;
                }
                return AddOutcome { id, outcome: Outcome::Existing };
            }
            if !opts.later {
                match running {
                    // Anything that had stopped is picked up again: re-adding the link is how you
                    // carry on an interrupted download. *Download Later* leaves it where it is.
                    None if matches!(it.status, Status::Paused | Status::Failed(_)) => {
                        it.status = Status::Queued;
                        fx.pump = true;
                    }
                    Some(l) if l.cancel.load(Ordering::SeqCst) => l.requeue.store(true, Ordering::SeqCst),
                    _ => {}
                }
            }
            return AddOutcome { id, outcome: Outcome::Existing };
        }

        let id = db.next_id;
        db.next_id += 1;
        // The disk was read before the lock; an unusual list that has claimed all of the names
        // checked falls back to reading it here.
        let path = candidates.into_iter().find(|c| !claimed.contains(c)).unwrap_or_else(|| unique_path(&dir, &hint, |p| claimed.contains(p)));
        claimed.insert(path.clone());
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
            note: None,
            torrent: None,
            part: None,
            replace: false,
            order: db.next_order,
            checksum_ok: None,
            waiting_for_space: false,
        };
        db.next_order += 1;
        if let Some(p) = probe {
            let n = self.0.settings.lock().unwrap().connections as u64;
            item.segments = initial_segments(p.size, p.resumable, n);
            item.size = p.size;
            item.resumable = p.resumable;
            item.etag = p.etag;
            item.last_modified = p.last_modified;
        }
        db.insert(item);
        fx.pump |= !opts.later;
        AddOutcome { id, outcome: Outcome::New }
    }

    fn add_inner(&self, req: Request, opts: AddOptions, probe: Option<Probe>) -> Result<AddOutcome> {
        let p = self.prepare(req, probe)?;
        std::fs::create_dir_all(&p.dir).with_context(|| format!("cannot use {} as the download folder", p.dir.display()))?;
        let mut fx = Effects::default();
        let out = {
            let mut db = self.0.db.lock().unwrap();
            let live = self.0.live.lock().unwrap();
            let mut claimed = Self::claimed_paths(&db);
            self.place(&mut db, &live, p, opts, &mut claimed, &mut fx)
        };
        self.apply(fx);
        Ok(out)
    }

    /// Every path a download in the list will be saved to.
    fn claimed_paths(db: &Db) -> HashSet<PathBuf> {
        db.items.values().map(|i| i.path.clone()).collect()
    }

    /// Queue many requests, skipping duplicates, and report per-URL what happened.
    pub fn add_many(&self, reqs: Vec<Request>) -> Vec<(String, Result<AddOutcome>)> {
        self.add_many_with(reqs, AddOptions::default())
    }

    /// Like adding each in turn, but the list is locked and saved once for the lot: ten
    /// thousand links take a moment, not the minutes that saving the list after each took.
    pub fn add_many_with(&self, reqs: Vec<Request>, opts: AddOptions) -> Vec<(String, Result<AddOutcome>)> {
        let mut prepared: Vec<(String, Result<Prepared>)> = reqs.into_iter().map(|r| (r.url.clone(), self.prepare(r, None))).collect();
        // Each folder once, not once per link.
        let mut dirs: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
        for p in prepared.iter().filter_map(|(_, p)| p.as_ref().ok()) {
            dirs.entry(p.dir.clone()).or_insert_with(|| std::fs::create_dir_all(&p.dir).err().map(|e| format!("cannot use {} as the download folder: {e}", p.dir.display())));
        }
        for (_, p) in &mut prepared {
            if let Some(Some(e)) = p.as_ref().ok().and_then(|p| dirs.get(&p.dir)) {
                *p = Err(anyhow!("{e}"));
            }
        }
        let mut fx = Effects::default();
        let out = {
            let mut db = self.0.db.lock().unwrap();
            let live = self.0.live.lock().unwrap();
            let mut claimed = Self::claimed_paths(&db);
            prepared.into_iter().map(|(url, p)| (url, p.map(|p| self.place(&mut db, &live, p, opts, &mut claimed, &mut fx)))).collect()
        };
        self.apply(fx);
        out
    }

    /// Do what changing the list called for, once the list is no longer locked.
    fn apply(&self, fx: Effects) {
        for f in &fx.delete {
            let _ = std::fs::remove_file(f);
        }
        for p in &fx.leftovers_of {
            // Scanned here, not while the list was locked: a slow or synced folder must not
            // hold up everything waiting for the list.
            for f in media::leftovers(p) {
                let _ = std::fs::remove_file(f);
            }
        }
        for d in &fx.delete_dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        for hash in fx.forget {
            // From nothing: the torrent engine forgets it, and its files go.
            self.forget_torrent(hash, true);
        }
        self.save_now();
        if fx.pump {
            self.pump();
        }
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
            if free == 0 {
                return;
            }
            // A queued item that is not live is exactly one that is waiting for a slot; the
            // first in the queue's order goes first.
            let mut waiting: Vec<(u64, &Item)> = db.items.values().filter(|it| it.status == Status::Queued && !live.contains_key(&it.id)).map(|it| (it.order, &**it)).collect();
            waiting.sort_unstable_by_key(|(order, it)| (*order, it.id));
            for (_, it) in waiting.into_iter().take(free) {
                let l = Live::new(it.downloaded, it.speed_limit, &self.0.rev);
                live.insert(it.id, l.clone());
                to_start.push((it.id, l));
            }
        }
        for (id, live) in to_start {
            self.launch(id, live);
        }
    }

    pub fn resume(&self, id: u64) -> Result<()> {
        {
            let mut db = self.0.db.lock().unwrap();
            let it = db.items.get(&id).ok_or_else(|| anyhow!("no such download {id}"))?;
            if it.status == Status::Done {
                bail!("already complete");
            }
            if let Some(l) = self.0.live.lock().unwrap().get(&id) {
                // Still stopping from a Pause a moment ago: start it again once it has stopped.
                if l.cancel.load(Ordering::SeqCst) {
                    l.requeue.store(true, Ordering::SeqCst);
                }
                return Ok(());
            }
            if let Some(it) = db.items.get_mut(&id) {
                it.status = Status::Queued;
                // It looks at the space again when it starts.
                it.waiting_for_space = false;
            }
        }
        self.save_now();
        // Through the scheduler, so resuming twenty paused rows does not start twenty transfers.
        self.pump();
        Ok(())
    }

    /// Re-queue a finished download from its original URL, replacing what is on disk.
    pub fn redownload(&self, id: u64) -> Result<()> {
        let mut fx = Effects::default();
        {
            let mut db = self.0.db.lock().unwrap();
            let it = db.items.get_mut(&id).ok_or_else(|| anyhow!("no such download {id}"))?;
            self.reset_item(it, &mut fx);
        }
        self.apply(fx);
        Ok(())
    }

    /// Make `it` a download that has not begun, and say what of its old one to delete.
    fn reset_item(&self, it: &mut Item, fx: &mut Effects) {
        fx.delete.push(self.part_for(it));
        // The file there now is this download's own (finished, or failed its checksum), so the
        // new one may take its place.
        it.replace = it.status == Status::Done || it.checksum_ok == Some(false);
        it.checksum_ok = None;
        it.waiting_for_space = false;
        it.segments.clear();
        it.downloaded = 0;
        it.size = None;
        it.resumable = false;
        it.etag = None;
        it.last_modified = None;
        it.finished_at = None;
        it.part = None;
        it.status = Status::Queued;
        if let Some(t) = it.torrent.take() {
            fx.forget.push(t.info_hash);
        }
        if it.request.media.is_some() {
            // yt-dlp would otherwise find the finished file and its pieces and stop there.
            fx.delete.push(it.path.clone());
            fx.leftovers_of.push(it.path.clone());
            fx.delete_dirs.push(media::work_dir(&self.temp_dir(), it.id));
        }
        fx.pump = true;
    }

    /// Stop a download if it is running, and start it again from the first byte.
    pub async fn restart(&self, id: u64) -> Result<()> {
        if !self.0.db.lock().unwrap().items.contains_key(&id) {
            bail!("no such download {id}");
        }
        if !self.stop_and_wait(id).await {
            bail!("download {id} is still stopping; try again in a moment");
        }
        self.redownload(id)
    }

    /// Stop a download: a running one stops its connections; a queued one stays where it is
    /// instead of starting when a slot frees up.
    pub fn pause(&self, id: u64) {
        let paused = {
            let mut db = self.0.db.lock().unwrap();
            let live = self.0.live.lock().unwrap();
            match live.get(&id) {
                Some(l) => {
                    // Pause after a Resume that was waiting for the stop: the Pause wins.
                    l.requeue.store(false, Ordering::SeqCst);
                    l.cancel.set();
                    false
                }
                None if db.items.get(&id).is_some_and(|i| i.status == Status::Queued) => {
                    if let Some(it) = db.items.get_mut(&id) {
                        it.status = Status::Paused;
                    }
                    true
                }
                // Waiting for space: paused by the user, it waits for the user instead.
                None if db.items.get(&id).is_some_and(|i| i.waiting_for_space) => {
                    if let Some(it) = db.items.get_mut(&id) {
                        it.waiting_for_space = false;
                        it.note = None;
                    }
                    true
                }
                None => false,
            }
        };
        if paused {
            self.save_now();
        }
    }

    /// Pause, and make sure nothing will start it again behind our back, before changing what a
    /// download *is*. True once its connections have stopped; false if they have not in time.
    async fn stop_and_wait(&self, id: u64) -> bool {
        self.pause(id);
        let deadline = Instant::now() + STOP_LIMIT;
        while self.0.live.lock().unwrap().contains_key(&id) {
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        true
    }

    /// Take a download off the list; `delete_files` deletes what it downloaded too. Its
    /// unfinished data always goes. A running download's files are deleted once its
    /// connections have stopped writing to them, not while they still are.
    pub fn remove(&self, id: u64, delete_files: bool) {
        let now = {
            let mut db = self.0.db.lock().unwrap();
            let live = self.0.live.lock().unwrap();
            let Some(it) = db.remove(id) else { return };
            match live.get(&id) {
                Some(l) => {
                    *l.removed.lock().unwrap() = Some((it, delete_files));
                    l.cancel.set();
                    None
                }
                None => Some(it),
            }
        };
        if let Some(it) = now {
            self.purge_files(&it, delete_files);
        }
        self.save_now();
        self.pump();
    }

    /// Delete a removed download's files.
    fn purge_files(&self, it: &Item, delete_files: bool) {
        // The part file is reIDM's own scratch space, so it always goes; leaving it behind
        // would make every later download of the same name pick "name (1).zip".
        let _ = std::fs::remove_file(self.part_for(it));
        if it.request.media.is_some() {
            for f in media::leftovers(&it.path) {
                let _ = std::fs::remove_file(f);
            }
            let _ = std::fs::remove_dir_all(media::work_dir(&self.temp_dir(), it.id));
        }
        if let Some(t) = &it.torrent {
            // The torrent engine owns a torrent's files (one, or a folder of them).
            self.forget_torrent(t.info_hash.clone(), delete_files);
            return;
        }
        if delete_files {
            // "remove and delete the file" must also work for finished downloads, which are
            // the ones people actually delete.
            let _ = std::fs::remove_file(&it.path);
        }
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
            // Found first, then taken for changing one by one: taking every item would count
            // all of them as changed.
            let ids: Vec<u64> = db.items.values().filter(|it| it.scheduled && !live.contains_key(&it.id) && matches!(it.status, Status::Paused | Status::Failed(_))).map(|it| it.id).collect();
            for id in &ids {
                if let Some(it) = db.items.get_mut(id) {
                    it.status = Status::Queued;
                    it.waiting_for_space = false;
                }
            }
            ids.len()
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
            let (running, queued): (Vec<u64>, Vec<u64>) = db.items.values().filter(|i| i.scheduled && (live.contains_key(&i.id) || i.status == Status::Queued)).map(|i| i.id).partition(|id| live.contains_key(id));
            for id in queued {
                if let Some(it) = db.items.get_mut(&id) {
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

    /// Run the scheduler for as long as the process lives — and the look for room for
    /// downloads waiting for space. Call once, from the daemon.
    pub fn spawn_scheduler(&self) {
        self.spawn_space_watch();
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
                prev = scheduler_mark(prev, now);
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

    /// Whether what `p` found at a new address can carry on `item`: the same size, and byte
    /// ranges if bytes have already been downloaded.
    fn same_file(item: &Item, p: &Probe) -> Result<()> {
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
        Ok(())
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
            if it.request.media.is_some() {
                bail!("a video download finds its own address each time it runs; there is none to refresh");
            }
            if it.status == Status::Done {
                bail!("that download is already complete");
            }
            let other = db.with_key(&key).find(|i| i.id != id).map(|i| i.id);
            if let Some(other) = other {
                bail!("that address is already in the list as download #{other}");
            }
        }

        // Checked while the download carries on: a link that turns out to be some other file
        // leaves it exactly as it was.
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
        Self::same_file(&item, &p)?;

        // Its connections are still using the old address.
        let was_active = item.status == Status::Queued || self.0.live.lock().unwrap().contains_key(&id);
        if !self.stop_and_wait(id).await {
            bail!("download {id} is still stopping; try again in a moment");
        }
        // Again, as it stopped: it may have got its first bytes while the new address was checked.
        let item = self.get(id).ok_or_else(|| anyhow!("download {id} was removed"))?;
        if let Err(e) = Self::same_file(&item, &p) {
            if was_active {
                let _ = self.resume(id);
            }
            return Err(e);
        }
        let progressed = item.segments.iter().any(|s| s.done > 0 || s.finished);

        let connections = self.0.settings.lock().unwrap().connections as u64;
        {
            let mut db = self.0.db.lock().unwrap();
            let it = db.items.get_mut(&id).ok_or_else(|| anyhow!("download {id} was removed"))?;
            let new_url = std::mem::replace(&mut req.url, it.request.url.clone());
            it.request = req;
            // The validators of the new server, not the old one's: a mirror has its own ETags.
            it.etag = p.etag;
            it.last_modified = p.last_modified;
            if it.segments.is_empty() {
                // Never probed: let the next run do it from scratch with the new address.
                it.size = None;
            } else if !progressed {
                // Nothing downloaded yet, so the ranges can follow the new server: one that
                // doesn't serve ranges gets one connection, rather than five failures of "server
                // stopped honouring Range".
                it.resumable = p.resumable;
                it.size = p.size;
                it.segments = initial_segments(p.size, p.resumable, connections);
            }
            it.status = Status::Queued;
            db.set_url(id, new_url);
        }
        self.save_now();
        self.pump();
        Ok(())
    }

    /// Run a download whose slot [`Manager::pump`] has already reserved in `live`.
    fn launch(&self, id: u64, live: Live) {
        let me = self.clone();
        tokio::spawn(async move {
            me.set_status(id, Status::Running);
            // Saved now, not at the first progress: a video download can spend a while fetching
            // yt-dlp first, and `reidm list` reads the saved list.
            me.save();
            let res = me.run(id, live.clone()).await;
            let st = match res {
                Ok(true) => Status::Done,
                Ok(false) => Status::Paused,
                Err(e) => {
                    tracing::warn!("download {id} failed: {e:#}");
                    Status::Failed(format!("{e:#}"))
                }
            };
            let removed = {
                // Under both locks, so a Resume or Remove arriving now either sees the download
                // still live (and leaves its request in `live`) or sees it stopped.
                let mut db = me.0.db.lock().unwrap();
                let mut lv = me.0.live.lock().unwrap();
                lv.remove(&id);
                if let Some(it) = db.items.get_mut(&id) {
                    // An HTTP download's progress is what its last checkpoint made durable;
                    // videos and torrents keep their own count.
                    if it.request.media.is_some() || it.request.torrent.is_some() {
                        it.downloaded = live.progress.load(Ordering::Relaxed);
                    }
                    it.note = None;
                    if st == Status::Done {
                        it.finished_at = Some(now_secs());
                        // A finished download leaves the scheduled queue, as in IDM.
                        it.scheduled = false;
                        it.replace = false;
                    }
                    let no_space = live.no_space.lock().unwrap().take();
                    it.status = if st == Status::Paused && live.requeue.load(Ordering::SeqCst) {
                        Status::Queued
                    } else {
                        // Stopped by the disk filling up, not by the user: it says why, and
                        // starts again by itself once there is room.
                        if let (Status::Paused, Some(why)) = (&st, no_space) {
                            tracing::info!("download {id}: {why}");
                            it.note = Some(why);
                            it.waiting_for_space = true;
                        }
                        st
                    };
                }
                live.removed.lock().unwrap().take()
            };
            if let Some((it, delete_files)) = removed {
                let me = me.clone();
                let _ = tokio::task::spawn_blocking(move || me.purge_files(&it, delete_files)).await;
            }
            me.save_now();
            me.pump();
        });
    }

    fn set_status(&self, id: u64, st: Status) {
        let mut db = self.0.db.lock().unwrap();
        if let Some(it) = db.items.get_mut(&id) {
            if st == Status::Running {
                it.last_try = Some(now_secs());
                it.waiting_for_space = false;
            }
            it.note = None;
            it.status = st;
        }
    }

    /// Set (or clear) what a running download is busy with; see [`Item::note`].
    fn set_note(&self, id: u64, note: Option<&str>) {
        // Looked at before it is taken for writing: taking it counts as a change, and a torrent
        // calls this twice a second.
        let changed = {
            let mut db = self.0.db.lock().unwrap();
            match db.items.get(&id) {
                Some(it) if it.note.as_deref() != note => {
                    if let Some(it) = db.items.get_mut(&id) {
                        it.note = note.map(str::to_string);
                    }
                    true
                }
                _ => false,
            }
        };
        // Saved, since `reidm list` reads the saved list; notes change a few times a download.
        if changed {
            self.save();
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
        let answer = self.0.settings.lock().unwrap().connect_timeout();
        let r = match tokio::time::timeout(answer, self.http().get(url).headers(headers.clone()).header(header::RANGE, "bytes=0-0").send()).await {
            Ok(r) => r?,
            Err(_) => bail!("the server did not answer within {}s", answer.as_secs()),
        };
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
            text(header::CONTENT_RANGE).as_deref().and_then(parse_content_range).and_then(|(_, _, total)| total)
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
        if let Some(why) = self.short_of_space(&item).await {
            live.stop_for_space(why);
            return Ok(false);
        }
        if item.request.media.is_some() {
            return self.run_media(id, live, item).await;
        }
        if item.request.torrent.is_some() {
            return self.run_torrent(id, live, item).await;
        }
        // Downloaded, but it failed its checksum: resuming it checks the file again (the
        // expected value may have been the wrong one) rather than fetching it a second time.
        if item.checksum_ok == Some(false) && item.path.is_file() && !item.segments.is_empty() && item.segments.iter().all(Segment::complete) {
            self.verify_checksum(id).await?;
            return Ok(true);
        }
        let headers = Self::headers(&item.request);

        if item.segments.is_empty() {
            let p = tokio::select! {
                p = self.probe(&item.request.url, &headers) => p?,
                _ = live.cancel.cancelled() => return Ok(false),
            };
            // The server's name can say a different type than the address did: file it in that
            // type's folder, unless a folder was asked for.
            let server_name = p.filename.as_deref().filter(|_| item.request.filename.is_none()).map(sanitize_filename);
            let dir = match (&server_name, &item.request.dir) {
                (None, _) => None,
                (Some(_), Some(d)) => Some(d.clone()),
                (Some(n), None) => Some(self.0.settings.lock().unwrap().folder_for(n)),
            };
            if let Some(d) = &dir {
                std::fs::create_dir_all(d)?;
            }
            // Read the folder before the lock (see `free_names`): the server's name may land in
            // a synced or slow folder, and the window's polls and the other downloads need the
            // list while this runs.
            let candidates = match (&dir, &server_name) {
                (Some(d), Some(name)) => free_names(d, name, 8),
                _ => Vec::new(),
            };
            let n = self.0.settings.lock().unwrap().connections as u64;
            // The name is chosen and recorded under one lock: two downloads whose servers give
            // the same name must not both take it.
            {
                let mut db = self.0.db.lock().unwrap();
                let claimed: HashSet<PathBuf> = db.items.iter().filter(|(k, _)| **k != id).map(|(_, i)| i.path.clone()).collect();
                // Update in place rather than inserting: the download may have been removed while
                // the probe was in flight, and re-inserting would resurrect it.
                let Some(slot) = db.items.get_mut(&id) else { return Ok(false) };
                if let (Some(d), Some(name)) = (&dir, &server_name) {
                    slot.path = candidates.into_iter().find(|c| !claimed.contains(c)).unwrap_or_else(|| unique_path(d, name, |p| claimed.contains(p)));
                }
                slot.size = p.size;
                slot.resumable = p.resumable;
                slot.etag = p.etag;
                slot.last_modified = p.last_modified;
                slot.segments = initial_segments(p.size, p.resumable, n);
                item = (**slot).clone();
            }
            // Now that its size is known.
            if let Some(why) = self.short_of_space(&item).await {
                live.stop_for_space(why);
                return Ok(false);
            }
        }

        let part = self.part_for(&item);
        if item.part.as_ref() != Some(&part) {
            item.part = Some(part.clone());
            if let Some(slot) = self.0.db.lock().unwrap().items.get_mut(&id) {
                slot.part = Some(part.clone());
            }
        }
        let file = {
            let (path, size, resumable) = (part.clone(), item.size, item.resumable);
            let progressed = item.segments.iter().any(|s| s.done > 0 || s.finished);
            let need = item.segments.iter().map(|s| s.start + s.done).max().unwrap_or(0);
            let (file, lost) = tokio::task::spawn_blocking(move || -> std::io::Result<(std::fs::File, bool)> {
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d)?;
                }
                let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path)?;
                // Progress is recorded only once it is on disk (see `checkpoint`), so the list
                // is never ahead of a part file that is still there. This catches the one
                // that is not: deleted, or truncated, between runs.
                let lost = progressed && (!resumable || f.metadata()?.len() < need);
                if let Some(sz) = size {
                    disk::preallocate(&f, sz)?;
                }
                Ok((f, lost))
            })
            .await
            .map_err(std::io::Error::other)?
            .map_err(|e| write_error(e, &part))?;
            if lost {
                tracing::warn!("saved progress for download {id} is not on disk; restarting it");
                for s in &mut item.segments {
                    s.done = 0;
                    s.finished = false;
                }
                if let Some(slot) = self.0.db.lock().unwrap().items.get_mut(&id) {
                    slot.segments = item.segments.clone();
                    slot.downloaded = 0;
                }
            }
            Arc::new(file)
        };
        live.progress.store(item.segments.iter().map(|s| s.done).sum(), Ordering::Relaxed);

        let splittable = item.resumable && item.size.is_some_and(|s| s >= MIN_SPLIT);
        let incomplete = item.segments.iter().filter(|s| !s.complete()).count();
        let opts = self.settings();
        let connections = opts.connections.max(1);
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
            file: file.clone(),
            resumable: item.resumable,
            size: item.size,
            validator: if item.resumable { item.etag.clone() } else { None },
            last_modified: item.last_modified.clone(),
            splittable,
            live: live.clone(),
            opts,
        });
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..workers {
            let (me, ctx) = (self.clone(), ctx.clone());
            tasks.spawn(async move { me.worker(ctx).await });
        }
        // periodic checkpoint so a crash loses little
        let ticker = {
            let (me, live, file) = (self.clone(), live.clone(), file.clone());
            tokio::spawn(async move {
                let mut saved = None;
                loop {
                    tokio::time::sleep(CHECKPOINT_EVERY).await;
                    if let Err(e) = me.checkpoint(id, &live, &file, &mut saved).await {
                        tracing::warn!("download {id}: progress not recorded this time: {e:#}");
                    }
                    // The disk may be filling up with something else meanwhile.
                    if let Some(it) = me.get(id) {
                        if let Some(why) = me.short_of_space(&it).await {
                            live.stop_for_space(why);
                        }
                    }
                }
            })
        };
        let mut first_err = None;
        let mut top_up = tokio::time::interval_at(tokio::time::Instant::now() + TOP_UP_EVERY, TOP_UP_EVERY);
        loop {
            tokio::select! {
                r = tasks.join_next() => match r {
                    None => break,
                    Some(r) => {
                        if let Err(e) = r.map_err(|e| anyhow!(e)).and_then(|r| r) {
                            first_err.get_or_insert(e);
                            live.cancel.set(); // stop siblings
                        }
                    }
                },
                _ = top_up.tick(), if splittable => self.top_up(&ctx, connections, &mut tasks),
            }
        }
        ticker.abort();
        let _ = ticker.await;
        // Everything written is made durable and recorded, however the run ended.
        let recorded = self.checkpoint(id, &live, &file, &mut None).await;
        if let Some(e) = first_err {
            return Err(e);
        }
        if live.cancel.load(Ordering::SeqCst) {
            if let Err(e) = recorded {
                tracing::warn!("download {id}: the last progress could not be recorded: {e:#}");
            }
            return Ok(false);
        }
        // A finished file reported as done must be on disk, not only in memory.
        recorded?;
        if !live.plan.lock().unwrap().segs.iter().all(Segment::complete) {
            // Every connection stopped without finishing and without reporting why. It should
            // not happen; if it does, say so rather than calling it "paused".
            bail!("the transfer stopped before the file was complete; resume it to try again");
        }
        // The handle is closed before the move: Windows refuses to rename some open files.
        drop(ctx);
        drop(file);
        self.finalize(id, &part, &live).await?;
        self.verify_checksum(id).await?;
        Ok(true)
    }

    /// Bring a download that lost connections back up to `target`: a link that dropped out
    /// for a while should not leave the rest of a long download on one connection.
    fn top_up(&self, ctx: &Arc<RunCtx>, target: usize, tasks: &mut tokio::task::JoinSet<Result<()>>) {
        if ctx.live.cancel.load(Ordering::SeqCst) {
            return;
        }
        {
            let mut p = ctx.live.plan.lock().unwrap();
            // None left means the run is ending; there is nothing to join.
            if p.workers == 0 || p.workers >= target || p.segs.iter().all(Segment::complete) {
                return;
            }
            p.workers += 1;
        }
        tracing::debug!("download {}: adding a connection back", ctx.id);
        let (me, ctx) = (self.clone(), ctx.clone());
        tasks.spawn(async move { me.worker(ctx).await });
    }

    /// Record a running download's progress in the list — only progress that is on disk.
    ///
    /// The ranges are read first, then the part file is flushed, then the ranges are recorded:
    /// every byte they count was written before the flush, so a crash, a power cut or a full
    /// disk can never leave the list counting bytes the file does not have. `saved` remembers
    /// what was last recorded, so an idle download does not flush for nothing.
    async fn checkpoint(&self, id: u64, live: &Live, file: &Arc<std::fs::File>, saved: &mut Option<(u64, usize, usize)>) -> Result<()> {
        let mut segs = live.plan.lock().unwrap().segs.clone();
        let mark = (segs.iter().map(|s| s.done).sum::<u64>(), segs.iter().filter(|s| s.finished).count(), segs.len());
        if *saved == Some(mark) {
            return Ok(());
        }
        let f = file.clone();
        tokio::task::spawn_blocking(move || f.sync_data()).await.map_err(std::io::Error::other)?.context("cannot flush the download to the disk")?;
        for s in &mut segs {
            s.active = false;
            s.claimed = 0;
        }
        if let Some(it) = self.0.db.lock().unwrap().items.get_mut(&id) {
            it.segments = segs;
            it.downloaded = mark.0;
        }
        *saved = Some(mark);
        Ok(())
    }

    /// Move a finished part file to its download's path. If something has appeared at that path
    /// since it was chosen — the browser's own copy, a file the user saved — that file is kept
    /// and the download takes the next free name.
    async fn finalize(&self, id: u64, part: &Path, live: &Live) -> Result<()> {
        let (mut dest, replace, url, referer) = {
            let db = self.0.db.lock().unwrap();
            let it = db.items.get(&id).map(|i| &**i);
            let removed = live.removed.lock().unwrap();
            let it = it.or(removed.as_ref().map(|(i, _)| i)).ok_or_else(|| anyhow!("vanished"))?;
            (it.path.clone(), it.replace, it.request.url.clone(), it.request.referer.clone())
        };
        let mut src = part.to_path_buf();
        let placed = loop {
            if let Some(d) = dest.parent() {
                tokio::fs::create_dir_all(d).await?;
            }
            let (from, to) = (src.clone(), dest.clone());
            let r = tokio::task::spawn_blocking(move || if replace { std::fs::rename(&from, &to) } else { disk::rename_noreplace(&from, &to) })
                .await
                .map_err(std::io::Error::other)?;
            match r {
                Ok(()) => break dest,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let next = self.rename_target(id, &dest);
                    tracing::info!("download {id}: {} already exists, so it is saved as {}", dest.display(), next.display());
                    dest = next;
                }
                Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices && src == part => {
                    // The temporary folder is on another drive: copy into the destination's
                    // folder under a temporary name, then rename that into place as above. A
                    // failed copy leaves no half-file under the real name, and the part file
                    // stays until the move has finished. A whole-file copy can take a while on
                    // a slow drive, so say what the 100% download is doing.
                    self.set_note(id, Some("moving the finished file to the download folder"));
                    let staging = dest.with_file_name(format!(".{id}.reidm-move"));
                    let (from, to) = (src.clone(), staging.clone());
                    let copied = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                        std::fs::copy(&from, &to)?;
                        std::fs::File::open(&to)?.sync_all()
                    })
                    .await
                    .map_err(std::io::Error::other)?;
                    if let Err(e) = copied {
                        let _ = tokio::fs::remove_file(&staging).await;
                        return Err(write_error(e, &dest));
                    }
                    src = staging;
                }
                Err(e) => return Err(anyhow::Error::new(e).context(format!("cannot move the finished file to {}", dest.display()))),
            }
        };
        if src != part {
            let _ = tokio::fs::remove_file(part).await;
        }
        if let Some((_, true)) = &*live.removed.lock().unwrap() {
            // Removed with its files while the move was under way.
            let _ = std::fs::remove_file(&placed);
            return Ok(());
        }
        disk::mark_from_internet(&placed, &url, referer.as_deref());
        Ok(())
    }

    /// The next free name beside `taken` for download `id`, recorded as its path.
    fn rename_target(&self, id: u64, taken: &Path) -> PathBuf {
        let mut db = self.0.db.lock().unwrap();
        let claimed: HashSet<PathBuf> = db.items.iter().filter(|(k, _)| **k != id).map(|(_, i)| i.path.clone()).collect();
        let name = taken.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "download".into());
        let next = unique_path(taken.parent().unwrap_or(Path::new(".")), &name, |p| claimed.contains(p));
        if let Some(it) = db.items.get_mut(&id) {
            it.path = next.clone();
        }
        next
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
            ctx.live.progress.touch();
            let r = self.fetch_segment(idx, &ctx).await;
            let mut p = plan.lock().unwrap();
            p.segs[idx].active = false;
            p.segs[idx].claimed = 0;
            ctx.live.progress.touch();
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
            let (from, end, before) = {
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
                    ctx.live.progress.sub(done);
                }
                (s.start + s.done, s.end, s.done)
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
                    // A connection that got somewhere before it broke is a working connection on
                    // a flaky line (or a server that caps how much one response may carry): carry
                    // on from where it got to, at once, with its retries intact. Counting those
                    // breaks against it made long downloads lose their connections one by one.
                    let advanced = ctx.live.plan.lock().unwrap().segs[idx].done > before;
                    if advanced && ctx.resumable {
                        attempt = 0;
                        tracing::debug!("download {} range {idx} broke off ({e:#}); continuing", ctx.id);
                        continue;
                    }
                    // A server refusing an extra connection will keep refusing it; when others
                    // are working, give the range back at once rather than after a minute of
                    // retries.
                    if is_busy(&e) && ctx.live.plan.lock().unwrap().workers > 1 {
                        return Err(e);
                    }
                    attempt += 1;
                    if ctx.opts.out_of_retries(attempt) {
                        return Err(e);
                    }
                    tracing::debug!("download {} range {idx} retry {attempt}: {e:#}", ctx.id);
                    nap(ctx.opts.retry_wait(attempt), &ctx.live.cancel).await;
                }
            }
        }
    }

    /// One request for range `idx`, starting at `from`. Writes until the range is complete (its
    /// end may move closer while this runs, when another connection takes the far half), the
    /// server stops sending, or the download is cancelled.
    async fn transfer(&self, idx: usize, ctx: &RunCtx, from: u64, end: u64) -> Result<()> {
        let open_ended = end == u64::MAX;
        // Taken for each request, so a new proxy applies from the next connection on.
        let mut rb = self.http().get(&ctx.url).headers(ctx.headers.clone());
        if ctx.resumable {
            rb = rb.header(header::RANGE, format!("bytes={from}-{end}"));
            if let Some(v) = &ctx.validator {
                rb = rb.header(header::IF_RANGE, v.as_str());
            }
        }
        let resp = tokio::select! {
            r = tokio::time::timeout(ctx.opts.connect_timeout(), rb.send()) => match r {
                Ok(r) => r?,
                Err(_) => bail!("the server did not answer within {}s", ctx.opts.connect_timeout_secs),
            },
            _ = ctx.live.cancel.cancelled() => return Ok(()),
        };
        let resp = if ctx.resumable {
            // With `If-Range`, a full `200` instead of `206` is the server saying the file is no
            // longer the one the validator describes.
            if resp.status() == reqwest::StatusCode::OK && ctx.validator.is_some() {
                return Err(ChangedOnServer("the server no longer recognises the version being resumed".into()).into());
            }
            // The bytes being resumed are past the end of the file: it got shorter.
            if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                return Err(ChangedOnServer("it no longer has the bytes being resumed".into()).into());
            }
            let resp = resp.error_for_status()?;
            if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                bail!("server stopped honouring Range (HTTP {})", resp.status());
            }
            let text = |k: header::HeaderName| resp.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
            // Writing bytes that start somewhere else at `from` would corrupt the file, so a
            // part that doesn't say which part it is can't be used.
            let Some((start, last, total)) = text(header::CONTENT_RANGE).as_deref().and_then(parse_content_range) else {
                bail!("the server sent part of the file without saying which part (no Content-Range)");
            };
            if let (Some(total), Some(size)) = (total, ctx.size) {
                if total != size {
                    return Err(ChangedOnServer(format!("it is now {total} bytes, it was {size}")).into());
                }
            }
            if start != from || last < start {
                bail!("the server sent bytes {start}-{last} when asked for {from}-{end}");
            }
            // Without an ETag to send as `If-Range`, the date is what shows a new version.
            if ctx.validator.is_none() {
                if let (Some(was), Some(now)) = (&ctx.last_modified, text(header::LAST_MODIFIED)) {
                    if *was != now {
                        return Err(ChangedOnServer(format!("it was modified {now}; the download began with the version of {was}")).into());
                    }
                }
            }
            resp
        } else {
            let resp = resp.error_for_status()?;
            // Without ranges every attempt starts again from byte 0, so a different length is a
            // different file, not a partial answer.
            if let (Some(size), Some(len)) = (ctx.size, resp.content_length()) {
                if len != size {
                    return Err(ChangedOnServer(format!("it is now {len} bytes, it was {size}")).into());
                }
            }
            resp
        };

        enum End {
            /// The server sent everything it was going to.
            Body,
            /// The range is complete; the rest of this response belongs to someone else.
            Range,
            Cancelled,
        }
        let mut stream = resp.bytes_stream();
        let cancel = &ctx.live.cancel;
        let mut buf: Vec<u8> = Vec::new();
        let mut held_since = Instant::now();
        // Gives up on a server that accepted the request and then went silent, instead of
        // holding the slot forever. Reset by every read, not recreated.
        let idle_limit = ctx.opts.idle_timeout();
        let write_batch = ctx.opts.write_buffer_kib << 10;
        let idle = tokio::time::sleep(idle_limit);
        tokio::pin!(idle);
        let outcome: Result<End> = loop {
            let chunk = tokio::select! {
                biased;
                _ = cancel.cancelled() => break Ok(End::Cancelled),
                _ = &mut idle => break Err(anyhow!("no data received for {}s", idle_limit.as_secs())),
                c = stream.next() => match c {
                    None => break Ok(End::Body),
                    Some(Err(e)) => break Err(e.into()),
                    Some(Ok(c)) => c,
                },
            };
            idle.as_mut().reset(tokio::time::Instant::now() + idle_limit);
            let want = chunk.len() as u64;
            // Pace first, then claim: the claim should cover a write, not a wait.
            ctx.live.limiter.take(want, cancel.cancelled()).await;
            self.0.limiter.take(want, cancel.cancelled()).await;
            // How much of this chunk is still ours: another connection may have taken the far
            // end of the range since the request went out. What is taken is claimed until it
            // is written, so a split can't hand it to someone else meanwhile.
            let take = {
                let mut p = ctx.live.plan.lock().unwrap();
                let s = &mut p.segs[idx];
                let pos = s.start + s.done + s.claimed;
                let room = if s.end == u64::MAX { u64::MAX } else { (s.end + 1).saturating_sub(pos) };
                let t = room.min(want);
                s.claimed += t;
                t as usize
            };
            if buf.is_empty() {
                held_since = Instant::now();
            }
            buf.extend_from_slice(&chunk[..take]);
            if (take as u64) < want {
                break Ok(End::Range);
            }
            if buf.len() >= write_batch || held_since.elapsed() >= WRITE_AT_LEAST_EVERY {
                if let Err(e) = self.write_out(idx, ctx, &mut buf).await {
                    break Err(e);
                }
            }
        };
        // What arrived is good data however the response ended: keep it.
        self.write_out(idx, ctx, &mut buf).await?;
        let end_reason = outcome?;
        if matches!(end_reason, End::Cancelled) {
            return Ok(());
        }
        let mut p = ctx.live.plan.lock().unwrap();
        let s = &mut p.segs[idx];
        if open_ended {
            if matches!(end_reason, End::Body) {
                s.finished = true;
            }
        } else if !s.complete() {
            bail!("connection closed early ({} bytes missing)", s.end + 1 - (s.start + s.done));
        }
        Ok(())
    }

    /// Write what a connection has collected to its place in the part file, and count it as
    /// downloaded only once the write has succeeded. A failed write counts nothing, and lets
    /// go of the claim so the bytes are fetched again.
    async fn write_out(&self, idx: usize, ctx: &RunCtx, buf: &mut Vec<u8>) -> Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        let off = {
            let p = ctx.live.plan.lock().unwrap();
            p.segs[idx].start + p.segs[idx].done
        };
        let data = std::mem::take(buf);
        let file = ctx.file.clone();
        #[cfg(test)]
        let faults = self.0.faults.clone();
        let (data, res) = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            if let Some(r) = faults.write(&file, &data, off) {
                return (data, r);
            }
            let r = disk::write_at(&file, &data, off);
            (data, r)
        })
        .await
        .map_err(std::io::Error::other)?;
        let n = data.len() as u64;
        {
            let mut p = ctx.live.plan.lock().unwrap();
            let s = &mut p.segs[idx];
            if res.is_ok() {
                s.done += n;
                s.claimed = s.claimed.saturating_sub(n);
            } else {
                s.claimed = 0;
            }
        }
        res.map_err(|e| write_error(e, &ctx.part))?;
        ctx.live.progress.add(n);
        // The allocation is kept for the next batch.
        *buf = data;
        buf.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    /// Make the part file's writes fail the way a full disk does, to prove what a failed write
    /// leaves behind.
    #[derive(Default)]
    pub(super) struct Faults {
        /// Once this many bytes have been written (0: never), writes fail. What still fits of
        /// the write that crosses the line is written, as a real short write would be.
        fail_after: AtomicU64,
        written: AtomicU64,
        /// The free space every drive reports, plus one (0: the real figure).
        free_space: AtomicU64,
    }

    impl Faults {
        pub(super) fn free_space(&self) -> Option<u64> {
            self.free_space.load(Ordering::SeqCst).checked_sub(1)
        }

        pub(super) fn write(&self, f: &std::fs::File, data: &[u8], off: u64) -> Option<std::io::Result<()>> {
            let limit = self.fail_after.load(Ordering::SeqCst);
            if limit == 0 {
                return None;
            }
            let before = self.written.fetch_add(data.len() as u64, Ordering::SeqCst);
            if before + data.len() as u64 <= limit {
                return None;
            }
            let fits = limit.saturating_sub(before).min(data.len() as u64) as usize;
            let _ = disk::write_at(f, &data[..fits], off);
            Some(Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "no space left on device (simulated)")))
        }
    }

    /// A server of one file that serves ranges, a little slowly so several connections overlap.
    fn serve(data: Vec<u8>) -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let data = Arc::new(data);
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { break };
                let data = data.clone();
                std::thread::spawn(move || {
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    loop {
                        let mut range = None;
                        let mut line = String::new();
                        if r.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        loop {
                            let mut h = String::new();
                            if r.read_line(&mut h).unwrap_or(0) == 0 {
                                return;
                            }
                            if h.trim().is_empty() {
                                break;
                            }
                            if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
                                let (a, b) = v.trim().split_once('-').unwrap();
                                let a: usize = a.parse().unwrap();
                                let b: usize = b.parse().unwrap_or(data.len() - 1).min(data.len() - 1);
                                range = Some((a, b));
                            }
                        }
                        let (a, b) = range.unwrap_or((0, data.len() - 1));
                        let head = match range {
                            Some(_) => format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\n\r\n", data.len(), b + 1 - a),
                            None => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", data.len()),
                        };
                        if s.write_all(head.as_bytes()).is_err() {
                            return;
                        }
                        for c in data[a..=b].chunks(32 << 10) {
                            if s.write_all(c).is_err() {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    }
                });
            }
        });
        port
    }

    fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(90);
        while !f() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn without_a_system_downloads_folder_downloads_go_to_the_home_folder_s() {
        let home = PathBuf::from("/home/u");
        assert_eq!(pick_download_dir(Some("/data/dl".into()), Some(home.clone())), PathBuf::from("/data/dl"));
        assert_eq!(pick_download_dir(None, Some(home)), PathBuf::from("/home/u/Downloads"));
        assert_eq!(pick_download_dir(None, None), PathBuf::from("."));
    }

    #[test]
    fn repeating_a_note_is_not_a_change() {
        // Torrents set their note twice a second; an unchanged note must not move the revision
        // (every poller would refetch the list) or schedule a save (an idle seed rewriting it).
        let dir = std::env::temp_dir().join(format!("reidm-note-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let m = Manager::open(&dir.join("data"), dir.join("dl"), 4).unwrap();
        let id = m.add_with(Request { url: "http://127.0.0.1:9/f.bin".into(), ..Default::default() }, AddOptions { later: true, ..Default::default() }).unwrap().id;
        m.set_note(id, Some("looking for peers"));
        let rev = m.revision();
        for _ in 0..10 {
            m.set_note(id, Some("looking for peers"));
        }
        assert_eq!(m.revision(), rev);
        m.set_note(id, None);
        assert!(m.revision() > rev);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_short_of_space_is_told_the_shortfall_not_its_own_size() {
        // The note is what the window shows on hover. It used to say how many bytes the
        // download still had to write — "needs 1 B more" while the size was unknown — which
        // said nothing about the drive being short of the free space kept.
        let dir = std::env::temp_dir().join(format!("reidm-space-note-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let m = Manager::open(&dir.join("data"), dir.join("dl"), 1).unwrap();
        m.update_settings(SettingsPatch { min_free_space_mib: Some(200), ..Default::default() }).unwrap();
        let id = m.add_with(Request { url: "http://127.0.0.1:9/f.bin".into(), ..Default::default() }, AddOptions { later: true, ..Default::default() }).unwrap().id;
        // Every drive reports 100 MiB free; the size is not known yet, so it needs one byte.
        m.0.faults.free_space.store((100 << 20) + 1, Ordering::SeqCst);
        let note = m.space_shortage(&m.get(id).unwrap()).expect("short of space");
        assert!(
            note.contains("needs 100.0 MiB more free space, 100.0 MiB free (reIDM keeps 200.0 MiB free"),
            "{note}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_write_never_leaves_counted_bytes_missing() {
        let data: Vec<u8> = (0..4_000_000u32).map(|i| (i % 251) as u8 ^ (i >> 13) as u8).collect();
        let port = serve(data.clone());
        let dir = std::env::temp_dir().join(format!("reidm-fault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dl = dir.join("dl");
        let m = Manager::open(&dir.join("data"), dl.clone(), 4).unwrap();
        // The disk "fills up" a quarter of the way in, part way through some connection's batch.
        m.0.faults.fail_after.store(1_000_000 + 12_345, Ordering::SeqCst);
        let id = m.add(Request { url: format!("http://127.0.0.1:{port}/f.bin"), dir: Some(dl), ..Default::default() }).unwrap();
        wait_for("the full disk to stop the download", || matches!(m.get(id).unwrap().status, Status::Failed(_)));

        let it = m.get(id).unwrap();
        let Status::Failed(why) = &it.status else { unreachable!() };
        assert!(why.contains("disk is full"), "the user is told what to do: {why}");
        // Every byte the list counts is in the part file — none of them is a hole.
        let part = std::fs::read(m.part_for(&it)).unwrap();
        let mut counted = 0;
        for s in it.segments() {
            let (a, b) = (s.start as usize, (s.start + s.done) as usize);
            assert_eq!(&part[a..b], &data[a..b], "range {a}..{b} is counted but not on disk");
            counted += s.done;
        }
        assert!(counted > 0 && counted <= 1_012_345, "{counted}");
        assert_eq!(it.downloaded, counted);
        // What is saved for the next start says the same.
        assert!(m.flush());
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("data/downloads.json")).unwrap()).unwrap();
        let saved_done: u64 = saved["items"][id.to_string()]["segments"].as_array().unwrap().iter().map(|s| s["done"].as_u64().unwrap()).sum();
        assert_eq!(saved_done, counted);

        // Space is freed and the download resumed: the result is the file, byte for byte.
        m.0.faults.fail_after.store(0, Ordering::SeqCst);
        m.resume(id).unwrap();
        wait_for("the resumed download to finish", || m.get(id).unwrap().status == Status::Done);
        assert!(std::fs::read(m.get(id).unwrap().path).unwrap() == data, "the resumed file differs from the original");
    }

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
    fn names_that_disguise_their_type_are_defused() {
        // RIGHT-TO-LEFT OVERRIDE would display this as "invoiceexe.pdf".
        assert_eq!(sanitize_filename("invoice\u{202E}fdp.exe"), "invoice_fdp.exe");
        assert_eq!(sanitize_filename("a\u{2067}b.txt"), "a_b.txt");
    }

    #[test]
    fn long_names_are_cut_keeping_the_extension() {
        let long = format!("{}.tar.gz", "é".repeat(200));
        let s = sanitize_filename(&long);
        assert!(s.len() <= MAX_NAME_BYTES, "{}", s.len());
        assert!(s.ends_with("é.gz"), "cut on a character boundary, extension kept: {s}");
        // A part file's name ("<id>-<name>.reidm") must still fit in 255 bytes.
        assert!(format!("{}-{s}.reidm", u64::MAX).len() <= 255);
        assert_eq!(sanitize_filename("short.zip"), "short.zip");
    }

    #[test]
    fn names_taken_on_disk_are_skipped_before_the_list_is_locked() {
        // The candidates are worked out without the list's lock, so a stat on a slow or synced
        // folder cannot hold up the window and the other downloads.
        let d = std::env::temp_dir().join(format!("reidm-free-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("f.bin"), b"x").unwrap();
        // A part file of an earlier run counts as taken too.
        std::fs::write(d.join("f (1).bin.reidm"), b"x").unwrap();
        assert_eq!(free_names(&d, "f.bin", 2), vec![d.join("f (2).bin"), d.join("f (3).bin")]);
        // Once its file is gone, the plain name is free again.
        std::fs::remove_file(d.join("f.bin")).unwrap();
        assert_eq!(free_names(&d, "f.bin", 2), vec![d.join("f.bin"), d.join("f (2).bin")]);
        let _ = std::fs::remove_dir_all(&d);
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
        // An encoded separator inside a value is not a separator.
        assert_ne!(k("https://a.example/f?a=1%26b%3D2"), k("https://a.example/f?a=1&b=2"));
        // A repeated parameter's order is part of what it means.
        assert_ne!(k("https://a.example/f?id=2&id=1"), k("https://a.example/f?id=1&id=2"));
        assert_eq!(k("https://a.example/f?b=1&id=2&a=0&id=1"), k("https://a.example/f?id=2&a=0&id=1&b=1"));
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
    fn the_scheduler_does_not_repeat_the_hour_the_clocks_go_back() {
        let t = |h: u32, m: u32| chrono::NaiveDate::from_ymd_opt(2026, 10, 25).unwrap().and_hms_opt(h, m, 0).unwrap();
        // 02:59 summer time, then 02:00 winter time: the mark stays at 02:59...
        assert_eq!(scheduler_mark(t(2, 59), t(2, 0)), t(2, 59));
        // ...so a start at 02:30 is not due a second time in the repeated hour.
        let sched = schedule::Schedule { enabled: true, start: Some("02:30".into()), ..Default::default() };
        assert_eq!(sched.due(t(2, 0), t(2, 31)), Some(schedule::Action::Start), "following the clock back starts it twice");
        assert_eq!(sched.due(scheduler_mark(t(2, 59), t(2, 0)), t(2, 31)), None);
        // Forward time moves on as ever, and a big step back (the clock being set) is followed.
        assert_eq!(scheduler_mark(t(2, 0), t(2, 1)), t(2, 1));
        assert_eq!(scheduler_mark(t(9, 0), t(1, 0)), t(1, 0));
    }

    #[test]
    fn content_range_parses() {
        assert_eq!(parse_content_range("bytes 0-0/1234"), Some((0, 0, Some(1234))));
        assert_eq!(parse_content_range("bytes 100-199/*"), Some((100, 199, None)));
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
