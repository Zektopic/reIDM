//! Client for the local reIDM daemon.
//!
//! The GUI is a *front-end*: the daemon owns the download engine and the browser-extension
//! ports, so the GUI can be closed and downloads keep running. A poller thread keeps a snapshot
//! of the list fresh and wakes the window only when something changed; commands go through one
//! worker thread (`worker.rs`), so the window's own thread never waits on the network.
//!
//! The list comes from `GET /api/downloads/rows`: the first poll fetches every row, and each
//! later one only the rows that changed since — or nothing at all (`304`) while the list is
//! idle. A list of 50,000 downloads therefore costs one transfer when the window opens, not
//! one every few hundred milliseconds.

use crate::model::{ExtensionSeen, Item, Settings, Status};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Port the daemon serves its JSON API on, unless `REIDM_UI_PORT` says otherwise (as for the
/// daemon itself; see `reidm::web::ui_port`).
pub fn ui_port() -> u16 {
    std::env::var("REIDM_UI_PORT").ok().and_then(|v| v.trim().parse().ok()).filter(|p| *p != 0).unwrap_or(61002)
}

/// The most a single answer may be. ureq's default of 10 MB is what made a list of about
/// 7,000 downloads unloadable; rows are about 400 bytes, so this holds well over 500,000.
const BODY_LIMIT: u64 = 256 << 20;

/// What the GUI currently knows about the daemon. Cheap to clone: the list is shared.
#[derive(Default, Clone)]
pub struct Snapshot {
    /// Changes whenever anything the window shows changed; the window keys its caches on it.
    pub rev: u64,
    /// Every download, in id order.
    pub items: Arc<Vec<Arc<Item>>>,
    /// Position of each id in `items`.
    pub index: Arc<HashMap<u64, usize>>,
    /// Per-item transfer rate, sampled by the poller (the API has only a byte counter).
    pub rates: Arc<HashMap<u64, f64>>,
    /// Per-torrent upload rate, sampled the same way from what each has shared.
    pub up_rates: Arc<HashMap<u64, f64>>,
    pub total_rate: f64,
    pub connected: bool,
    /// Why the service can't be reached, when it can't.
    pub problem: Option<String>,
    /// Refreshed every few seconds rather than on every poll.
    pub service: Arc<Service>,
}

impl Snapshot {
    pub fn item(&self, id: u64) -> Option<&Item> {
        self.index.get(&id).and_then(|i| self.items.get(*i)).map(|a| a.as_ref())
    }

    /// The row itself, shared: for code that must keep it while the window's state changes.
    /// Cloning the `Arc` is free; cloning the row is not.
    pub fn item_arc(&self, id: u64) -> Option<Arc<Item>> {
        self.index.get(&id).and_then(|i| self.items.get(*i)).cloned()
    }
}

/// The slower-changing half of the daemon's state.
#[derive(Default, Clone, PartialEq)]
pub struct Service {
    /// Owns the port the official IDM extension dials first.
    pub idm_port: bool,
    pub platform: String,
    pub version: String,
    pub settings: Option<Settings>,
    /// reIDM's own extension, if it has checked in since the daemon started.
    pub extension: Option<ExtensionSeen>,
    /// A download waiting for the browser to supply a new address.
    pub refresh_armed: Option<u64>,
    /// The folder to point the browser's "Load unpacked" at.
    pub extension_dir: Option<String>,
    /// Problems with the download list's storage the user should know about.
    pub warnings: Vec<String>,
    /// Free space on the download and temporary folders' disks; `None` from a service that
    /// doesn't report it.
    pub free_space: Option<Vec<crate::model::FreeSpace>>,
    /// What happens when everything is done, and the countdown to it; `None` from a service
    /// that can't do it.
    pub when_done: Option<crate::when_done::WhenDone>,
    /// The service is stopping (*When done › Exit*): the window closes too.
    pub exiting: bool,
}

/// One line of a batch add's reply.
#[derive(Debug, Clone)]
pub struct BatchResult {
    pub url: String,
    /// `new`, `existing` or `already_complete`; `None` when the URL failed.
    pub outcome: Option<String>,
    pub error: Option<String>,
}

/// How far back the transfer rate looks. A single poll interval is too short: a download held
/// back by a speed limit, or a server that sends in bursts, would flicker between zero and
/// double its real rate.
const RATE_WINDOW: Duration = Duration::from_secs(3);

/// Read the token the daemon publishes in its data directory, once, without waiting: a missing
/// file is the poller's to wait out, never a click's.
fn read_token_now(file: &std::path::Path) -> Option<String> {
    let s = std::fs::read_to_string(file).ok()?;
    Some(s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Is reIDM's service answering on its port? A bare TCP connect would also succeed against some
/// other program that took the port, and the window would then wait for ever.
fn daemon_up() -> bool {
    let a = agent(Duration::from_millis(800));
    let Ok(mut r) = a.get(format!("http://127.0.0.1:{}/ext/ping", ui_port())).call() else { return false };
    r.status().is_success() && r.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).is_some_and(|v| v["app"] == "reIDM")
}

/// Make sure a daemon is running, starting one if needed. Returns false if it never came up.
/// Blocks for up to 15 s, so call it from the poller or `--background`, never a frame.
pub fn ensure_daemon() -> bool {
    if daemon_up() {
        return true;
    }
    let exe = std::env::current_exe().ok().map(|p| p.with_file_name(if cfg!(windows) { "reidm.exe" } else { "reidm" }));
    let Some(exe) = exe else { return false };

    let mut c = std::process::Command::new(exe);
    c.arg("daemon").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0000_0008 | 0x0800_0000); // DETACHED_PROCESS | CREATE_NO_WINDOW
    }
    #[cfg(unix)]
    {
        // Its own process group, so closing a terminal that started the window does not take the
        // service down with it.
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let Ok(mut child) = c.spawn() else { return false };
    for _ in 0..60 {
        if daemon_up() {
            // It runs on; reap it if it ever exits while the window is open.
            std::thread::spawn(move || child.wait());
            return true;
        }
        // A service that exits at once (the port is taken, or one is already running for this
        // data folder) has said why on its own stderr; there is no point waiting it out.
        if let Ok(Some(_)) = child.try_wait() {
            return daemon_up();
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        // Error statuses come back as responses, so the daemon's explanation can be shown.
        .http_status_as_error(false)
        .build()
        .new_agent()
}

/// An answer from the service.
pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body on success; otherwise the service's explanation, or the status.
    pub fn into_result(self) -> Result<String, String> {
        if self.ok() {
            Ok(self.body)
        } else if self.body.trim().is_empty() {
            Err(format!("the service answered {}", self.status))
        } else {
            Err(self.body)
        }
    }
}

/// Talks to the service. Clones share the token, so when the service restarts with a new one,
/// the first request that is refused fetches it for everybody.
#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    base: String,
    token: Arc<Mutex<Option<String>>>,
    token_file: Arc<std::path::PathBuf>,
}

impl Client {
    pub fn new(timeout: Duration) -> Client {
        Client {
            agent: agent(timeout),
            base: format!("http://127.0.0.1:{}", ui_port()),
            token: Default::default(),
            token_file: Arc::new(reidm_core::default_data_dir().join("api-token")),
        }
    }

    /// The same service and token, with another timeout (a playlist lookup can take minutes).
    pub fn with_timeout(&self, timeout: Duration) -> Client {
        Client { agent: agent(timeout), ..self.clone() }
    }

    fn token(&self) -> Option<String> {
        let mut t = self.token.lock().unwrap_or_else(|p| p.into_inner());
        if t.is_none() {
            *t = read_token_now(&self.token_file);
        }
        t.clone()
    }

    /// Forget a token the service refused, and read the file again. Whether it changed, i.e.
    /// whether trying again could help.
    fn renew_token(&self, refused: &str) -> bool {
        let mut t = self.token.lock().unwrap_or_else(|p| p.into_inner());
        *t = read_token_now(&self.token_file);
        t.as_deref().is_some_and(|n| n != refused)
    }

    /// One request. A `401` means the service restarted with a new token: read it and try once
    /// more.
    pub fn request(&self, method: &str, path: &str, body: Option<&serde_json::Value>, headers: &[(&str, &str)]) -> Result<Reply, String> {
        let token = self.token().ok_or("reIDM's background service is not running")?;
        let r = self.send(method, path, body, headers, &token)?;
        if r.status == 401 && self.renew_token(&token) {
            let token = self.token().ok_or("reIDM's background service is not running")?;
            return self.send(method, path, body, headers, &token);
        }
        Ok(r)
    }

    fn send(&self, method: &str, path: &str, body: Option<&serde_json::Value>, headers: &[(&str, &str)], token: &str) -> Result<Reply, String> {
        let url = format!("{}{path}", self.base);
        let r = match method {
            "GET" | "DELETE" => {
                let mut b = if method == "GET" { self.agent.get(&url) } else { self.agent.delete(&url) };
                b = b.header("X-Reidm-Token", token);
                for (k, v) in headers {
                    b = b.header(*k, *v);
                }
                b.call()
            }
            _ => {
                let mut b = self.agent.post(&url).header("X-Reidm-Token", token).header("Content-Type", "application/json");
                for (k, v) in headers {
                    b = b.header(*k, *v);
                }
                b.send(body.map(|b| b.to_string()).unwrap_or_else(|| "{}".into()))
            }
        };
        let mut r = r.map_err(|e| match e {
            ureq::Error::Io(ref io) if io.kind() == std::io::ErrorKind::ConnectionRefused => "reIDM's background service is not running".to_string(),
            e => e.to_string(),
        })?;
        let status = r.status().as_u16();
        let body = if status == 304 { String::new() } else { r.body_mut().with_config().limit(BODY_LIMIT).read_to_string().map_err(|e| e.to_string())? };
        Ok(Reply { status, body })
    }

    pub fn get(&self, path: &str) -> Result<String, String> {
        self.request("GET", path, None, &[])?.into_result()
    }

    pub fn post(&self, path: &str, body: serde_json::Value) -> Result<String, String> {
        self.request("POST", path, Some(&body), &[])?.into_result()
    }

    /// POST, and decode the answer.
    pub fn post_json<T: serde::de::DeserializeOwned>(&self, path: &str, body: serde_json::Value) -> Result<T, String> {
        serde_json::from_str(&self.post(path, body)?).map_err(|e| e.to_string())
    }

    pub fn settings(&self) -> Result<Settings, String> {
        serde_json::from_str(&self.get("/api/settings")?).map_err(|e| e.to_string())
    }

    /// One download with every field, for Properties.
    pub fn detail(&self, id: u64) -> Result<crate::model::Detail, String> {
        serde_json::from_str(&self.get(&format!("/api/downloads/{id}"))?).map_err(|e| e.to_string())
    }

    /// The same action on many downloads, in one request. Answers how many it applied to, and
    /// the first refusal.
    pub fn bulk(&self, ids: &[u64], action: &str) -> Result<(usize, Option<String>), String> {
        let mut done = 0;
        let mut first_error = None;
        // The service takes at most 10,000 per request.
        for chunk in ids.chunks(10_000) {
            let v: serde_json::Value = self.post_json("/api/downloads/bulk", serde_json::json!({ "ids": chunk, "action": action }))?;
            done += v["ok"].as_u64().unwrap_or(0) as usize;
            if first_error.is_none() {
                first_error = v["results"].as_array().into_iter().flatten().find_map(|r| r["error"].as_str().map(str::to_string));
            }
        }
        Ok((done, first_error))
    }

    /// Queue many URLs, or entries with their own referer and cookie. One line per URL back.
    pub fn batch(&self, body: serde_json::Value) -> Result<Vec<BatchResult>, String> {
        let v: serde_json::Value = self.post_json("/api/downloads/batch", body)?;
        Ok(v["results"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| BatchResult {
                url: r["url"].as_str().unwrap_or_default().to_string(),
                outcome: r["outcome"].as_str().map(str::to_string),
                error: r["error"].as_str().map(str::to_string),
            })
            .collect())
    }

    /// Status and settings in one go.
    fn service(&self) -> Option<Service> {
        let v: serde_json::Value = serde_json::from_str(&self.get("/api/status").ok()?).ok()?;
        Some(Service {
            idm_port: v["idm_port"].as_bool().unwrap_or(false),
            platform: v["platform"].as_str().unwrap_or_default().to_string(),
            version: v["version"].as_str().unwrap_or_default().to_string(),
            extension: serde_json::from_value(v["extension"].clone()).ok(),
            refresh_armed: v["refresh_armed"].as_u64(),
            extension_dir: v["extension_dir"].as_str().map(str::to_string),
            warnings: v["warnings"].as_array().into_iter().flatten().filter_map(|w| w.as_str().map(str::to_string)).collect(),
            free_space: serde_json::from_value(v["free_space"].clone()).ok(),
            when_done: serde_json::from_value(v["when_done"].clone()).ok(),
            exiting: v["exiting"].as_bool().unwrap_or(false),
            settings: self.settings().ok(),
        })
    }
}

/// Calls that can take minutes (yt-dlp fetching itself, a magnet link waiting for peers, a
/// page scan). Each runs on its own thread and answers over a channel, so it neither blocks a
/// frame nor holds up the command thread's quick jobs behind it.
impl Client {
    fn in_background<T: Send + 'static>(&self, timeout: Duration, f: impl FnOnce(&Client) -> T + Send + 'static) -> Receiver<T> {
        let (tx, rx) = std::sync::mpsc::channel();
        let c = self.with_timeout(timeout);
        std::thread::spawn(move || {
            let _ = tx.send(f(&c));
        });
        rx
    }

    /// The videos of a playlist. Slow the first time: the service fetches yt-dlp.
    pub fn playlist_async(&self, url: &str) -> Receiver<Result<reidm_core::media::Playlist, String>> {
        let body = serde_json::json!({ "url": url });
        self.in_background(Duration::from_secs(600), move |c| c.post_json("/api/media/playlist", body))
    }

    /// The heights and codecs one video comes in.
    pub fn video_info_async(&self, url: &str) -> Receiver<Result<reidm_core::media::VideoInfo, String>> {
        let body = serde_json::json!({ "url": url });
        self.in_background(Duration::from_secs(600), move |c| c.post_json("/api/media/info", body))
    }

    /// What a torrent holds. For a magnet link this waits for peers to describe it.
    pub fn torrent_inspect_async(&self, url: Option<String>, data: Option<String>) -> Receiver<Result<reidm_core::TorrentInfo, String>> {
        let body = serde_json::json!({ "url": url, "data": data });
        self.in_background(Duration::from_secs(180), move |c| c.post_json("/api/torrent/inspect", body))
    }

    /// Add a torrent: by link, or a `.torrent` file's bytes (base64); `files` picks some.
    pub fn add_torrent_async(&self, url: &str, data: Option<String>, files: Option<Vec<usize>>, later: bool) -> Receiver<Result<(), String>> {
        let body = serde_json::json!({ "url": url, "later": later, "torrent": { "files": files, "data": data } });
        self.in_background(Duration::from_secs(60), move |c| c.post("/api/downloads", body).map(|_| ()))
    }

    /// Queue videos, each `(page, media, file name)`. Answers how many were added, and the
    /// errors of those that weren't.
    pub fn add_videos_async(&self, items: Vec<(String, serde_json::Value, String)>, dir: Option<String>, later: bool) -> Receiver<(usize, Vec<String>)> {
        self.in_background(Duration::from_secs(30), move |c| {
            let (mut ok, mut errors) = (0, Vec::new());
            for (url, media, filename) in items {
                match c.post("/api/downloads", serde_json::json!({ "url": url, "media": media, "dir": dir, "later": later, "filename": filename })) {
                    Ok(_) => ok += 1,
                    Err(e) => errors.push(e),
                }
            }
            (ok, errors)
        })
    }

    /// Scan a page for files.
    pub fn grab_async(&self, url: &str, opts: &crate::model::GrabOptions) -> Receiver<Result<Vec<crate::model::Found>, String>> {
        let body = serde_json::json!({ "url": url, "options": opts });
        self.in_background(Duration::from_secs(180), move |c| {
            let v: serde_json::Value = c.post_json("/api/grab", body)?;
            serde_json::from_value(v["files"].clone()).map_err(|e| e.to_string())
        })
    }

    /// Compute a finished download's checksum. Reading a big file takes a while.
    pub fn verify_async(&self, id: u64, algo: &str) -> Receiver<Result<String, String>> {
        let body = serde_json::json!({ "algo": algo });
        self.in_background(Duration::from_secs(3600), move |c| {
            let v: serde_json::Value = c.post_json(&format!("/api/downloads/{id}/verify"), body)?;
            v["value"].as_str().map(str::to_string).ok_or_else(|| "the service gave no checksum".to_string())
        })
    }

    /// Point a download at a new address. The service checks the new address first, which takes
    /// a network round trip.
    pub fn refresh_async(&self, id: u64, url: &str) -> Receiver<Result<(), String>> {
        let body = serde_json::json!({ "url": url });
        self.in_background(Duration::from_secs(60), move |c| c.post(&format!("/api/downloads/{id}/refresh"), body).map(|_| ()))
    }
}

