//! Mirrors the daemon's `downloads.json` / `/api/downloads` JSON.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub url: String,
    #[serde(default)]
    pub referer: Option<String>,
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
}

impl Item {
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
    Documents,
    Compressed,
    Programs,
    Other,
}

pub const CATEGORIES: [Category; 7] = [
    Category::All,
    Category::Video,
    Category::Music,
    Category::Documents,
    Category::Compressed,
    Category::Programs,
    Category::Other,
];

const VIDEO: &[&str] = &["3GP", "ASF", "AVI", "FLV", "M4V", "MKV", "MOV", "MP4", "MPE", "MPEG", "MPG", "OGV", "QT", "RM", "RMVB", "TS", "VOB", "WEBM", "WMV"];
const MUSIC: &[&str] = &["AAC", "AIF", "FLAC", "M4A", "MP3", "MPA", "OGG", "RA", "WAV", "WMA"];
const DOCUMENTS: &[&str] = &["CHM", "DOC", "DOCX", "EPUB", "MOBI", "ODP", "ODS", "ODT", "PDF", "PPS", "PPT", "PPTX", "RTF", "TXT", "XLS", "XLSX"];
const COMPRESSED: &[&str] = &["7Z", "ACE", "ARJ", "BZ2", "GZ", "GZIP", "LZH", "RAR", "TAR", "TGZ", "XZ", "Z", "ZIP"];
const PROGRAMS: &[&str] = &["APK", "APP", "BIN", "DEB", "DMG", "EXE", "IMG", "ISO", "MSI", "MSU", "PKG", "RPM"];

impl Category {
    pub fn of(ext: &str) -> Category {
        if VIDEO.contains(&ext) {
            Category::Video
        } else if MUSIC.contains(&ext) {
            Category::Music
        } else if DOCUMENTS.contains(&ext) {
            Category::Documents
        } else if COMPRESSED.contains(&ext) {
            Category::Compressed
        } else if PROGRAMS.contains(&ext) {
            Category::Programs
        } else {
            Category::Other
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Category::All => "All Downloads",
            Category::Video => "Video",
            Category::Music => "Music",
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

/// `00:01:12`, or empty when unknown. IDM counts down the remaining time.
pub fn eta_human(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return String::new();
    }
    let s = secs as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}
