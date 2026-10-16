//! Mirrors the daemon's `downloads.json` / `/api/downloads` JSON.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub url: String,
    #[serde(default)]
    pub referer: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    /// A video fetched through yt-dlp: `url` is then the page, and this the format asked for.
    #[serde(default)]
    pub media: Option<Media>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Media {
    pub format: String,
}

/// One byte range of a download and how far it has got.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Segment {
    pub start: u64,
    /// Inclusive; `u64::MAX` when the length is unknown.
    pub end: u64,
    pub done: u64,
    #[serde(default)]
    pub finished: bool,
    /// A connection is working on it right now.
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", content = "error")]
pub enum Status {
    Queued,
    Running,
    Paused,
    Done,
    Failed(String),
}

impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Status::Queued => "Queued",
            Status::Running => "Downloading",
            Status::Paused => "Paused",
            Status::Done => "Complete",
            Status::Failed(_) => "Error",
        }
    }

    /// Only these states can be stopped or started.
    pub fn is_active(&self) -> bool {
        matches!(self, Status::Running | Status::Queued)
    }
    pub fn is_stopped(&self) -> bool {
        matches!(self, Status::Paused | Status::Failed(_))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    pub id: u64,
    pub request: Request,
    pub path: PathBuf,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub downloaded: u64,
    pub status: Status,
    #[serde(default)]
    pub added: u64,
    #[serde(default)]
    pub last_try: Option<u64>,
    #[serde(default)]
    pub finished_at: Option<u64>,
    #[serde(default)]
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub resumable: bool,
    /// This download's own speed cap, bytes per second.
    #[serde(default)]
    pub speed_limit: Option<u64>,
    /// In the scheduled queue (*Download Later*).
    #[serde(default)]
    pub scheduled: bool,
    /// What a running download is busy with ("looking up the video"), when progress can't say.
    #[serde(default)]
    pub note: Option<String>,
    /// A torrent's files, peers, and what it has shared.
    #[serde(default)]
    pub torrent: Option<reidm_core::TorrentStatus>,
}

impl Item {
    /// Uploaded divided by size: how much of its own size a torrent has shared.
    pub fn ratio(&self) -> Option<f64> {
        let t = self.torrent.as_ref()?;
        let size = self.size.filter(|s| *s > 0)?;
        Some(t.uploaded as f64 / size as f64)
    }

    /// A finished torrent that is sharing.
    pub fn seeding(&self) -> bool {
        self.status == Status::Done && self.torrent.as_ref().is_some_and(|t| t.seeding)
    }
}

impl Item {
    /// Connections transferring right now.
    pub fn connections(&self) -> usize {
        self.segments.iter().filter(|s| s.active).count()
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }

    /// 0.0..=1.0, or `None` when the server never told us a size.
    pub fn fraction(&self) -> Option<f32> {
        match (self.size, self.status.clone()) {
            (Some(_), Status::Done) => Some(1.0),
            (Some(sz), _) if sz > 0 => Some((self.downloaded as f32 / sz as f32).clamp(0.0, 1.0)),
            _ => None,
        }
    }

    /// The category this file belongs to, from its extension.
    pub fn category(&self) -> Category {
        let ext = self
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_uppercase())
            .unwrap_or_default();
        Category::of(&ext)
    }
}

/// IDM groups downloads into a fixed set of categories; reIDM uses the same idea and the same
/// extension vocabulary the browser extension sends in its type-4 settings message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    All,
    Video,
    Music,
    Images,
    Documents,
    Compressed,
    Programs,
    Other,
}

pub const CATEGORIES: [Category; 8] = [
    Category::All,
    Category::Video,
    Category::Music,
    Category::Images,
    Category::Documents,
    Category::Compressed,
    Category::Programs,
    Category::Other,
];