/// How often the poller asks, which the window adjusts to what it is showing.
#[derive(Clone)]
pub struct Pace(Arc<AtomicU64>);

impl Pace {
    /// Something is moving and the window is in front: keep the numbers live.
    pub const LIVE: Duration = Duration::from_millis(500);
    /// Nothing is moving: a change made elsewhere (the browser) still shows within a second.
    pub const IDLE: Duration = Duration::from_millis(1000);
    /// In the background: nobody is watching closely.
    pub const BACKGROUND: Duration = Duration::from_millis(3000);
    /// Minimised: only enough to notice completions.
    pub const HIDDEN: Duration = Duration::from_millis(5000);

    pub fn for_window(busy: bool, focused: bool, minimized: bool) -> Duration {
        match (minimized, focused, busy) {
            (true, _, _) => Self::HIDDEN,
            (false, false, _) => Self::BACKGROUND,
            (false, true, true) => Self::LIVE,
            (false, true, false) => Self::IDLE,
        }
    }

    pub fn set(&self, d: Duration) {
        self.0.store(d.as_millis() as u64, Ordering::Relaxed);
    }

    fn get(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::Relaxed).max(100))
    }
}

/// The list as the poller holds it, and the rows answer applied to it.
#[derive(Default)]
pub struct ListState {
    pub items: Vec<Arc<Item>>,
    pub index: HashMap<u64, usize>,
    pub cursor: Option<String>,
}

