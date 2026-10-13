//! Segmented, resumable download engine used by the reIDM daemon and CLI.

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

const MIN_SPLIT: u64 = 1 << 20; // don't split files under 1 MiB
const MAX_RETRIES: u32 = 5;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", content = "error")]
pub enum Status {
    Queued,
    Running,
    Paused,
    Done,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Segment {
    start: u64,
    end: u64, // inclusive; u64::MAX = length unknown
    done: u64,
    #[serde(default)]
    finished: bool,
}

impl Segment {
    fn complete(&self) -> bool {
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
}

struct Live {
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
}

#[derive(Serialize, Deserialize, Default)]
struct Db {
    next_id: u64,
    items: BTreeMap<u64, Item>,
}

struct Inner {
    db: Mutex<Db>,
    live: Mutex<BTreeMap<u64, Live>>,
    db_path: PathBuf,
    default_dir: PathBuf,
    connections: usize,
    http: reqwest::Client,
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

/// Strip path separators and control characters so a hostile server/page cannot escape `dir`.
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() { "download".into() } else { cleaned }
}

fn filename_from_disposition(v: &str) -> Option<String> {
    // RFC 6266: prefer filename*=UTF-8''..., then filename="..."
    for part in v.split(';').map(str::trim) {
        if let Some(rest) = part.strip_prefix("filename*=") {
            let rest = rest.trim_matches('"');
            if let Some((_, enc)) = rest.split_once("''") {
                return Some(percent_encoding::percent_decode_str(enc).decode_utf8_lossy().into_owned());
            }
        }
    }
    for part in v.split(';').map(str::trim) {
        if let Some(rest) = part.strip_prefix("filename=") {
            return Some(rest.trim_matches('"').to_string());
        }
    }
    None
}

fn filename_from_url(u: &url::Url) -> String {
    let seg = u.path_segments().and_then(|mut s| s.next_back()).unwrap_or("");
    let decoded = percent_encoding::percent_decode_str(seg).decode_utf8_lossy().into_owned();
    if decoded.is_empty() { "download".into() } else { decoded }
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let mut p = dir.join(name);
    if !p.exists() && !part_path(&p).exists() {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for n in 1.. {
        p = dir.join(format!("{stem} ({n}){ext}"));
        if !p.exists() && !part_path(&p).exists() {
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

impl Manager {
    pub fn open(data_dir: &Path, default_dir: PathBuf, connections: usize) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("downloads.json");
        let mut db: Db = match std::fs::read(&db_path) {
            Ok(b) => serde_json::from_slice(&b).context("corrupt downloads.json")?,
            Err(_) => Db { next_id: 1, ..Default::default() },
        };
        if db.next_id == 0 {
            db.next_id = 1;
        }
        // anything that was running when we last exited is now paused
        for it in db.items.values_mut() {
            if it.status == Status::Running || it.status == Status::Queued {
                it.status = Status::Paused;
            }
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?;
        Ok(Self(Arc::new(Inner {
            db: Mutex::new(db),
            live: Mutex::new(BTreeMap::new()),
            db_path,
            default_dir,
            connections: connections.clamp(1, 32),
            http,
        })))
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

    pub fn list(&self) -> Vec<Item> {
        let live = self.0.live.lock().unwrap();
        let db = self.0.db.lock().unwrap();
        db.items
            .values()
            .cloned()
            .map(|mut it| {
                if let Some(l) = live.get(&it.id) {
                    it.downloaded = l.progress.load(Ordering::Relaxed);
                }
                it
            })
            .collect()
    }

    pub fn get(&self, id: u64) -> Option<Item> {
        self.list().into_iter().find(|i| i.id == id)
    }

    /// Queue a download and start it immediately. Returns the new id.
    pub fn add(&self, req: Request) -> Result<u64> {
        let u = url::Url::parse(&req.url).context("invalid URL")?;
        if !matches!(u.scheme(), "http" | "https") {
            bail!("unsupported scheme {:?} (only http/https)", u.scheme());
        }
        let dir = req.dir.clone().unwrap_or_else(|| self.0.default_dir.clone());
        std::fs::create_dir_all(&dir)?;
        let id = {
            let mut db = self.0.db.lock().unwrap();
            let id = db.next_id;
            db.next_id += 1;
            let hint = req.filename.as_deref().map(sanitize_filename).unwrap_or_else(|| sanitize_filename(&filename_from_url(&u)));
            let path = unique_path(&dir, &hint);
            db.items.insert(
                id,
                Item { id, request: req, path, size: None, downloaded: 0, status: Status::Queued, segments: vec![], resumable: false },
            );
            id
        };
        self.save();
        self.start(id);
        Ok(id)
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
        self.start(id);
        Ok(())
    }

    pub fn pause(&self, id: u64) {
        if let Some(l) = self.0.live.lock().unwrap().get(&id) {
            l.cancel.store(true, Ordering::SeqCst);
        }
    }

    pub fn remove(&self, id: u64, delete_files: bool) {
        self.pause(id);
        let removed = self.0.db.lock().unwrap().items.remove(&id);
        if let (Some(it), true) = (removed, delete_files) {
            let _ = std::fs::remove_file(part_path(&it.path));
            if it.status != Status::Done {
                let _ = std::fs::remove_file(&it.path);
            }
        }
        self.save();
    }

    fn start(&self, id: u64) {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(self.0.db.lock().unwrap().items.get(&id).map_or(0, |i| i.downloaded)));
        self.0.live.lock().unwrap().insert(id, Live { cancel: cancel.clone(), progress: progress.clone() });
        let me = self.clone();
        tokio::spawn(async move {
            me.set_status(id, Status::Running);
            let res = me.run(id, cancel.clone(), progress.clone()).await;
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
                    it.downloaded = progress.load(Ordering::Relaxed);
                    it.status = st;
                }
            }
            me.0.live.lock().unwrap().remove(&id);
            me.save();
        });
    }

    fn set_status(&self, id: u64, st: Status) {
        if let Some(it) = self.0.db.lock().unwrap().items.get_mut(&id) {
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

    /// Returns Ok(true) when finished, Ok(false) when paused/cancelled.
    async fn run(&self, id: u64, cancel: Arc<AtomicBool>, progress: Arc<AtomicU64>) -> Result<bool> {
        let mut item = self.get(id).ok_or_else(|| anyhow!("vanished"))?;
        let headers = Self::headers(&item.request);

        if item.segments.is_empty() {
            // Probe: a 1-byte ranged GET tells us size, range support and name in one request.
            let r = self
                .0
                .http
                .get(&item.request.url)
                .headers(headers.clone())
                .header(header::RANGE, "bytes=0-0")
                .send()
                .await?
                .error_for_status()?;
            let partial = r.status() == reqwest::StatusCode::PARTIAL_CONTENT;
            let size = if partial {
                r.headers()
                    .get(header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.rsplit('/').next())
                    .and_then(|n| n.parse::<u64>().ok())
            } else {
                r.content_length()
            };
            if item.request.filename.is_none() {
                if let Some(n) = r
                    .headers()
                    .get(header::CONTENT_DISPOSITION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(filename_from_disposition)
                {
                    let dir = item.path.parent().unwrap().to_path_buf();
                    item.path = unique_path(&dir, &sanitize_filename(&n));
                }
            }
            drop(r);
            item.size = size;
            item.resumable = partial && size.is_some();
            item.segments = match (size, item.resumable) {
                (Some(sz), true) if sz >= MIN_SPLIT => {
                    let n = self.0.connections as u64;
                    let chunk = sz.div_ceil(n);
                    (0..n)
                        .map(|i| Segment { start: i * chunk, end: ((i + 1) * chunk - 1).min(sz - 1), done: 0, finished: false })
                        .filter(|s| s.start < sz)
                        .collect()
                }
                (Some(sz), _) => vec![Segment { start: 0, end: sz.saturating_sub(1), done: 0, finished: false }],
                (None, _) => vec![Segment { start: 0, end: u64::MAX, done: 0, finished: false }],
            };
            self.0.db.lock().unwrap().items.insert(id, item.clone());
            self.save();
        }

        let part = part_path(&item.path);
        {
            let f = tokio::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&part).await?;
            if let Some(sz) = item.size {
                f.set_len(sz).await?;
            }
        }
        progress.store(item.segments.iter().map(|s| s.done).sum(), Ordering::Relaxed);

        let shared = Arc::new(Mutex::new(item.segments.clone()));
        let mut tasks = Vec::new();
        for idx in 0..item.segments.len() {
            let (me, shared, cancel, progress, part, req, headers) =
                (self.clone(), shared.clone(), cancel.clone(), progress.clone(), part.clone(), item.request.clone(), headers.clone());
            let resumable = item.resumable;
            tasks.push(tokio::spawn(async move { me.fetch_segment(idx, shared, cancel, progress, part, req, headers, resumable).await }));
        }
        // periodic checkpoint so a crash loses little
        let ticker = {
            let (me, shared, progress) = (self.clone(), shared.clone(), progress.clone());
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    me.checkpoint(id, &shared, &progress);
                }
            })
        };
        let mut first_err = None;
        for t in tasks {
            if let Err(e) = t.await.map_err(|e| anyhow!(e)).and_then(|r| r) {
                first_err.get_or_insert(e);
                cancel.store(true, Ordering::SeqCst); // stop siblings
            }
        }
        ticker.abort();
        self.checkpoint(id, &shared, &progress);
        if let Some(e) = first_err {
            return Err(e);
        }
        let complete = shared.lock().unwrap().iter().all(Segment::complete);
        if cancel.load(Ordering::SeqCst) || !complete {
            return Ok(false);
        }
        tokio::fs::rename(&part, &item.path).await?;
        Ok(true)
    }

    fn checkpoint(&self, id: u64, shared: &Arc<Mutex<Vec<Segment>>>, progress: &AtomicU64) {
        if let Some(it) = self.0.db.lock().unwrap().items.get_mut(&id) {
            it.segments = shared.lock().unwrap().clone();
            it.downloaded = progress.load(Ordering::Relaxed);
        }
        self.save();
    }

    #[allow(clippy::too_many_arguments)]
    async fn fetch_segment(
        &self,
        idx: usize,
        shared: Arc<Mutex<Vec<Segment>>>,
        cancel: Arc<AtomicBool>,
        progress: Arc<AtomicU64>,
        part: PathBuf,
        req: Request,
        headers: HeaderMap,
        resumable: bool,
    ) -> Result<()> {
        let mut attempt = 0;
        loop {
            let seg = shared.lock().unwrap()[idx].clone();
            let open_ended = seg.end == u64::MAX;
            if seg.complete() {
                return Ok(());
            }
            let from = seg.start + seg.done;
            let mut rb = self.0.http.get(&req.url).headers(headers.clone());
            if resumable {
                rb = rb.header(header::RANGE, format!("bytes={from}-{}", seg.end));
            }
            let outcome: Result<()> = async {
                let resp = rb.send().await?.error_for_status()?;
                if resumable && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                    bail!("server stopped honouring Range (HTTP {})", resp.status());
                }
                let mut f = tokio::fs::OpenOptions::new().write(true).open(&part).await?;
                f.seek(std::io::SeekFrom::Start(if resumable { from } else { 0 })).await?;
                let mut stream = resp.bytes_stream();
                let mut remaining = if open_ended { u64::MAX } else { seg.end - from + 1 };
                while let Some(chunk) = stream.next().await {
                    if cancel.load(Ordering::SeqCst) {
                        f.flush().await?;
                        return Ok(());
                    }
                    let chunk = chunk?;
                    let take = (chunk.len() as u64).min(remaining) as usize;
                    f.write_all(&chunk[..take]).await?;
                    remaining -= take as u64;
                    shared.lock().unwrap()[idx].done += take as u64;
                    progress.fetch_add(take as u64, Ordering::Relaxed);
                    if remaining == 0 {
                        break;
                    }
                }
                f.flush().await?;
                if open_ended {
                    shared.lock().unwrap()[idx].finished = true;
                } else if remaining != 0 {
                    bail!("connection closed early ({remaining} bytes missing)");
                }
                Ok(())
            }
            .await;
            match outcome {
                Ok(()) => {
                    if cancel.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    if shared.lock().unwrap()[idx].complete() {
                        return Ok(());
                    }
                }
                Err(e) => {
                    attempt += 1;
                    if attempt > MAX_RETRIES || cancel.load(Ordering::SeqCst) {
                        return Err(e);
                    }
                    tracing::debug!("segment {idx} retry {attempt}: {e:#}");
                    tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
                }
            }
        }
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
    }

    #[test]
    fn disposition() {
        assert_eq!(filename_from_disposition("attachment; filename=\"a b.zip\"").as_deref(), Some("a b.zip"));
        assert_eq!(filename_from_disposition("attachment; filename*=UTF-8''%E2%82%AC.txt").as_deref(), Some("€.txt"));
    }
}