impl Category {
    /// By extension, as the download service files it (`reidm_core::Category`).
    pub fn of(ext: &str) -> Category {
        use reidm_core::Category as C;
        match C::of(&format!("x.{ext}")) {
            C::Video => Category::Video,
            C::Music => Category::Music,
            C::Images => Category::Images,
            C::Documents => Category::Documents,
            C::Compressed => Category::Compressed,
            C::Programs => Category::Programs,
            C::General => Category::Other,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Category::All => "All Downloads",
            Category::Video => "Video",
            Category::Music => "Music",
            Category::Images => "Images",
            Category::Documents => "Documents",
            Category::Compressed => "Compressed",
            Category::Programs => "Programs",
            Category::Other => "Other",
        }
    }

    pub fn matches(&self, item: &Item) -> bool {
        *self == Category::All || item.category() == *self
    }

}

/// What the category tree selects: a file-type category, or one of IDM's status nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Category(Category),
    Unfinished,
    Finished,
    /// The scheduled queue: everything added with *Download Later*.
    Queue,
}

impl View {
    pub fn matches(&self, it: &Item) -> bool {
        match self {
            View::Category(c) => c.matches(it),
            View::Unfinished => it.status != Status::Done,
            View::Finished => it.status == Status::Done,
            View::Queue => it.scheduled && it.status != Status::Done,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            View::Category(c) => c.label(),
            View::Unfinished => "Unfinished",
            View::Finished => "Finished",
            View::Queue => "Scheduled queue",
        }
    }
}

/// The daemon's schedule (`/api/settings`), mirrored.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct Schedule {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub stop: Option<String>,
    /// Monday = 0 … Sunday = 6; empty means every day.
    #[serde(default)]
    pub days: Vec<u8>,
}

/// The daemon's settings, mirrored.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub default_dir: String,
    #[serde(default)]
    pub connections: usize,
    #[serde(default)]
    pub max_concurrent: usize,
    #[serde(default)]
    pub speed_limit: u64,
    #[serde(default)]
    pub limit_enabled: bool,
    #[serde(default)]
    pub schedule: Schedule,
    #[serde(default)]
    pub folders: Folders,
    #[serde(default)]
    pub seed_ratio: f64,
    #[serde(default)]
    pub upload_limit: u64,
    #[serde(default)]
    pub torrent_port: u16,
    #[serde(default)]
    pub torrent_dht: bool,
}

/// Where each type of file goes, as the service reports it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Folders {
    #[serde(default)]
    pub sort: bool,
    /// Category name -> the folder in effect.
    #[serde(default)]
    pub dirs: std::collections::BTreeMap<String, String>,
    /// The categories whose folder the user chose.
    #[serde(default)]
    pub custom: Vec<String>,
    #[serde(default)]
    pub temp_dir: String,
    #[serde(default)]
    pub temp_custom: bool,
}

/// When reIDM's own browser extension last checked in.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExtensionSeen {
    #[serde(default)]
    pub seconds_ago: u64,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub browser: String,
}

/// `1.4 GB`, `812 KB`, … (IDM shows decimal units, so we do too).
pub fn size_human(n: u64) -> String {
    const U: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i < U.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// `1.4 MB/s`
pub fn rate_human(bps: f64) -> String {
    if !bps.is_finite() || bps <= 0.0 {
        return String::new();
    }
    format!("{}/s", size_human(bps as u64))
}

/// Options for a site-grabber scan, mirroring the daemon's.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GrabOptions {
    pub depth: u32,
    pub types: Vec<String>,
    pub same_host_only: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub sizes: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Found {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub ext: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub found_on: String,
}

/// How long ago a unix timestamp was, for the "Last Try" column.
///
/// Relative rather than a clock time: the standard library exposes no local timezone, and
/// "3 min ago" reads the same in every zone.
pub fn since_human(ts: u64, now: u64) -> String {
    if ts == 0 || ts > now {
        return String::new();
    }
    match now - ts {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s if s < 86_400 * 7 => format!("{} d ago", s / 86_400),
        s => format!("{} w ago", s / (86_400 * 7)),
    }
}

/// Seconds since the unix epoch.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `00:01:12`, or empty when unknown. IDM counts down the remaining time.
pub fn eta_human(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return String::new();
    }
    let s = secs as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}