/// `GET /api/downloads/rows` as JSON.
#[derive(serde::Deserialize)]
pub struct RowsAnswer {
    pub cursor: String,
    pub full: bool,
    pub rows: Vec<Item>,
    #[serde(default)]
    pub removed: Vec<u64>,
}

impl ListState {
    /// Apply an answer. Whether anything changed.
    pub fn apply(&mut self, a: RowsAnswer) -> bool {
        self.cursor = Some(a.cursor);
        if a.full {
            let mut rows = a.rows;
            rows.sort_unstable_by_key(|i| i.id);
            let changed = rows.len() != self.items.len() || rows.iter().zip(&self.items).any(|(a, b)| a != b.as_ref());
            self.items = rows.into_iter().map(Arc::new).collect();
            self.reindex();
            return changed;
        }
        let mut changed = false;
        let mut reorder = false;
        for row in a.rows {
            match self.index.get(&row.id) {
                Some(&i) => {
                    if *self.items[i] != row {
                        self.items[i] = Arc::new(row);
                        changed = true;
                    }
                }
                None => {
                    // New downloads have the highest ids, so this is nearly always an append.
                    reorder |= self.items.last().is_some_and(|l| l.id > row.id);
                    self.index.insert(row.id, self.items.len());
                    self.items.push(Arc::new(row));
                    changed = true;
                }
            }
        }
        if !a.removed.is_empty() {
            let gone: HashSet<u64> = a.removed.into_iter().collect();
            let before = self.items.len();
            self.items.retain(|i| !gone.contains(&i.id));
            changed |= self.items.len() != before;
            reorder = true;
        }
        if reorder {
            self.items.sort_unstable_by_key(|i| i.id);
            self.reindex();
        }
        changed
    }

