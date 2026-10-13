//! Client for the local reIDM daemon.
//!
//! The GUI is a *front-end*: the daemon owns the download engine and the browser-extension
//! ports, so the GUI can be closed and downloads keep running. The GUI keeps a snapshot fresh on a
//! background thread and sends commands over the same JSON API the web page uses.

use crate::model::Item;
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::model::Status;

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
    /// Service capabilities, refreshed occasionally.
    pub idm_port: bool,
    pub platform: String,
}

struct Sampled {
    downloaded: u64,
    at: Instant,
}

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

impl Api {
    pub fn start() -> Api {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .new_agent();
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let (tx, rx) = std::sync::mpsc::channel();

        let base = format!("http://127.0.0.1:{UI_PORT}");
        let poller = {
            let snapshot = snapshot.clone();
            let mut token = None;
            let base2 = base.clone();
            std::thread::spawn(move || {
                let agent = ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(5)))
                    .build()
                    .new_agent();
                let mut prev: std::collections::HashMap<u64, Sampled> = Default::default();
                let mut caps = (false, String::new());
                let mut ticks: u64 = 0;
                loop {
                    if token.is_none() {
                        token = read_token();
                    }
                    // Capabilities change rarely, so refresh them about every 5 seconds.
                    if ticks % 16 == 0 {
                        if let Some(t) = token.as_deref() {
                            if let Some(c) = fetch_status(&agent, &base2, t) {
                                caps = c;
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
                                    if it.status == Status::Running {
                                        if let Some(p) = prev.get(&it.id) {
                                            let dt = now.duration_since(p.at).as_secs_f64();
                                            if dt > 0.05 && it.downloaded >= p.downloaded {
                                                let r = (it.downloaded - p.downloaded) as f64 / dt;
                                                rates.insert(it.id, r);
                                                total += r;
                                            }
                                        }
                                    }
                                    prev.insert(it.id, Sampled { downloaded: it.downloaded, at: now });
                                }
                                prev.retain(|id, _| items.iter().any(|i| i.id == *id));
                                Snapshot {
                                    items,
                                    rates,
                                    total_rate: total,
                                    connected: true,
                                    last_error: None,
                                    idm_port: caps.0,
                                    platform: caps.1.clone(),
                                }
                            }
                            Err(e) => {
                                token = None; // daemon restarted; its token is new
                                Snapshot {
                                    connected: false,
                                    last_error: Some(e),
                                    idm_port: caps.0,
                                    platform: caps.1.clone(),
                                    ..Default::default()
                                }
                            }
                        },
                        None => Snapshot {
                            connected: false,
                            last_error: Some("waiting for the download service…".into()),
                            idm_port: caps.0,
                            platform: caps.1.clone(),
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

        Api { agent, base, token: Mutex::new(None), snapshot, rx, _poller: poller }
    }

    /// Non-blocking: returns the newest snapshot if the poller produced one.
    pub fn try_recv(&self) -> Option<Snapshot> {
        let mut last = None;
        loop {
            match self.rx.try_recv() {
                Ok(s) => last = Some(s),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
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

    pub fn add(&self, url: &str, filename: Option<String>) {
        let Some(t) = self.token() else { return self.note_error("not connected to the download service".into()) };
        let body = serde_json::json!({ "url": url, "filename": filename });
        let r = self
            .agent
            .post(format!("{}/api/downloads", self.base))
            .header("X-Reidm-Token", &t)
            .header("Content-Type", "application/json")
            .send(body.to_string());
        if let Err(e) = r {
            self.note_error(format!("could not add the download: {e}"));
        }
    }

    pub fn action(&self, id: u64, what: &str) {
        let Some(t) = self.token() else { return };
        let r = self
            .agent
            .post(format!("{}/api/downloads/{id}/{what}", self.base))
            .header("X-Reidm-Token", &t)
            .send("");
        if let Err(e) = r {
            self.note_error(format!("{what} failed: {e}"));
        }
    }

    pub fn remove(&self, id: u64, purge: bool) {
        let Some(t) = self.token() else { return };
        let url = format!("{}/api/downloads/{id}?purge={purge}", self.base);
        let r = self.agent.delete(url).header("X-Reidm-Token", &t).call();
        if let Err(e) = r {
            self.note_error(format!("remove failed: {e}"));
        }
    }

    /// `(download folder, connections per download)`, or `None` if the daemon is unreachable.
    pub fn fetch_settings(&self) -> Option<(String, usize)> {
        let t = self.token()?;
        let mut r = self.agent.get(format!("{}/api/settings", self.base)).header("X-Reidm-Token", &t).call().ok()?;
        let v: serde_json::Value = serde_json::from_str(&r.body_mut().read_to_string().ok()?).ok()?;
        Some((v["default_dir"].as_str()?.to_string(), v["connections"].as_u64()? as usize))
    }

    pub fn set_settings(&self, dir: &str, connections: usize) {
        let Some(t) = self.token() else { return };
        let body = serde_json::json!({ "default_dir": dir, "connections": connections });
        if let Err(e) = self
            .agent
            .post(format!("{}/api/settings", self.base))
            .header("X-Reidm-Token", &t)
            .header("Content-Type", "application/json")
            .send(body.to_string())
        {
            self.note_error(format!("could not save options: {e}"));
        }
    }

    /// Pause every active download.
    pub fn stop_all(&self) {
        for it in self.snapshot().items.iter().filter(|i| i.status.is_active()) {
            self.action(it.id, "pause");
        }
    }
}

/// `(owns the extension's default port, platform)`.
fn fetch_status(agent: &ureq::Agent, base: &str, token: &str) -> Option<(bool, String)> {
    let mut r = agent.get(format!("{base}/api/status")).header("X-Reidm-Token", token).call().ok()?;
    let v: serde_json::Value = serde_json::from_str(&r.body_mut().read_to_string().ok()?).ok()?;
    Some((v["idm_port"].as_bool()?, v["platform"].as_str().unwrap_or_default().to_string()))
}

fn fetch(agent: &ureq::Agent, base: &str, token: &str) -> Result<Vec<Item>, String> {
    let mut r = agent
        .get(format!("{base}/api/downloads"))
        .header("X-Reidm-Token", token)
        .call()
        .map_err(|e| e.to_string())?;
    let body = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}
