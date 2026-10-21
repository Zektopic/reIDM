//! The state each dialog keeps between frames.

use std::sync::mpsc::Receiver;
use std::time::Instant;
use crate::model::{Detail, Found};

/// A release the updater found, plus whether the user has dismissed the banner.
pub(super) struct UpdateOffer {
    pub(super) version: String,
    pub(super) url: String,
    pub(super) asset: Option<(String, String)>, // (name, download url)
    pub(super) notes: String,
    pub(super) dismissed: bool,
    pub(super) checking: bool,
    pub(super) error: Option<String>,
}

/// What a file or folder picker was opened for.
pub(super) enum PickFor {
    AddDir,
    DownloadDir,
    CategoryDir(&'static str),
    TempDir,
    Import,
    Export { ef2: bool },
    ExportFound(Vec<String>),
}

/// The Add URL dialog.
#[derive(Default)]
pub(super) struct AddState {
    pub(super) open: bool,
    pub(super) urls: String,
    pub(super) name: String,
    pub(super) dir: String,
    /// The text `count` was worked out for: parsing ten thousand pasted lines on every frame
    /// would make typing crawl.
    pub(super) parsed: String,
    pub(super) count: usize,
    /// *More options › Expected checksum*, as typed.
    pub(super) checksum: String,
}

impl AddState {
    /// How many addresses the text holds.
    pub(super) fn count(&mut self) -> usize {
        if self.parsed != self.urls {
            self.count = reidm_core::grabber::parse_url_list(&self.urls).len();
            self.parsed = self.urls.clone();
        }
        self.count
    }
}

/// The most addresses one batch pattern may make, as the service caps a batch.
pub(super) const MAX_BATCH: u64 = 10_000;

/// What a batch preview depends on: the pattern, from, to and padding.
pub(super) type BatchKey = (String, String, String, usize);

/// IDM's *Add batch download*: an address with one `*` and a range.
pub(super) struct BatchState {
    pub(super) open: bool,
    pub(super) pattern: String,
    pub(super) from: String,
    pub(super) to: String,
    pub(super) width: usize,
    /// The addresses for `(pattern, from, to, width)`, worked out when one of them changes.
    pub(super) preview: Option<(BatchKey, Result<Vec<String>, String>)>,
}

impl Default for BatchState {
    fn default() -> Self {
        Self { open: false, pattern: String::new(), from: "1".into(), to: "10".into(), width: 0, preview: None }
    }
}

impl BatchState {
    /// The addresses the pattern makes, or why it makes none.
    pub(super) fn expand(&mut self) -> Result<Vec<String>, String> {
        let key = (self.pattern.clone(), self.from.clone(), self.to.clone(), self.width);
        if let Some((k, r)) = &self.preview {
            if *k == key {
                return r.clone();
            }
        }
        let r = expand_batch(&self.pattern, &self.from, &self.to, self.width);
        self.preview = Some((key, r.clone()));
        r
    }
}

/// Expand an *Add batch download* pattern. The range is checked before anything is built, so a
/// typo like `0` to `99999999999` is an error at once instead of a window that stops responding.
pub(super) fn expand_batch(pattern: &str, from: &str, to: &str, width: usize) -> Result<Vec<String>, String> {
    use reidm_core::lists::{expand_pattern, Sequence};
    let seq = match (from.trim().parse::<u64>(), to.trim().parse::<u64>()) {
        (Ok(from), Ok(to)) => {
            let n = to.checked_sub(from).and_then(|d| d.checked_add(1)).ok_or("To must not be less than From")?;
            if n > MAX_BATCH {
                return Err(format!("That is {n} addresses; one batch can hold at most {MAX_BATCH}"));
            }
            Sequence::Numbers { from, to, width: width.min(20) }
        }
        _ => {
            let one = |s: &str| {
                let mut c = s.trim().chars();
                match (c.next(), c.next()) {
                    (Some(ch), None) if ch.is_ascii_alphabetic() => Some(ch),
                    _ => None,
                }
            };
            match (one(from), one(to)) {
                (Some(from), Some(to)) => Sequence::Letters { from, to },
                _ => return Err("From and To must both be numbers, or both be single letters".to_string()),
            }
        }
    };
    expand_pattern(pattern, &seq).map_err(|e| e.to_string())
}

/// *Refresh download address* for one download.
pub(super) struct RefreshState {
    pub(super) id: u64,
    pub(super) url: String,
    pub(super) busy: Option<Receiver<Result<(), String>>>,
    pub(super) error: Option<String>,
    /// Waiting for the browser to supply the address.
    pub(super) capturing: bool,
    /// The address when capturing started, to tell a refresh from a timeout.
    pub(super) old_url: String,
}

/// The scheduler dialog's edit state.
#[derive(Default)]
pub(super) struct SchedEdit {
    pub(super) open: bool,
    pub(super) enabled: bool,
    pub(super) start: String,
    pub(super) use_stop: bool,
    pub(super) stop: String,
    pub(super) days: [bool; 7],
    pub(super) error: Option<String>,
}

#[derive(Default)]
pub(super) struct LimiterEdit {
    pub(super) open: bool,
    pub(super) on: bool,
    pub(super) kb: u32,
}

/// The properties window of one download.
pub(super) struct PropsState {
    pub(super) id: u64,
    pub(super) limit_on: bool,
    pub(super) limit_kb: u32,
    /// Every field of the download (its segments, a torrent's files), which list rows leave
    /// out; refreshed about once a second while the window is open.
    pub(super) detail: Option<Detail>,
    pub(super) fetched: Option<Instant>,
    pub(super) fetching: bool,
    /// *Integrity*: the algorithm to compute, a value to compare with, and the computation
    /// running or its result.
    pub(super) algo: crate::checksum::Algo,
    pub(super) expected: String,
    pub(super) verifying: Option<Receiver<Result<String, String>>>,
    pub(super) verified: Option<(crate::checksum::Algo, Result<String, String>)>,
}

/// The site-grabber window's state. Every field defaults to empty/off.
#[derive(Default)]
pub(super) struct GrabState {
    pub(super) url: String,
    pub(super) depth: u32,
    /// Comma-separated; empty means the built-in list of common file types.
    pub(super) types: String,
    pub(super) same_host: bool,
    pub(super) include: String,
    pub(super) exclude: String,
    pub(super) sizes: bool,
    pub(super) scanning: bool,
    pub(super) error: Option<String>,
    pub(super) found: Vec<Found>,
    /// Parallel to `found`.
    pub(super) selected: Vec<bool>,
    /// A scan runs on its own thread because it is network-bound and can take a while.
    pub(super) rx: Option<Receiver<Result<Vec<Found>, String>>>,
}