    fn reindex(&mut self) {
        self.index = self.items.iter().enumerate().map(|(i, it)| (it.id, i)).collect();
    }
}

/// Transfer rates from successive readings of each download's byte counter.
#[derive(Default)]
pub struct Rates {
    down: HashMap<u64, VecDeque<(Instant, u64)>>,
    up: HashMap<u64, VecDeque<(Instant, u64)>>,
}

fn sample(h: &mut VecDeque<(Instant, u64)>, now: Instant, v: u64) -> Option<f64> {
    if h.back().is_some_and(|(_, d)| *d > v) {
        // Restarted from zero: the old samples mean nothing now.
        h.clear();
    }
    h.push_back((now, v));
    while h.len() > 2 && h.front().is_some_and(|(t, _)| now.duration_since(*t) > RATE_WINDOW) {
        h.pop_front();
    }
    let (t0, d0) = h.front()?;
    let dt = now.duration_since(*t0).as_secs_f64();
    (dt > 0.25).then(|| (v - d0) as f64 / dt)
}

impl Rates {
    /// Rates at `now`: download rates of running downloads, upload rates of torrents. Only
    /// moving downloads are tracked, so a long finished history costs nothing here.
    pub fn update(&mut self, items: &[Arc<Item>], now: Instant) -> (HashMap<u64, f64>, HashMap<u64, f64>, f64) {
        let (mut down, mut up, mut total) = (HashMap::new(), HashMap::new(), 0.0);
        let mut live = HashSet::new();
        let mut torrents = HashSet::new();
        for it in items {
            if it.status == Status::Running {
                live.insert(it.id);
                if let Some(r) = sample(self.down.entry(it.id).or_default(), now, it.downloaded) {
                    down.insert(it.id, r);
                    total += r;
                }
            }
            if let Some(t) = it.torrent.as_ref().filter(|t| t.seeding || it.status == Status::Running) {
                torrents.insert(it.id);
                if let Some(r) = sample(self.up.entry(it.id).or_default(), now, t.uploaded).filter(|r| *r > 0.0) {
                    up.insert(it.id, r);
                }
            }
        }
        self.down.retain(|id, _| live.contains(id));
        self.up.retain(|id, _| torrents.contains(id));
        (down, up, total)
    }

