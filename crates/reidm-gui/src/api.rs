//! Client for the local reIDM daemon.
//!
//! The GUI is a *front-end*: the daemon owns the download engine and the browser-extension
//! ports, so the GUI can be closed and downloads keep running. The GUI keeps a snapshot fresh on a
//! background thread and sends commands over the same JSON API the web page uses.

use crate::model::{ExtensionSeen, Item, Settings, Status};
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Port the daemon serves its JSON API on (must match `reidm::web::UI_PORT`).
pub const UI_PORT: u16 = 61002;
// The daemon also serves the IDM extension on ws://127.0.0.1:61001 (`reidm::ws::LOCAL_PORT`);
// the GUI never needs that socket itself.

/// What the GUI currently knows about the daemon.
#[derive(Default, Clone)]
pub struct Snapshot {
    pub items: Vec<Item>,
    /// Per-item transfer rate, sampled by the poller (the API has only a byte counter).
    pub rates: std::collections::HashMap<u64, f64>,
    pub total_rate: f64,
    pub connected: bool,
    /// Set when a command failed, so the UI can show it.
    pub last_error: Option<String>,
    /// Refreshed every few seconds rather than on every poll.
    pub service: Service,
}

/// The slower-changing half of the daemon's state.
#[derive(Default, Clone)]
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
}

/// One line of a batch add's reply.
#[derive(Debug, Clone)]
pub struct BatchResult {
    pub url: String,
    /// `new`, `existing` or `already_complete`; `None` when the URL failed.
    pub outcome: Option<String>,
    pub error: Option<String>,
}

/// How far back the transfer rate looks. A single 300 ms poll interval is too short: a download
/// held back by a speed limit, or a server that sends in bursts, would flicker between zero and
/// double its real rate.
const RATE_WINDOW: Duration = Duration::from_secs(3);

pub struct Api {
    agent: ureq::Agent,
    base: String,
    token: Mutex<Option<String>>,
    snapshot: Arc<Mutex<Snapshot>>,
    rx: Receiver<Snapshot>,
    /// Keep the poller alive for the lifetime of the app.
    _poller: std::thread::JoinHandle<()>,
}