    pub fn tracking(&self) -> bool {
        !self.down.is_empty() || !self.up.is_empty()
    }
}

/// Starts the poller. It tells the window about a new snapshot by sending it and asking for a
/// repaint; when nothing changed, it does neither.
pub struct Poller {
    pub rx: Receiver<Snapshot>,
    pub pace: Pace,
    /// Ask now rather than at the next tick (after a command, so its effect shows at once).
    pub poke: Sender<()>,
    pub client: Client,
}

impl Poller {
    pub fn start(ctx: Option<egui::Context>) -> Poller {
        let (tx, rx) = std::sync::mpsc::channel();
        let (poke, poked) = std::sync::mpsc::channel::<()>();
        let pace = Pace(Arc::new(AtomicU64::new(Pace::IDLE.as_millis() as u64)));
        let client = Client::new(Duration::from_secs(10));
        let (c, p) = (client.with_timeout(Duration::from_secs(30)), pace.clone());
        std::thread::Builder::new()
            .name("reidm-poller".into())
            .spawn(move || poll_loop(c, tx, p, poked, ctx))
            .expect("cannot start the poller thread");
        Poller { rx, pace, poke, client }
    }
}

fn poll_loop(client: Client, tx: Sender<Snapshot>, pace: Pace, poked: Receiver<()>, ctx: Option<egui::Context>) {
    let repaint = || {
        if let Some(c) = &ctx {
            c.request_repaint();
        }
    };
    let mut snap = Snapshot { problem: Some("Starting the download service…".into()), ..Default::default() };
    let _ = tx.send(snap.clone());
    repaint();
    // Started here rather than before the window opens: the window shows at once, saying what
    // it is waiting for, instead of after up to 15 silent seconds.
    if !ensure_daemon() {
        snap.problem = Some("reIDM's background service could not be started. Is `reidm` installed next to this program?".into());
        snap.rev += 1;
        let _ = tx.send(snap.clone());
        repaint();
    }
    let mut list = ListState::default();
    let mut rates = Rates::default();
    let mut last_service = Instant::now() - Duration::from_secs(60);
    loop {
        let mut changed = false;
        let mut list_changed = false;
        // Settings and integration status change rarely: every few seconds, or after a command.
        let poked_now = poked.try_iter().count() > 0;
        if poked_now || last_service.elapsed() >= Duration::from_secs(3) {
            if let Some(s) = client.service() {
                if *snap.service != s {
                    snap.service = Arc::new(s);
                    changed = true;
                }
            }
            last_service = Instant::now();
        }

        let since = list.cursor.clone();
        let path = match &since {
            Some(c) => format!("/api/downloads/rows?since={c}"),
            None => "/api/downloads/rows".to_string(),
        };
        match client.request("GET", &path, None, &[]) {
            Ok(r) if r.status == 304 => {
                if !snap.connected {
                    snap.connected = true;
                    snap.problem = None;
                    changed = true;
                }
            }
            Ok(r) if r.ok() => match serde_json::from_str::<RowsAnswer>(&r.body) {
                Ok(a) => {
                    list_changed = list.apply(a);
                    changed |= list_changed;
                    if !snap.connected {
                        snap.connected = true;
                        snap.problem = None;
                        changed = true;
                    }
                }
                Err(e) => {
                    list.cursor = None;
                    changed |= set_problem(&mut snap, format!("could not read the download list: {e}"));
                }
            },
            Ok(r) => {
                list.cursor = None;
                let why = r.into_result().err().unwrap_or_default();
                changed |= set_problem(&mut snap, why);
            }
            Err(e) => {
                // The service restarted, or stopped: start over with the whole list.
                list.cursor = None;
                changed |= set_problem(&mut snap, e);
            }
        }

        if changed || rates.tracking() {
            let (down, up, total) = rates.update(&list.items, Instant::now());
            if changed || *snap.rates != down || *snap.up_rates != up {
                snap.rates = Arc::new(down);
                snap.up_rates = Arc::new(up);
                snap.total_rate = total;
                changed = true;
            }
        }
        if changed {
            if list_changed {
                // A copy of the pointers, not of the rows: the window keeps reading the previous
                // list while this one is built.
                snap.items = Arc::new(list.items.clone());
                snap.index = Arc::new(list.index.clone());
            }
            snap.rev += 1;
            if tx.send(snap.clone()).is_err() {
                return; // the window has closed
            }
            repaint();
        }
        let _ = poked.recv_timeout(pace.get());
    }
}

fn set_problem(snap: &mut Snapshot, why: String) -> bool {
    let was = (snap.connected, snap.problem.clone());
    snap.connected = false;
    snap.problem = Some(why);
    was != (snap.connected, snap.problem.clone())
}

/// Hand a URL or a folder to the desktop: the browser for a link, the file manager for a path.
pub fn open_external(target: &str) {
    #[cfg(windows)]
    let mut c = {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", target]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut c = {
        let mut c = std::process::Command::new("open");
        c.arg(target);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut c = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(target);
        c
    };
    if let Ok(mut child) = c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
        // Waited for, so it doesn't linger as a zombie while the window is open.
        std::thread::spawn(move || child.wait());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::tests::row;
    use std::io::{BufRead, BufReader, Write};

    /// A stand-in service that answers `200 ok` to the token `good` and `401` to anything else.
    fn service(good: &'static str) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut token = String::new();
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("x-reidm-token:") {
                        token = v.trim().to_string();
                    }
                }
                let (status, body) = if token == good { ("200 OK", "ok") } else { ("401 Unauthorized", "") };
                let _ = write!(&c, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        base
    }

    fn client(base: String, file: &std::path::Path) -> Client {
        Client { base, token_file: Arc::new(file.to_path_buf()), ..Client::new(Duration::from_secs(5)) }
    }

    #[test]
    fn a_restarted_service_s_new_token_is_picked_up_without_a_restart_of_the_window() {
        let dir = std::env::temp_dir().join(format!("reidm-gui-token-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("api-token");
        std::fs::write(&file, "old").unwrap();
        let c = client(service("new"), &file);
        assert!(c.get("/x").is_err(), "the old token is refused");
        // The service restarts and publishes a new token: the next command just works.
        std::fs::write(&file, "new").unwrap();
        assert_eq!(c.get("/x").unwrap(), "ok");
        // And every clone (the poller, the command thread) has it too.
        assert_eq!(c.with_timeout(Duration::from_secs(1)).get("/x").unwrap(), "ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_token_means_an_answer_at_once_not_a_wait() {
        let c = client(service("x"), std::path::Path::new("/nonexistent/api-token"));
        let t = Instant::now();
        let e = c.get("/x").unwrap_err();
        assert!(t.elapsed() < Duration::from_millis(50), "{:?}", t.elapsed());
        assert!(e.contains("not running"), "{e}");
    }

    fn answer(full: bool, rows: Vec<crate::model::Item>, removed: Vec<u64>) -> RowsAnswer {
        RowsAnswer { cursor: "c".into(), full, rows, removed }
    }

    #[test]
    fn deltas_update_the_list_in_place() {
        let mut l = ListState::default();
        assert!(l.apply(answer(true, vec![row(3, "c", "Done", 0), row(1, "a", "Done", 0), row(2, "b", "Running", 5)], vec![])));
        assert_eq!(l.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![1, 2, 3], "kept in id order");
        let before = l.items[0].clone();

        assert!(l.apply(answer(false, vec![row(2, "b", "Running", 50), row(4, "d", "Queued", 0)], vec![3])));
        assert_eq!(l.items.iter().map(|i| (i.id, i.downloaded)).collect::<Vec<_>>(), vec![(1, 0), (2, 50), (4, 0)]);
        assert!(Arc::ptr_eq(&before, &l.items[0]), "an unchanged row is shared, not copied");
        assert_eq!(l.index[&4], 2);

        assert!(!l.apply(answer(false, vec![row(2, "b", "Running", 50)], vec![])), "the same row again is no change");
        assert!(!l.apply(answer(true, l.items.iter().map(|i| (**i).clone()).collect(), vec![])), "nor is the same full list");
    }

    #[test]
    fn rates_track_only_moving_downloads() {
        let mut r = Rates::default();
        let mut items: Vec<Arc<crate::model::Item>> = (1..=10_000).map(|i| Arc::new(row(i, "f", "Done", 1000))).collect();
        items.push(Arc::new(row(20_000, "g", "Running", 0)));
        let t0 = Instant::now();
        r.update(&items, t0);
        assert_eq!(r.down.len(), 1, "a long finished history keeps no samples");
        *items.last_mut().unwrap() = Arc::new(row(20_000, "g", "Running", 1000));
        let (down, _, total) = r.update(&items, t0 + Duration::from_secs(1));
        assert!((down[&20_000] - 1000.0).abs() < 1.0, "{down:?}");
        assert!((total - 1000.0).abs() < 1.0);
        *items.last_mut().unwrap() = Arc::new(row(20_000, "g", "Paused", 1000));
        r.update(&items, t0 + Duration::from_secs(2));
        assert!(!r.tracking(), "a paused download stops being sampled");
    }

    #[test]
    fn polling_slows_down_when_nobody_is_watching() {
        assert_eq!(Pace::for_window(true, true, false), Pace::LIVE);
        assert_eq!(Pace::for_window(false, true, false), Pace::IDLE);
        assert_eq!(Pace::for_window(true, false, false), Pace::BACKGROUND);
        assert_eq!(Pace::for_window(true, true, true), Pace::HIDDEN);
    }
}