fn daemon_up() -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], UI_PORT).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Read the token the daemon publishes in its own data directory. Retries briefly because the
/// daemon writes it just after it starts listening.
fn read_token() -> Option<String> {
    let path = reidm_core::default_data_dir().join("api-token");
    for _ in 0..60 {
        if let Ok(s) = std::fs::read_to_string(&path) {
            let s = s.trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    None
}

/// Make sure a daemon is running, starting one if needed. Returns false if it never came up.
pub fn ensure_daemon() -> bool {
    if daemon_up() {
        return true;
    }
    let exe = std::env::current_exe()
        .ok()
        .map(|p| p.with_file_name(if cfg!(windows) { "reidm.exe" } else { "reidm" }));
    let Some(exe) = exe else { return false };

    let mut c = std::process::Command::new(exe);
    c.arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
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
    if c.spawn().is_err() {
        return false;
    }
    for _ in 0..60 {
        if daemon_up() {
            return true;
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

impl Api {
    pub fn start() -> Api {
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let (tx, rx) = std::sync::mpsc::channel();

        let base = format!("http://127.0.0.1:{UI_PORT}");
        let poller = {
            let snapshot = snapshot.clone();
            let mut token = None;
            let base2 = base.clone();
            std::thread::spawn(move || {
                let agent = agent(Duration::from_secs(5));
                let mut history: std::collections::HashMap<u64, std::collections::VecDeque<(Instant, u64)>> = Default::default();
                let mut service = Service::default();
                let mut ticks: u64 = 0;
                loop {
                    if token.is_none() {
                        token = read_token();
                    }
                    // Settings and integration status change rarely: refresh about every 3 s.
                    if ticks.is_multiple_of(10) {
                        if let Some(t) = token.as_deref() {
                            if let Some(s) = fetch_service(&agent, &base2, t) {
                                service = s;
                            }
                        }
                    }
                    ticks += 1;
                    let mut snap = match token.as_deref() {
                        Some(t) => match fetch(&agent, &base2, t) {
                            Ok(items) => {
                                let now = Instant::now();
                                let mut rates = std::collections::HashMap::new();
                                let mut total = 0.0;
                                for it in &items {
                                    let h = history.entry(it.id).or_default();
                                    if it.status != Status::Running || h.back().is_some_and(|(_, d)| *d > it.downloaded) {
                                        // Stopped, or restarted from zero: the old samples mean nothing now.
                                        h.clear();
                                    }
                                    h.push_back((now, it.downloaded));
                                    while h.len() > 2 && h.front().is_some_and(|(t, _)| now.duration_since(*t) > RATE_WINDOW) {
                                        h.pop_front();
                                    }
                                    if let (Some((t0, d0)), true) = (h.front(), it.status == Status::Running) {
                                        let dt = now.duration_since(*t0).as_secs_f64();
                                        if dt > 0.25 {
                                            let r = (it.downloaded - d0) as f64 / dt;
                                            rates.insert(it.id, r);
                                            total += r;
                                        }
                                    }
                                }
                                history.retain(|id, _| items.iter().any(|i| i.id == *id));
                                Snapshot { items, rates, total_rate: total, connected: true, last_error: None, service: service.clone() }
                            }
                            Err(e) => {
                                token = None; // daemon restarted; its token is new
                                Snapshot { connected: false, last_error: Some(e), service: service.clone(), ..Default::default() }
                            }
                        },
                        None => Snapshot {
                            connected: false,
                            last_error: Some("waiting for the download service…".into()),
                            service: service.clone(),
                            ..Default::default()
                        },
                    };
                    if let Ok(lock) = snapshot.lock() {
                        if let Some(prev_err) = lock.last_error.clone() {
                            // keep a command error visible for one poll cycle
                            snap.last_error = snap.last_error.or(Some(prev_err));
                        }
                    }
                    *snapshot.lock().unwrap() = snap.clone();
                    let _ = tx.send(snap);
                    std::thread::sleep(Duration::from_millis(300));
                }
            })
        };

        Api { agent: agent(Duration::from_secs(10)), base, token: Mutex::new(None), snapshot, rx, _poller: poller }
    }

    /// Non-blocking: returns the newest snapshot if the poller produced one.
    pub fn try_recv(&self) -> Option<Snapshot> {
        let mut last = None;
        while let Ok(s) = self.rx.try_recv() {
            last = Some(s);
        }
        last
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().unwrap().clone()
    }

    fn token(&self) -> Option<String> {
        let mut t = self.token.lock().unwrap();
        if t.is_none() {
            *t = read_token();
        }
        t.clone()
    }

    fn note_error(&self, e: String) {
        if let Ok(mut s) = self.snapshot.lock() {
            s.last_error = Some(e);
        }
    }

    /// POST JSON; `Ok(body)` on success, the daemon's explanation otherwise.
    fn post(&self, path: &str, body: serde_json::Value) -> Result<String, String> {
        let t = self.token().ok_or("not connected to the download service")?;
        let mut r = self
            .agent
            .post(format!("{}{path}", self.base))
            .header("X-Reidm-Token", &t)
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(|e| e.to_string())?;
        let ok = r.status().is_success();
        let text = r.body_mut().read_to_string().unwrap_or_default();
        if ok {
            Ok(text)
        } else {
            Err(if text.trim().is_empty() { format!("HTTP {}", r.status()) } else { text })
        }
    }

    /// Queue one download. `later` adds it to the scheduled queue without starting it.
    pub fn add(&self, url: &str, filename: Option<String>, dir: Option<String>, later: bool) -> Result<(), String> {
        let body = serde_json::json!({ "url": url, "filename": filename, "dir": dir, "later": later });
        self.post("/api/downloads", body).map(|_| ())
    }

    pub fn action(&self, id: u64, what: &str) {
        let Some(t) = self.token() else { return };
        let r = self
            .agent
            .post(format!("{}/api/downloads/{id}/{what}", self.base))
            .header("X-Reidm-Token", &t)
            .send("");
        match r {
            Ok(mut r) if !r.status().is_success() => {
                let text = r.body_mut().read_to_string().unwrap_or_default();
                self.note_error(format!("{what} failed: {text}"));
            }
            Err(e) => self.note_error(format!("{what} failed: {e}")),
            _ => {}
        }
    }

    /// Like [`Api::action`], but reports the daemon's answer.
    pub fn try_action(&self, id: u64, what: &str) -> Result<(), String> {
        self.post(&format!("/api/downloads/{id}/{what}"), serde_json::json!({})).map(|_| ())
    }

    pub fn remove(&self, id: u64, purge: bool) {
        let Some(t) = self.token() else { return };
        let url = format!("{}/api/downloads/{id}?purge={purge}", self.base);
        let r = self.agent.delete(url).header("X-Reidm-Token", &t).call();
        if let Err(e) = r {
            self.note_error(format!("remove failed: {e}"));
        }
    }

    /// The daemon's settings, fetched now.
    pub fn fetch_settings(&self) -> Option<Settings> {
        let t = self.token()?;
        let mut r = self.agent.get(format!("{}/api/settings", self.base)).header("X-Reidm-Token", &t).call().ok()?;
        serde_json::from_str(&r.body_mut().read_to_string().ok()?).ok()
    }

    /// A partial settings update: only the fields present in `patch` change.
    pub fn update_settings(&self, patch: serde_json::Value) -> Result<(), String> {
        self.post("/api/settings", patch).map(|_| ())
    }

    /// Queue many URLs. Returns one line per URL describing what happened.
    pub fn add_batch(&self, urls: &[String], later: bool) -> Result<Vec<BatchResult>, String> {
        self.batch(serde_json::json!({ "urls": urls, "later": later }))
    }

    /// Queue entries that carry their own referer/cookie/user agent (an `.ef2` import).
    pub fn add_entries(&self, entries: &[reidm_core::lists::Entry], later: bool) -> Result<Vec<BatchResult>, String> {
        self.batch(serde_json::json!({ "items": entries, "later": later }))
    }

    fn batch(&self, body: serde_json::Value) -> Result<Vec<BatchResult>, String> {
        let text = self.post("/api/downloads/batch", body)?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
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

    /// Cap one download (bytes per second), or lift its cap with `None`.
    pub fn set_limit(&self, id: u64, limit: Option<u64>) -> Result<(), String> {
        self.post(&format!("/api/downloads/{id}/limit"), serde_json::json!({ "limit": limit })).map(|_| ())
    }

    /// Start or stop the scheduled queue; returns how many downloads it touched.
    pub fn queue(&self, start: bool) -> Result<u64, String> {
        let text = self.post(&format!("/api/queue/{}", if start { "start" } else { "stop" }), serde_json::json!({}))?;
        Ok(serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["count"].as_u64()).unwrap_or(0))
    }

    pub fn cancel_refresh(&self) {
        let _ = self.post("/api/refresh-capture/cancel", serde_json::json!({}));
    }

    /// The whole list as `ef2` or `txt`.
    pub fn export(&self, format: &str) -> Result<String, String> {
        let t = self.token().ok_or("not connected to the download service")?;
        let mut r = self
            .agent
            .get(format!("{}/api/export?format={format}", self.base))
            .header("X-Reidm-Token", &t)
            .call()
            .map_err(|e| e.to_string())?;
        let ok = r.status().is_success();
        let text = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
        if ok {
            Ok(text)
        } else {
            Err(text)
        }
    }

    /// Point a download at a new address. It checks the new address first, which takes a
    /// network round trip, so the answer arrives over a channel.
    pub fn refresh_async(&self, id: u64, url: &str) -> Receiver<Result<(), String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let (base, token, url) = (self.base.clone(), self.token(), url.to_string());
        std::thread::spawn(move || {
            let r = (|| {
                let t = token.ok_or("not connected to the download service")?;
                let mut r = agent(Duration::from_secs(60))
                    .post(format!("{base}/api/downloads/{id}/refresh"))
                    .header("X-Reidm-Token", &t)
                    .header("Content-Type", "application/json")
                    .send(serde_json::json!({ "url": url }).to_string())
                    .map_err(|e| e.to_string())?;
                if r.status().is_success() {
                    Ok(())
                } else {
                    Err(r.body_mut().read_to_string().unwrap_or_default())
                }
            })();
            let _ = tx.send(r);
        });
        rx
    }

    /// Scan a page on a worker thread. A scan is network-bound and can take many seconds, so
    /// the result arrives over a channel rather than blocking a frame.
    pub fn grab_async(&self, url: &str, opts: &crate::model::GrabOptions) -> std::sync::mpsc::Receiver<Result<Vec<crate::model::Found>, String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let base = self.base.clone();
        let token = self.token();
        let url = url.to_string();
        let opts = opts.clone();
        std::thread::spawn(move || {
            let _ = tx.send(grab_with(&agent(Duration::from_secs(180)), &base, token.as_deref(), &url, &opts));
        });
        rx
    }

    /// Open a URL in the user's browser (used for release notes and update downloads).
    pub fn open_url(&self, url: &str) {
        open_external(url);
    }

    /// Pause every active download.
    pub fn stop_all(&self) {
        for it in self.snapshot().items.iter().filter(|i| i.status.is_active()) {
            self.action(it.id, "pause");
        }
    }
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
    let _ = c
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn grab_with(
    agent: &ureq::Agent,
    base: &str,
    token: Option<&str>,
    url: &str,
    opts: &crate::model::GrabOptions,
) -> Result<Vec<crate::model::Found>, String> {
    let t = token.ok_or("not connected to the download service")?;
    let body = serde_json::json!({ "url": url, "options": opts });
    let mut r = agent
        .post(format!("{base}/api/grab"))
        .header("X-Reidm-Token", t)
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|e| e.to_string())?;
    let status = r.status();
    let text = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(text);
    }
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    serde_json::from_value(v["files"].clone()).map_err(|e| e.to_string())
}

/// Status and settings in one go.
fn fetch_service(agent: &ureq::Agent, base: &str, token: &str) -> Option<Service> {
    let get = |path: &str| -> Option<serde_json::Value> {
        let mut r = agent.get(format!("{base}{path}")).header("X-Reidm-Token", token).call().ok()?;
        if !r.status().is_success() {
            return None;
        }
        serde_json::from_str(&r.body_mut().read_to_string().ok()?).ok()
    };
    let v = get("/api/status")?;
    Some(Service {
        idm_port: v["idm_port"].as_bool().unwrap_or(false),
        platform: v["platform"].as_str().unwrap_or_default().to_string(),
        version: v["version"].as_str().unwrap_or_default().to_string(),
        extension: serde_json::from_value(v["extension"].clone()).ok(),
        refresh_armed: v["refresh_armed"].as_u64(),
        extension_dir: v["extension_dir"].as_str().map(str::to_string),
        settings: get("/api/settings").and_then(|s| serde_json::from_value(s).ok()),
    })
}

fn fetch(agent: &ureq::Agent, base: &str, token: &str) -> Result<Vec<Item>, String> {
    let mut r = agent
        .get(format!("{base}/api/downloads"))
        .header("X-Reidm-Token", token)
        .call()
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("HTTP {}", r.status()));
    }
    let body = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}
