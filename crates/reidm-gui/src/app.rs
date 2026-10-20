//! The reIDM desktop window.
//!
//! Laid out like the classic IDM main window: a menu bar, a toolbar of icon-over-label
//! buttons, a category tree down the left, the download table in the middle, and a status
//! bar. Every IDM feature the window shows is implemented; anything reIDM does not have is left
//! out rather than shown as a dead button.

use crate::api::{open_external, BatchResult, Client, Pace, Poller, Snapshot};
use crate::icons;
use crate::model::{eta_human, now_secs, rate_human, since_human, size_human, Category, Detail, Found, GrabOptions, Item, Schedule, Status, View, CATEGORIES};
use crate::options::OptionsForm;
use crate::prefs::Prefs;
use crate::theme::{self, Palette};
use crate::view::{SortKey, ViewModel};
use crate::worker::{Reply, Worker};
use egui::{Align, Layout, RichText, Vec2};
use egui_extras::{Column, TableBuilder};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// A reply from the command thread: what to do in the window with a command's result.
fn reply(f: impl FnOnce(&mut App) + Send + 'static) -> Option<Reply<App>> {
    Some(Box::new(f))
}

/// A confirmation the window is waiting for before doing something that can't be undone.
enum Confirm {
    /// Remove these from the list, and maybe delete their files.
    Remove(Vec<u64>),
    /// Start these again from the first byte.
    Restart(Vec<u64>),
    /// Remove every finished download from the list.
    ClearCompleted(Vec<u64>),
    /// The window is closing for the first time: say that downloads carry on.
    Close,
}

struct Toast {
    text: String,
    until: Instant,
}

/// A release the updater found, plus whether the user has dismissed the banner.
struct UpdateOffer {
    version: String,
    url: String,
    asset: Option<(String, String)>, // (name, download url)
    notes: String,
    dismissed: bool,
    checking: bool,
    error: Option<String>,
}

/// The Add URL dialog.
#[derive(Default)]
struct AddState {
    open: bool,
    urls: String,
    name: String,
    dir: String,
    /// The text `count` was worked out for: parsing ten thousand pasted lines on every frame
    /// would make typing crawl.
    parsed: String,
    count: usize,
}

impl AddState {
    /// How many addresses the text holds.
    fn count(&mut self) -> usize {
        if self.parsed != self.urls {
            self.count = reidm_core::grabber::parse_url_list(&self.urls).len();
            self.parsed = self.urls.clone();
        }
        self.count
    }
}

/// The most addresses one batch pattern may make, as the service caps a batch.
const MAX_BATCH: u64 = 10_000;

/// What a batch preview depends on: the pattern, from, to and padding.
type BatchKey = (String, String, String, usize);

/// IDM's *Add batch download*: an address with one `*` and a range.
struct BatchState {
    open: bool,
    pattern: String,
    from: String,
    to: String,
    width: usize,
    /// The addresses for `(pattern, from, to, width)`, worked out when one of them changes.
    preview: Option<(BatchKey, Result<Vec<String>, String>)>,
}

impl Default for BatchState {
    fn default() -> Self {
        Self { open: false, pattern: String::new(), from: "1".into(), to: "10".into(), width: 0, preview: None }
    }
}

impl BatchState {
    /// The addresses the pattern makes, or why it makes none.
    fn expand(&mut self) -> Result<Vec<String>, String> {
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
fn expand_batch(pattern: &str, from: &str, to: &str, width: usize) -> Result<Vec<String>, String> {
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum OptTab {
    General,
    Folders,
    Torrents,
}

/// *Refresh download address* for one download.
struct RefreshState {
    id: u64,
    url: String,
    busy: Option<Receiver<Result<(), String>>>,
    error: Option<String>,
    /// Waiting for the browser to supply the address.
    capturing: bool,
    /// The address when capturing started, to tell a refresh from a timeout.
    old_url: String,
}

/// The scheduler dialog's edit state.
#[derive(Default)]
struct SchedEdit {
    open: bool,
    enabled: bool,
    start: String,
    use_stop: bool,
    stop: String,
    days: [bool; 7],
    error: Option<String>,
}

#[derive(Default)]
struct LimiterEdit {
    open: bool,
    on: bool,
    kb: u32,
}

/// The properties window of one download.
struct PropsState {
    id: u64,
    limit_on: bool,
    limit_kb: u32,
    /// Every field of the download (its segments, a torrent's files), which list rows leave
    /// out; refreshed about once a second while the window is open.
    detail: Option<Detail>,
    fetched: Option<Instant>,
    fetching: bool,
}

pub struct App {
    poller: Poller,
    /// Quick commands, one at a time, off the window's thread.
    cmd: Worker<App>,
    /// For calls that take long enough to need a thread of their own.
    client: Client,
    snap: Snapshot,
    /// The visible rows and the counts, rebuilt only when the list or the view changes.
    vm: ViewModel,
    view: View,
    selected: HashSet<u64>,
    /// The row last clicked: what Properties, Open and Enter act on.
    focus: Option<u64>,
    anchor: Option<u64>,
    search: String,
    sort: SortKey,
    desc: bool,
    prefs: Prefs,
    applied_theme: Option<bool>,
    toast: Option<Toast>,

    // dialogs
    add: AddState,
    batch: BatchState,
    show_options: bool,
    opt_tab: OptTab,
    /// The service's settings as edited in Options.
    opt: OptionsForm,
    /// The settings Options was opened with, to send only what changed; `None` until they have
    /// loaded, and Save is not offered until then.
    opt_base: Option<OptionsForm>,
    confirm: Option<Confirm>,
    about: bool,
    refresh: Option<RefreshState>,
    sched: SchedEdit,
    limiter: LimiterEdit,
    props: Option<PropsState>,
    show_integration: bool,

    // "Download complete" windows, oldest first, and what each download was last seen doing.
    completed: Vec<u64>,
    last_status: HashMap<u64, Status>,
    primed: bool,
    /// The snapshot `note_completions` last looked at.
    noted_rev: u64,

    // updates
    update: Option<UpdateOffer>,
    update_rx: Option<Receiver<Result<Option<reidm_core::update::Release>, String>>>,
    /// IDM's "Categories > All Downloads" node is collapsible; start open.
    cats_open: bool,
    downloads_open: bool,
    /// An update check is kicked off once, a moment after startup.
    checked_on_start: bool,

    // site grabber
    show_grabber: bool,
    playlist: crate::playlist::PlaylistWindow,
    torrent: crate::torrent::TorrentWindow,
    grab: GrabState,
}

/// The site-grabber window's state. Every field defaults to empty/off.
#[derive(Default)]
struct GrabState {
    url: String,
    depth: u32,
    /// Comma-separated; empty means the built-in list of common file types.
    types: String,
    same_host: bool,
    include: String,
    exclude: String,
    sizes: bool,
    scanning: bool,
    error: Option<String>,
    found: Vec<Found>,
    /// Parallel to `found`.
    selected: Vec<bool>,
    /// A scan runs on its own thread because it is network-bound and can take a while.
    rx: Option<Receiver<Result<Vec<Found>, String>>>,
}

/// The text on the clipboard, if any. Read only when the user opens a dialog that uses it.
fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, force_dark: bool) -> Self {
        let mut prefs = Prefs::load();
        if force_dark {
            prefs.dark = true;
        }
        theme::apply(&cc.egui_ctx, prefs.dark);
        let poller = Poller::start(Some(cc.egui_ctx.clone()));
        let cmd = {
            let (ctx, poke) = (cc.egui_ctx.clone(), poller.poke.clone());
            Worker::start(poller.client.clone(), move || {
                // Show the command's effect now rather than at the next poll.
                let _ = poke.send(());
                ctx.request_repaint();
            })
        };
        Self {
            client: poller.client.clone(),
            poller,
            cmd,
            snap: Snapshot::default(),
            vm: ViewModel::default(),
            view: View::Category(Category::All),
            selected: HashSet::new(),
            focus: None,
            anchor: None,
            search: String::new(),
            sort: SortKey::Name,
            desc: false,
            prefs,
            applied_theme: None,
            toast: None,
            add: AddState::default(),
            batch: BatchState::default(),
            show_options: false,
            opt_tab: OptTab::General,
            opt: OptionsForm::from_settings(&Default::default()),
            opt_base: None,
            confirm: None,
            about: false,
            refresh: None,
            sched: SchedEdit::default(),
            limiter: LimiterEdit::default(),
            props: None,
            show_integration: false,
            completed: Vec::new(),
            last_status: HashMap::new(),
            primed: false,
            noted_rev: 0,
            update: None,
            update_rx: None,
            cats_open: true,
            downloads_open: true,
            checked_on_start: false,
            show_grabber: false,
            playlist: Default::default(),
            torrent: Default::default(),
            grab: GrabState::default(),
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), until: Instant::now() + Duration::from_secs(5) });
    }

    /// Waiting for an answer that arrives on a channel the window has to look at.
    fn waiting(&self) -> bool {
        self.update_rx.is_some()
            || self.grab.rx.is_some()
            || self.refresh.as_ref().is_some_and(|r| r.busy.is_some() || r.capturing)
            || self.props.is_some()
            || self.playlist.busy()
            || self.torrent.busy()
    }

    // ── update checking ──────────────────────────────────────────────────

    /// Kick off a check on a worker thread: it talks to the network and must not block a frame.
    fn check_for_updates(&mut self) {
        if self.update_rx.is_some() {
            return;
        }
        if let Some(u) = &mut self.update {
            u.checking = true;
            u.error = None;
        } else {
            self.update = Some(UpdateOffer {
                version: String::new(),
                url: String::new(),
                asset: None,
                notes: String::new(),
                dismissed: false,
                checking: true,
                error: None,
            });
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.update_rx = Some(rx);
        std::thread::spawn(move || {
            let token = reidm_core::update::token_from_env();
            let r = reidm_core::update::check(reidm_core::update::REPO, env!("CARGO_PKG_VERSION"), token.as_deref())
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(r);
        });
    }

    fn poll_update(&mut self) {
        let Some(rx) = &self.update_rx else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.update_rx = None;
        match result {
            Ok(Some(rel)) => {
                let asset = rel.asset_for(std::env::consts::OS).map(|a| (a.name.clone(), a.browser_download_url.clone()));
                if let Some(u) = &mut self.update {
                    u.version = rel.version().to_string();
                    u.url = rel.html_url.clone();
                    u.asset = asset;
                    u.notes = rel.body.clone();
                    u.checking = false;
                    u.dismissed = false;
                }
            }
            Ok(None) => {
                self.update = None;
                self.toast(format!("reIDM {} is up to date", env!("CARGO_PKG_VERSION")));
            }
            Err(e) => {
                if let Some(u) = &mut self.update {
                    u.checking = false;
                    u.error = Some(e);
                }
            }
        }
    }

    /// Notice downloads that have just finished, for IDM's "Download complete" window. The first
    /// snapshot only records state: downloads that finished before the window opened are old news.
    fn note_completions(&mut self) {
        if !self.snap.connected || self.snap.rev == self.noted_rev {
            return;
        }
        self.noted_rev = self.snap.rev;
        let mut ids = HashSet::with_capacity(self.snap.items.len());
        for it in self.snap.items.iter() {
            ids.insert(it.id);
            let before = self.last_status.insert(it.id, it.status.clone());
            let finished_now = it.status == Status::Done && before.is_some_and(|b| b != Status::Done);
            if self.primed && finished_now && self.prefs.notify_complete && !self.completed.contains(&it.id) {
                self.completed.push(it.id);
            }
        }
        self.last_status.retain(|id, _| ids.contains(id));
        self.completed.retain(|id| ids.contains(id));
        self.selected.retain(|id| ids.contains(id));
        self.primed = true;
    }

    fn item(&self, id: u64) -> Option<&Item> {
        self.snap.item(id)
    }

    // ── selection ────────────────────────────────────────────────────────

    fn select_only(&mut self, id: u64) {
        self.selected.clear();
        self.selected.insert(id);
        self.anchor = Some(id);
        self.focus = Some(id);
    }

    fn toggle_select(&mut self, id: u64) {
        if !self.selected.remove(&id) {
            self.selected.insert(id);
            self.anchor = Some(id);
            self.focus = Some(id);
        }
    }

    fn select_range(&mut self, id: u64) {
        let Some(a) = self.anchor else { return self.select_only(id) };
        let ordered: Vec<u64> = self.vm.ids(&self.snap).collect();
        let (Some(i), Some(j)) = (ordered.iter().position(|x| *x == a), ordered.iter().position(|x| *x == id)) else {
            return self.select_only(id);
        };
        let (lo, hi) = if i <= j { (i, j) } else { (j, i) };
        self.selected = ordered[lo..=hi].iter().copied().collect();
        self.focus = Some(id);
    }

    /// The selected downloads, in list order.
    fn selected_items(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self.selected.iter().filter_map(|id| self.item(*id)).collect();
        v.sort_by_key(|i| i.id);
        v
    }

    fn selected_ids(&self) -> Vec<u64> {
        self.selected_items().iter().map(|i| i.id).collect()
    }

    // ── commands ─────────────────────────────────────────────────────────
    //
    // Each runs on the command thread; its result comes back as a toast.

    /// One action on many downloads, in one request.
    fn bulk(&mut self, ids: Vec<u64>, action: &'static str, done: impl FnOnce(usize) -> String + Send + 'static) {
        if ids.is_empty() {
            return;
        }
        self.cmd.run(move |c| {
            let r = c.bulk(&ids, action);
            reply(move |app: &mut App| match r {
                Ok((n, None)) => app.toast(done(n)),
                Ok((0, Some(e))) => app.toast(format!("Could not {}: {e}", action.replace('-', " "))),
                Ok((n, Some(e))) => app.toast(format!("{} — {e}", done(n))),
                Err(e) => app.toast(format!("Could not {}: {e}", action.replace('-', " "))),
            })
        });
    }

    /// A single action whose failure is worth saying (open, seed…).
    fn action(&self, id: u64, what: &'static str) {
        self.cmd.run(move |c| {
            let r = c.post(&format!("/api/downloads/{id}/{what}"), serde_json::json!({}));
            r.err().and_then(|e| reply(move |app: &mut App| app.toast(format!("Could not {}: {e}", what.replace('-', " ")))))
        });
    }

    fn start_selected(&mut self) {
        let ids: Vec<u64> = self.selected_items().iter().filter(|i| i.status.is_stopped()).map(|i| i.id).collect();
        if ids.is_empty() {
            return self.toast("Select a paused or failed download to resume");
        }
        self.bulk(ids, "resume", |n| format!("Resumed {n} download(s)"));
    }

    fn stop_selected(&mut self) {
        let ids: Vec<u64> = self.selected_items().iter().filter(|i| i.status.is_active()).map(|i| i.id).collect();
        if ids.is_empty() {
            return self.toast("Select a running download to pause");
        }
        self.bulk(ids, "pause", |n| format!("Paused {n} download(s)"));
    }

    fn stop_all(&mut self) {
        let ids: Vec<u64> = self.snap.items.iter().filter(|i| i.status.is_active()).map(|i| i.id).collect();
        if ids.is_empty() {
            return self.toast("Nothing is downloading");
        }
        self.bulk(ids, "pause", |n| format!("Paused {n} download(s)"));
    }

    fn start_all(&mut self) {
        let ids: Vec<u64> = self.snap.items.iter().filter(|i| i.status.is_stopped()).map(|i| i.id).collect();
        if ids.is_empty() {
            return self.toast("Nothing is paused");
        }
        self.bulk(ids, "resume", |n| format!("Resumed {n} download(s)"));
    }

    /// Ask first: restarting throws away what has arrived, and for a finished download replaces
    /// the file.
    fn restart_selected(&mut self) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return self.toast("Select a download to restart");
        }
        self.confirm = Some(Confirm::Restart(ids));
    }

    fn set_queued(&mut self, queued: bool) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return self.toast("Select a download first");
        }
        self.bulk(ids, if queued { "schedule" } else { "unschedule" }, move |n| match (queued, n) {
            (true, 0) => "Finished downloads can't be added to the scheduled queue".to_string(),
            (true, n) => format!("{n} added to the scheduled queue"),
            (false, n) => format!("{n} removed from the scheduled queue"),
        });
    }

    fn queue_now(&mut self, start: bool) {
        self.cmd.run(move |c| {
            let r = c.post(&format!("/api/queue/{}", if start { "start" } else { "stop" }), serde_json::json!({}));
            let r = r.map(|t| serde_json::from_str::<serde_json::Value>(&t).ok().and_then(|v| v["count"].as_u64()).unwrap_or(0));
            reply(move |app: &mut App| match r {
                Ok(0) if start => app.toast("Nothing in the scheduled queue is waiting to start"),
                Ok(0) => app.toast("Nothing in the scheduled queue is running"),
                Ok(n) => app.toast(if start { format!("Started the queue: {n} download(s)") } else { format!("Stopped the queue: {n} download(s)") }),
                Err(e) => app.toast(format!("Could not {} the queue: {e}", if start { "start" } else { "stop" })),
            })
        });
    }

    /// Ask first, with the count: this empties the history, with no undo.
    fn delete_completed(&mut self) {
        let done: Vec<u64> = self.snap.items.iter().filter(|i| i.status == Status::Done).map(|i| i.id).collect();
        if done.is_empty() {
            return self.toast("There are no completed downloads");
        }
        self.confirm = Some(Confirm::ClearCompleted(done));
    }

    fn ask_remove(&mut self) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return self.toast("Select the downloads to remove");
        }
        self.confirm = Some(Confirm::Remove(ids));
    }

    /// Stop the background service too, and close the window. Downloads carry on from where
    /// they stopped the next time reIDM starts.
    fn quit_completely(&mut self, ctx: &egui::Context) {
        self.prefs.close_explained = true;
        self.prefs.save();
        let ctx = ctx.clone();
        self.cmd.run(move |c| {
            let r = c.post("/api/shutdown", serde_json::json!({}));
            reply(move |app: &mut App| match r {
                Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => app.toast(format!("Could not stop the background service: {e}")),
            })
        });
    }

    fn remove_ids(&mut self, ids: &[u64], purge: bool) {
        for id in ids {
            self.selected.remove(id);
        }
        self.bulk(ids.to_vec(), if purge { "purge" } else { "remove" }, move |n| match (purge, n) {
            (true, 1) => "Download and file removed".to_string(),
            (true, n) => format!("{n} downloads and their files removed"),
            (false, 1) => "Download removed from the list".to_string(),
            (false, n) => format!("{n} downloads removed from the list"),
        });
    }

    /// Set the service's settings, then say how it went.
    fn save_settings(&self, patch: serde_json::Value, done: impl FnOnce(&mut App, Result<(), String>) + Send + 'static) {
        self.cmd.run(move |c| {
            let r = c.post("/api/settings", patch).map(|_| ());
            reply(move |app: &mut App| done(app, r))
        });
    }

    fn report_batch(&mut self, results: Result<Vec<BatchResult>, String>, later: bool) {
        match results {
            Ok(results) => {
                let count = |k: &str| results.iter().filter(|r| r.outcome.as_deref() == Some(k)).count();
                let new = count("new");
                let resumed = count("existing");
                let have = count("already_complete");
                let failed = results.iter().filter(|r| r.error.is_some()).count();
                let mut parts = Vec::new();
                if new > 0 {
                    parts.push(if later { format!("{new} added to the queue") } else { format!("{new} queued") });
                }
                if resumed > 0 {
                    parts.push(format!("{resumed} {}", if later { "already listed" } else { "resumed" }));
                }
                if have > 0 {
                    parts.push(format!("{have} already downloaded"));
                }
                if failed > 0 {
                    parts.push(format!("{failed} failed"));
                }
                if let Some(first) = results.iter().find(|r| r.error.is_some()) {
                    parts.push(format!("— {}: {}", first.url, first.error.clone().unwrap_or_default()));
                }
                self.toast(if parts.is_empty() { "Nothing to add".to_string() } else { parts.join(", ") });
            }
            Err(e) => self.toast(format!("could not queue: {e}")),
        }
    }

    /// Queue every URL in a line- or file-shaped list, and say what happened.
    fn queue_urls(&mut self, urls: Vec<String>, later: bool) {
        if urls.is_empty() {
            return self.toast("No URLs to add");
        }
        self.queue_batch(serde_json::json!({ "urls": urls, "later": later }), later);
    }

    fn queue_batch(&mut self, body: serde_json::Value, later: bool) {
        self.toast("Adding…");
        self.cmd.run(move |c| {
            // A big batch can keep the service busy for a while.
            let r = c.with_timeout(Duration::from_secs(600)).batch(body);
            reply(move |app: &mut App| app.report_batch(r, later))
        });
    }

    /// Open the Add URL dialog, with whatever links are on the clipboard.
    fn open_add(&mut self) {
        if self.add.urls.trim().is_empty() && self.prefs.clipboard {
            if let Some(text) = clipboard_text() {
                let urls = reidm_core::grabber::parse_url_list(&text);
                if !urls.is_empty() {
                    self.add.urls = urls.join("\n");
                }
            }
        }
        if self.add.dir.is_empty() {
            if let Some(s) = &self.snap.service.settings {
                self.add.dir = s.default_dir.clone();
            }
        }
        self.add.open = true;
    }

    /// Read URLs from a text file or an IDM `.ef2` export.
    fn import_from_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("URL lists and IDM exports", &["txt", "list", "csv", "ef2"])
            .set_title("Import downloads")
            .pick_file()
        else {
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                if reidm_core::lists::looks_like_ef2(&text) {
                    let entries = reidm_core::lists::parse_ef2(&text);
                    if entries.is_empty() {
                        return self.toast("That IDM export holds no downloads");
                    }
                    self.queue_batch(serde_json::json!({ "items": entries, "later": false }), false);
                } else {
                    self.queue_urls(reidm_core::grabber::parse_url_list(&text), false);
                }
            }
            Err(e) => self.toast(format!("could not read {}: {e}", path.display())),
        }
    }

    /// Write the download list as plain URLs or as IDM's `.ef2`.
    fn export_download_list(&mut self, ef2: bool) {
        if self.snap.items.is_empty() {
            return self.toast("There is nothing to export");
        }
        let format = if ef2 { "ef2" } else { "txt" };
        let Some(path) = rfd::FileDialog::new()
            .add_filter(if ef2 { "IDM export" } else { "URL list" }, &[format])
            .set_file_name(format!("reidm-downloads.{format}"))
            .set_title("Export the download list")
            .save_file()
        else {
            return;
        };
        let n = self.snap.items.len();
        self.cmd.run(move |c| {
            let r = c.with_timeout(Duration::from_secs(120)).get(&format!("/api/export?format={format}")).and_then(|text| std::fs::write(&path, text).map_err(|e| e.to_string()));
            reply(move |app: &mut App| match r {
                Ok(()) => app.toast(format!("Exported {n} download(s) to {}", path.display())),
                Err(e) => app.toast(format!("Could not export: {e}")),
            })
        });
    }

    /// Export just the grabber's results as a URL list.
    fn export_found(&mut self, urls: &[String]) {
        let Some(path) = rfd::FileDialog::new().add_filter("URL list", &["txt"]).set_file_name("grabbed-urls.txt").save_file() else {
            return;
        };
        match std::fs::write(&path, reidm_core::grabber::url_list_text(urls)) {
            Ok(()) => self.toast(format!("Exported {} URL(s)", urls.len())),
            Err(e) => self.toast(format!("could not write {}: {e}", path.display())),
        }
    }

    // ── site grabber ─────────────────────────────────────────────────────

    fn start_grab(&mut self) {
        let url = self.grab.url.trim().to_string();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.grab.error = Some("Enter an http or https page address".into());
            return;
        }
        let opts = GrabOptions {
            depth: self.grab.depth,
            types: self.grab.types.split(',').map(|t| t.trim().trim_start_matches('.').to_ascii_lowercase()).filter(|t| !t.is_empty()).collect(),
            same_host_only: self.grab.same_host,
            include: self.grab.include.lines().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
            exclude: self.grab.exclude.lines().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
            sizes: self.grab.sizes,
        };
        self.grab.rx = Some(self.client.grab_async(&url, &opts));
        self.grab.scanning = true;
        self.grab.error = None;
    }

    fn poll_grab(&mut self) {
        let Some(rx) = &self.grab.rx else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.grab.rx = None;
        self.grab.scanning = false;
        match result {
            Ok(files) => {
                self.grab.selected = vec![true; files.len()];
                self.grab.found = files;
                if self.grab.found.is_empty() {
                    self.grab.error = Some("No downloadable files found on that page".into());
                }
            }
            Err(e) => {
                self.grab.found.clear();
                self.grab.selected.clear();
                self.grab.error = Some(e);
            }
        }
    }

    fn poll_refresh(&mut self) {
        let mut done: Option<Result<(), String>> = None;
        if let Some(r) = &mut self.refresh {
            if let Some(rx) = &r.busy {
                if let Ok(res) = rx.try_recv() {
                    r.busy = None;
                    done = Some(res);
                }
            }
        }
        match done {
            Some(Ok(())) => {
                self.refresh = None;
                self.toast("New address accepted — the download carries on from where it was");
            }
            Some(Err(e)) => {
                if let Some(r) = &mut self.refresh {
                    r.error = Some(e);
                }
            }
            None => {}
        }
        // Capturing from the browser: it ends when the daemon stops waiting.
        let finished = self.refresh.as_ref().filter(|r| r.capturing).and_then(|r| {
            let still_armed = self.snap.service.refresh_armed == Some(r.id);
            let now_url = self.item(r.id).map(|i| i.request.url.clone()).unwrap_or_default();
            if now_url != r.old_url {
                Some(true)
            } else if !still_armed && self.snap.connected {
                Some(false)
            } else {
                None
            }
        });
        match finished {
            Some(true) => {
                self.refresh = None;
                self.toast("The browser's link is in — the download carries on from where it was");
            }
            Some(false) => {
                if let Some(r) = &mut self.refresh {
                    r.capturing = false;
                    r.error = Some("Stopped waiting for the browser (it waits three minutes). Try again, or paste the address.".into());
                }
            }
            None => {}
        }
    }

    fn open_selected(&mut self, reveal: bool) {
        let Some(id) = self.focus.filter(|id| self.selected.contains(id)) else {
            return self.toast("Select a download first");
        };
        self.open_id(id, reveal);
    }

    fn open_id(&mut self, id: u64, reveal: bool) {
        let Some(it) = self.item(id) else { return };
        if !matches!(it.status, Status::Done) && !reveal {
            return self.toast("That download has not finished yet");
        }
        self.action(id, if reveal { "folder" } else { "open" });
    }

    fn open_props(&mut self, id: u64) {
        let limit = self.item(id).and_then(|i| i.speed_limit);
        self.props = Some(PropsState { id, limit_on: limit.is_some(), limit_kb: limit.map(|l| (l / 1024) as u32).unwrap_or(512).max(1), detail: None, fetched: None, fetching: false });
    }

    /// Keep Properties' full copy of its download fresh: about once a second, and never more
    /// than one request at a time.
    fn refresh_props(&mut self) {
        let Some(st) = &mut self.props else { return };
        if st.fetching || st.fetched.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        st.fetching = true;
        let id = st.id;
        self.cmd.run(move |c| {
            let d = c.detail(id).ok();
            reply(move |app: &mut App| {
                if let Some(st) = app.props.as_mut().filter(|st| st.id == id) {
                    st.fetching = false;
                    st.fetched = Some(Instant::now());
                    if d.is_some() {
                        st.detail = d;
                    }
                }
            })
        });
    }

    fn open_refresh(&mut self, id: u64) {
        if self.item(id).is_some_and(|i| i.request.media.is_some()) {
            self.toast("A video download finds its own address each time it starts: there is nothing to refresh. Start it again.");
            return;
        }
        let old_url = self.item(id).map(|i| i.request.url.clone()).unwrap_or_default();
        self.refresh = Some(RefreshState { id, url: String::new(), busy: None, error: None, capturing: false, old_url });
    }

    /// The service's settings as last polled (at most a few seconds old), or why there are none.
    fn known_settings(&self) -> Option<crate::model::Settings> {
        self.snap.service.settings.clone()
    }

    fn open_scheduler(&mut self) {
        let Some(s) = self.known_settings().map(|s| s.schedule) else {
            return self.toast("The scheduler's settings haven't loaded yet: is the download service running?");
        };
        let mut days = [false; 7];
        for d in &s.days {
            if let Some(slot) = days.get_mut(*d as usize) {
                *slot = true;
            }
        }
        if s.days.is_empty() {
            days = [true; 7];
        }
        self.sched = SchedEdit {
            open: true,
            enabled: s.enabled,
            start: s.start.clone().unwrap_or_else(|| "02:00".into()),
            use_stop: s.stop.is_some(),
            stop: s.stop.clone().unwrap_or_else(|| "07:00".into()),
            days,
            error: None,
        };
    }

    fn open_limiter(&mut self) {
        let Some(s) = self.known_settings() else {
            return self.toast("The speed limiter's settings haven't loaded yet: is the download service running?");
        };
        self.limiter = LimiterEdit { open: true, on: s.limit_enabled, kb: if s.speed_limit > 0 { (s.speed_limit / 1024) as u32 } else { 1024 } };
    }

    /// Turn the limiter on or off from the menu. With no rate chosen yet, "on" would limit
    /// nothing, so that opens the limiter's settings instead.
    fn toggle_limiter(&mut self) {
        let Some(s) = self.known_settings() else { return self.open_limiter() };
        if !s.limit_enabled && s.speed_limit == 0 {
            return self.open_limiter();
        }
        let on = !s.limit_enabled;
        self.save_settings(serde_json::json!({ "limit_enabled": on }), move |app, r| match r {
            Ok(()) => app.toast(if on { "Speed limiter on" } else { "Speed limiter off" }),
            Err(e) => app.toast(format!("Could not change the speed limiter: {e}")),
        });
    }

    /// Options opens on the settings last polled, and fetches them again: Save sends only what
    /// changed against those, and is offered only once they are known.
    fn open_options(&mut self) {
        self.opt_base = self.known_settings().map(|s| OptionsForm::from_settings(&s));
        if let Some(b) = &self.opt_base {
            self.opt = b.clone();
        }
        self.cmd.run(|c| {
            let s = c.settings();
            reply(move |app: &mut App| {
                let Ok(s) = s else { return };
                let fresh = OptionsForm::from_settings(&s);
                // Unless the user has started editing, show the very latest.
                if app.opt_base.as_ref().is_none_or(|b| *b == app.opt) {
                    app.opt = fresh.clone();
                    app.opt_base = Some(fresh);
                }
            })
        });
        self.opt_tab = OptTab::General;
        self.show_options = true;
    }

    /// *Add YouTube playlist*, with a playlist link from the clipboard already filled in.
    fn open_playlist(&mut self) {
        let s = self.known_settings().unwrap_or_default();
        let dir = |n: &str| s.folders.dirs.get(n).cloned().unwrap_or_else(|| s.default_dir.clone());
        let (video, music) = if s.folders.sort { (dir("Video"), dir("Music")) } else { (s.default_dir.clone(), s.default_dir.clone()) };
        let clip = arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()).map(|t| t.trim().to_string()).filter(|t| t.starts_with("http") && (t.contains("list=") || t.contains("/@") || t.contains("/channel/")));
        self.playlist.open_with(clip, video, music);
    }

    /// Where a category goes when the user hasn't chosen: under the download folder.
    fn default_folder(&self, name: &str) -> String {
        let base = std::path::Path::new(&self.opt.dir);
        if name == reidm_core::Category::General.name() { base.to_string_lossy().into_owned() } else { base.join(name).to_string_lossy().into_owned() }
    }

    /// Keyboard shortcuts for the list. Ignored while a text field has the keyboard.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (del, all, enter, new, paste) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            (
                i.key_pressed(egui::Key::Delete),
                cmd && i.key_pressed(egui::Key::A),
                i.key_pressed(egui::Key::Enter),
                (cmd && i.key_pressed(egui::Key::N)) || i.key_pressed(egui::Key::Insert),
                i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
            )
        });
        if del && !self.selected.is_empty() {
            self.confirm = Some(Confirm::Remove(self.selected_ids()));
        }
        if all {
            self.selected = self.vm.ids(&self.snap).collect();
        }
        if enter {
            if let Some(id) = self.focus.filter(|id| self.selected.contains(id)) {
                if self.item(id).is_some_and(|i| i.status == Status::Done) {
                    self.open_id(id, false);
                } else {
                    self.open_props(id);
                }
            }
        }
        if new || (paste && !self.add.open) {
            self.open_add();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(s) = self.poller.rx.try_iter().last() {
            self.snap = s;
            self.note_completions();
        }
        for r in self.cmd.replies() {
            r(self);
        }
        self.vm.update(&self.snap, self.view, &self.search, self.sort, self.desc);
        self.poll_update();
        self.poll_grab();
        self.poll_refresh();
        self.refresh_props();
        // One check shortly after launch, so a new release is noticed without the user going
        // looking. It is a single GET to api.github.com and reports nothing back to us.
        if !self.checked_on_start && self.snap.connected {
            self.checked_on_start = true;
            self.check_for_updates();
        }
        // The first time the window is closed, say that downloads carry on without it.
        if ctx.input(|i| i.viewport().close_requested()) && !self.prefs.close_explained && self.snap.connected {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Close);
        }
        // Poll as often as what is on screen deserves.
        let (focused, minimized) = ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.viewport().minimized.unwrap_or(false)));
        let busy = self.vm.counts.running > 0 || self.snap.total_rate > 0.0;
        self.poller.pace.set(Pace::for_window(busy, focused, minimized));

        if self.applied_theme != Some(self.prefs.dark) {
            theme::apply(&ctx, self.prefs.dark);
            if self.applied_theme.is_some() {
                self.prefs.save();
            }
            self.applied_theme = Some(self.prefs.dark);
        }
        let p = theme::palette_for(self.prefs.dark);
        self.shortcuts(&ctx);

        // Panel order matters: outermost first, central panel last.
        if self.update.as_ref().is_some_and(|u| !u.version.is_empty() && !u.dismissed) {
            self.update_banner(ui, &p);
        }
        self.menu_bar(ui, &p);
        self.toolbar(ui, &p);
        self.status_bar(ui, &p);
        self.sidebar(ui, &p);
        self.table(ui, &p);

        self.add_dialog(&ctx, &p);
        self.batch_dialog(&ctx, &p);
        self.options_dialog(&ctx, &p);
        if let Some(msg) = self.playlist.show(&ctx, &self.client, &p) {
            self.toast(msg);
        }
        if let Some(msg) = self.torrent.show(&ctx, &self.client, &p) {
            self.toast(msg);
        }
        self.about_dialog(&ctx);
        self.confirm_dialog(&ctx, &p);
        self.grabber_window(&ctx, &p);
        self.props_window(&ctx, &p);
        self.refresh_window(&ctx, &p);
        self.scheduler_window(&ctx, &p);
        self.limiter_window(&ctx, &p);
        self.integration_window(&ctx, &p);
        self.complete_window(&ctx, &p);

        // Draw again only when something will have changed. A new snapshot and a command's
        // answer each ask for a frame themselves; what is left is time passing.
        let mut next = Duration::from_secs(30); // "Last Try" says "2 min ago"
        if let Some(t) = &self.toast {
            let left = t.until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.toast = None;
            } else {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -48.0))
                    .order(egui::Order::Tooltip)
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_max_width(640.0);
                            ui.label(RichText::new(&t.text).color(p.text));
                        });
                    });
                next = next.min(left);
            }
        }
        if self.waiting() {
            // An answer arriving on a channel of our own doesn't wake the window.
            next = next.min(Duration::from_millis(250));
        }
        ctx.request_repaint_after(next);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::palette_for(self.prefs.dark).bg.to_normalized_gamma_f32()
    }
}

// ── chrome ───────────────────────────────────────────────────────────────

impl App {
    fn update_banner(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let Some(u) = &self.update else { return };
        let (version, url, asset) = (u.version.clone(), u.url.clone(), u.asset.clone());
        let mut open = false;
        let mut dismiss = false;

        egui::Panel::top("update")
            .frame(egui::Frame::NONE.fill(p.accent.gamma_multiply(0.16)).inner_margin(egui::Margin::symmetric(10, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    icons::dot(ui.painter(), egui::Rect::from_center_size(ui.cursor().min + Vec2::new(5.0, 8.0), Vec2::splat(10.0)), p.accent);
                    ui.add_space(14.0);
                    ui.label(RichText::new(format!("reIDM {version} is available — you have {}", env!("CARGO_PKG_VERSION"))).color(p.text));
                    if let Some((name, _)) = &asset {
                        ui.label(RichText::new(format!("({name})")).color(p.text_dim).size(11.0));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Dismiss").clicked() {
                            dismiss = true;
                        }
                        if ui.small_button("Release notes").clicked() {
                            open = true;
                        }
                    });
                });
            });

        if dismiss {
            if let Some(u) = &mut self.update {
                u.dismissed = true;
            }
        }
        if open {
            self.about = false;
            open_external(&url);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::top("menu")
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(8, 4)))
            .show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    // IDM's own menu order.
                    ui.menu_button("Tasks", |ui| {
                        if ui.button("Add URL…").clicked() {
                            self.open_add();
                            ui.close();
                        }
                        if ui.button("Add torrent…").clicked() {
                            let clip = arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()).map(|t| t.trim().to_string()).filter(|t| t.starts_with("magnet:"));
                            self.torrent.open_with(clip);
                            ui.close();
                        }
                        if ui.button("Add YouTube playlist…").clicked() {
                            self.open_playlist();
                            ui.close();
                        }
                        if ui.button("Add batch download…").clicked() {
                            self.batch.open = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Import (URL list or IDM .ef2)…").clicked() {
                            self.import_from_file();
                            ui.close();
                        }
                        ui.menu_button("Export", |ui| {
                            if ui.button("As a URL list (.txt)…").clicked() {
                                self.export_download_list(false);
                                ui.close();
                            }
                            if ui.button("As an IDM export (.ef2)…").clicked() {
                                self.export_download_list(true);
                                ui.close();
                            }
                        });
                        ui.separator();
                        if ui.button("Site Grabber…").clicked() {
                            self.show_grabber = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Exit").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.menu_button("File", |ui| {
                        if ui.button("Open download").clicked() {
                            self.open_selected(false);
                            ui.close();
                        }
                        if ui.button("Open containing folder").clicked() {
                            self.open_selected(true);
                            ui.close();
                        }
                        if ui.button("Properties…").clicked() {
                            if let Some(id) = self.focus.filter(|id| self.selected.contains(id)) {
                                self.open_props(id);
                            }
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Remove…").clicked() {
                            self.ask_remove();
                            ui.close();
                        }
                        if ui.button("Remove completed…").clicked() {
                            self.delete_completed();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Quit reIDM completely (stops downloads)").clicked() {
                            self.quit_completely(ui.ctx());
                            ui.close();
                        }
                    });
                    ui.menu_button("Downloads", |ui| {
                        if ui.button("Resume").clicked() {
                            self.start_selected();
                            ui.close();
                        }
                        if ui.button("Pause").clicked() {
                            self.stop_selected();
                            ui.close();
                        }
                        if ui.button("Resume all").clicked() {
                            self.start_all();
                            ui.close();
                        }
                        if ui.button("Pause all").clicked() {
                            self.stop_all();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Restart from the beginning…").clicked() {
                            self.restart_selected();
                            ui.close();
                        }
                        if ui.button("Refresh download address…").clicked() {
                            match self.focus.filter(|id| self.selected.contains(id)) {
                                Some(id) => self.open_refresh(id),
                                None => self.toast("Select the download whose address expired"),
                            }
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Start queue").clicked() {
                            self.queue_now(true);
                            ui.close();
                        }
                        if ui.button("Stop queue").clicked() {
                            self.queue_now(false);
                            ui.close();
                        }
                        if ui.button("Scheduler…").clicked() {
                            self.open_scheduler();
                            ui.close();
                        }
                        ui.separator();
                        let on = self.snap.service.settings.as_ref().is_some_and(|s| s.limit_enabled);
                        if ui.button(if on { "Speed limiter: turn off" } else { "Speed limiter: turn on" }).clicked() {
                            self.toggle_limiter();
                            ui.close();
                        }
                        if ui.button("Speed limiter settings…").clicked() {
                            self.open_limiter();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Options…").clicked() {
                            self.open_options();
                            ui.close();
                        }
                    });
                    ui.menu_button("View", |ui| {
                        ui.checkbox(&mut self.prefs.dark, "Dark theme");
                        ui.checkbox(&mut self.cats_open, "Show categories");
                        ui.separator();
                        for v in [View::Category(Category::All), View::Unfinished, View::Finished, View::Queue] {
                            if ui.button(v.label()).clicked() {
                                self.view = v;
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button("Browser integration…").clicked() {
                            self.show_integration = true;
                            ui.close();
                        }
                    });
                    ui.menu_button("Help", |ui| {
                        if ui.button("Browser integration…").clicked() {
                            self.show_integration = true;
                            ui.close();
                        }
                        if ui.button("Check for updates").clicked() {
                            self.check_for_updates();
                            ui.close();
                        }
                        if ui.button("About reIDM").clicked() {
                            self.about = true;
                            ui.close();
                        }
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (txt, col) = if self.snap.connected { ("Ready", p.ok) } else { ("Background service not running", p.err) };
                        ui.label(RichText::new(txt).color(col).size(12.0));
                        // Painted: the bundled font has no reliable bullet glyph.
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                        icons::dot(ui.painter(), r, col);
                    });
                });
            });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::top("toolbar")
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(8, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let sep = |ui: &mut egui::Ui| {
                        ui.add_space(5.0);
                        let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, 40.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 0.0, p.border);
                        ui.add_space(5.0);
                    };

                    if tool(ui, p, "Add URL", Icon::Plus, true).clicked() {
                        self.open_add();
                    }
                    sep(ui);
                    if tool(ui, p, "Resume", Icon::Play, false).clicked() {
                        self.start_selected();
                    }
                    if tool(ui, p, "Pause", Icon::Pause, false).clicked() {
                        self.stop_selected();
                    }
                    if tool(ui, p, "Pause All", Icon::PauseAll, false).clicked() {
                        self.stop_all();
                    }
                    sep(ui);
                    if tool(ui, p, "Remove", Icon::Cross, false).clicked() {
                        self.ask_remove();
                    }
                    if tool(ui, p, "Remove\nCompleted", Icon::CrossAll, false).clicked() {
                        self.delete_completed();
                    }
                    sep(ui);
                    if tool(ui, p, "Options", Icon::Gear, false).clicked() {
                        self.open_options();
                    }
                    if tool(ui, p, "Scheduler", Icon::Clock, false).clicked() {
                        self.open_scheduler();
                    }
                    if tool(ui, p, "Start\nQueue", Icon::QueueStart, false).clicked() {
                        self.queue_now(true);
                    }
                    if tool(ui, p, "Stop\nQueue", Icon::QueueStop, false).clicked() {
                        self.queue_now(false);
                    }
                    sep(ui);
                    if tool(ui, p, "Site\nGrabber", Icon::Grabber, false).clicked() {
                        self.show_grabber = true;
                    }
                    if tool(ui, p, "Speed\nLimiter", Icon::Gauge, false).clicked() {
                        self.open_limiter();
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut s = std::mem::take(&mut self.search);
                        ui.add_sized(Vec2::new(190.0, 24.0), egui::TextEdit::singleline(&mut s).hint_text("Search…"));
                        self.search = s;
                        let (sr, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
                        icons::search(ui.painter(), sr, p.text_dim);
                    });
                });
            });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::left("cats")
            .default_size(204.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(6, 8)))
            .show(ui, |ui| {
                ui.label(RichText::new("CATEGORIES").size(10.0).color(p.text_dim).strong());
                ui.add_space(2.0);

                // IDM nests its categories under "Categories > All Downloads"; mirror that with
                // real expanders rather than a flat list.
                let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), egui::Sense::click());
                icons::chevron(ui.painter(), egui::Rect::from_min_size(r.min + Vec2::new(2.0, 4.0), Vec2::splat(12.0)), p.text_dim, self.cats_open);
                ui.painter().text(r.min + Vec2::new(18.0, 10.0), egui::Align2::LEFT_CENTER, "Categories", egui::FontId::proportional(11.5), p.text_dim);
                if resp.clicked() {
                    self.cats_open = !self.cats_open;
                }

                if self.cats_open {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::click());
                    icons::chevron(ui.painter(), egui::Rect::from_min_size(r.min + Vec2::new(16.0, 5.0), Vec2::splat(12.0)), p.text, self.downloads_open);
                    icons::folder(ui.painter(), egui::Rect::from_min_size(r.min + Vec2::new(30.0, 4.0), Vec2::splat(14.0)), p.text_dim);
                    let all = View::Category(Category::All);
                    let on_all = self.view == all;
                    ui.painter().text(r.min + Vec2::new(50.0, 11.0), egui::Align2::LEFT_CENTER, "All Downloads", egui::FontId::proportional(12.5), if on_all { p.accent } else { p.text });
                    ui.painter().text(r.right_center() - Vec2::new(8.0, 0.0), egui::Align2::RIGHT_CENTER, self.vm.counts.total.to_string(), egui::FontId::proportional(11.5), p.text_dim);
                    if resp.clicked() {
                        // The chevron toggles; the label selects.
                        if ui.input(|i| i.pointer.interact_pos()).is_some_and(|pos| pos.x < r.min.x + 30.0) {
                            self.downloads_open = !self.downloads_open;
                        } else {
                            self.view = all;
                        }
                    }

                    if self.downloads_open {
                        for c in CATEGORIES.into_iter().filter(|c| *c != Category::All) {
                            let v = View::Category(c);
                            let dark = self.prefs.dark;
                            if tree_row(ui, p, 34.0, c.label(), Some(self.vm.counts.of(v)), self.view == v, |pt, r| icons::file_card(pt, r, c, dark)) {
                                self.view = v;
                            }
                        }
                    }
                    for (v, done) in [(View::Unfinished, false), (View::Finished, true)] {
                        let col = if done { p.ok } else { p.accent };
                        if tree_row(ui, p, 16.0, v.label(), Some(self.vm.counts.of(v)), self.view == v, |pt, r| {
                            if done {
                                icons::dot(pt, r, col)
                            } else {
                                icons::play(pt, r, col)
                            }
                        }) {
                            self.view = v;
                        }
                    }
                }

                ui.add_space(8.0);
                ui.label(RichText::new("QUEUES").size(10.0).color(p.text_dim).strong());
                let q = View::Queue;
                let qcol = p.text_dim;
                if tree_row(ui, p, 4.0, q.label(), Some(self.vm.counts.of(q)), self.view == q, |pt, r| icons::clock(pt, r, qcol)) {
                    self.view = q;
                }
                ui.add_space(8.0);
                ui.label(RichText::new("GRABBER").size(10.0).color(p.text_dim).strong());
                let gcol = p.text_dim;
                if tree_row(ui, p, 4.0, "Site grabber…", None, false, |pt, r| icons::grabber(pt, r, gcol)) {
                    self.show_grabber = true;
                }
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::bottom("status")
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(10, 5)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let c = &self.vm.counts;
                    let (active, done, failed, queued) = (c.running, c.done, c.failed, c.queued);

                    let mut parts = vec![format!("{} total", c.total)];
                    if active > 0 {
                        parts.push(format!("{active} downloading"));
                    }
                    if done > 0 {
                        parts.push(format!("{done} complete"));
                    }
                    if failed > 0 {
                        parts.push(format!("{failed} failed"));
                    }
                    if queued > 0 {
                        parts.push(format!("{queued} in queue"));
                    }
                    ui.label(RichText::new(parts.join("  ·  ")).color(p.text_dim).size(12.0));

                    // Browser capture: the one thing users most need to know is working.
                    ui.separator();
                    let ext = self.snap.service.extension.clone().filter(|e| e.seconds_ago < 180);
                    let (txt, col, tip) = match &ext {
                        Some(e) => (
                            format!("Browser: {}", if e.browser.is_empty() { "connected".to_string() } else { e.browser.clone() }),
                            p.ok,
                            format!("reIDM's extension {} checked in {}s ago. Downloads started in the browser come here.", e.version, e.seconds_ago),
                        ),
                        None => (
                            "Browser: extension not connected".to_string(),
                            p.warn,
                            "reIDM's browser extension has not been seen. Click to set it up.".to_string(),
                        ),
                    };
                    if ui.add(egui::Label::new(RichText::new(txt).color(col).size(12.0)).sense(egui::Sense::click())).on_hover_text(tip).clicked() {
                        self.show_integration = true;
                    }

                    if self.snap.connected && self.snap.service.platform == "windows" && !self.snap.service.idm_port {
                        ui.separator();
                        ui.label(RichText::new("Port 1001 busy").color(p.warn).size(12.0)).on_hover_text(
                            "Another program (usually an installed IDM) holds port 1001, so the official IDM extension \
                             cannot reach reIDM directly. reIDM's own extension is unaffected.",
                        );
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.snap.total_rate > 0.0 {
                            ui.label(RichText::new(rate_human(self.snap.total_rate)).color(p.accent).size(12.0).strong());
                            ui.label(RichText::new("Total rate:").color(p.text_dim).size(12.0));
                            ui.separator();
                        }
                        if let Some(s) = &self.snap.service.settings {
                            let txt = if s.limit_enabled && s.speed_limit > 0 { format!("Speed limit: {}", rate_human(s.speed_limit as f64)) } else { "Speed limit: off".into() };
                            let col = if s.limit_enabled { p.warn } else { p.text_dim };
                            if ui.add(egui::Label::new(RichText::new(txt).color(col).size(12.0)).sense(egui::Sense::click())).on_hover_text("Click to change").clicked() {
                                self.open_limiter();
                            }
                            ui.separator();
                        }
                        if let Some(u) = &self.update {
                            if u.checking {
                                ui.label(RichText::new("Checking for updates…").color(p.text_dim).size(11.0));
                            } else if let Some(e) = &u.error {
                                ui.label(RichText::new("Update check failed").color(p.warn).size(11.0)).on_hover_text(e);
                            }
                        }
                    });
                });
            });
    }
}

/// One row of the category tree. Returns whether it was clicked.
fn tree_row(ui: &mut egui::Ui, p: &Palette, indent: f32, label: &str, n: Option<usize>, on: bool, icon: impl FnOnce(&egui::Painter, egui::Rect)) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 23.0), egui::Sense::click());
    if on {
        ui.painter().rect_filled(rect, 3.0, p.accent.gamma_multiply(0.22));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, p.accent.gamma_multiply(0.09));
    }
    icon(ui.painter(), egui::Rect::from_min_size(rect.min + Vec2::new(indent, 4.0), Vec2::splat(15.0)));
    ui.painter().text(rect.min + Vec2::new(indent + 22.0, 11.5), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), if on { p.accent } else { p.text });
    if let Some(n) = n {
        ui.painter().text(rect.right_center() - Vec2::new(8.0, 0.0), egui::Align2::RIGHT_CENTER, n.to_string(), egui::FontId::proportional(11.5), p.text_dim);
    }
    resp.clicked()
}

// ── the download list ────────────────────────────────────────────────────

impl App {
    fn table(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(p.panel_alt).inner_margin(egui::Margin::ZERO))
            .show(ui, |ui| {
                // Shared, not copied: the snapshot's rows and the view's order.
                let snap = self.snap.clone();
                let rows = std::mem::take(&mut self.vm.rows);
                let mut click: Option<(u64, bool, bool)> = None; // id, ctrl, shift
                let mut check_click: Option<u64> = None;
                let mut open_req: Option<u64> = None;
                let mut ctx_menu: Option<u64> = None;
                let mut ctx_cmd: Option<CtxCmd> = None;
                let mut sort_click: Option<SortKey> = None;
                let mut toggle_all = false;
                let now = now_secs();

                if rows.is_empty() {
                    let waiting = (!self.snap.connected).then(|| self.snap.problem.clone()).flatten();
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        if let Some(why) = &waiting {
                            ui.label(RichText::new(why).color(p.warn));
                            ui.add_space(8.0);
                            ui.label(RichText::new("The list appears here as soon as reIDM's background service answers.").color(p.text_dim));
                            return;
                        }
                        ui.label(
                            RichText::new(if self.snap.items.is_empty() {
                                "No downloads yet — use Add URL, or start a download in your browser."
                            } else if self.view == View::Queue {
                                "The scheduled queue is empty. Add downloads to it with Download Later, or right-click › Add to queue."
                            } else {
                                "Nothing in this category."
                            })
                            .color(p.text_dim),
                        );
                    });
                    self.vm.rows = rows;
                    return;
                }

                // Header checkbox state: all / none / some of the visible rows.
                let selected_visible = if self.selected.is_empty() { 0 } else { rows.iter().filter(|&&i| self.selected.contains(&snap.items[i].id)).count() };
                let header_check = if selected_visible == 0 {
                    Some(false)
                } else if selected_visible == rows.len() {
                    Some(true)
                } else {
                    None
                };

                TableBuilder::new(ui)
                    .striped(false)
                    .resizable(true)
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .column(Column::exact(30.0)) // select
                    .column(Column::initial(300.0).at_least(160.0).clip(true)) // name
                    .column(Column::initial(96.0).at_least(72.0)) // size
                    .column(Column::initial(118.0).at_least(84.0)) // status
                    .column(Column::initial(82.0).at_least(64.0)) // time left
                    .column(Column::initial(100.0).at_least(80.0)) // rate
                    .column(Column::initial(88.0).at_least(70.0)) // last try
                    .column(Column::remainder().at_least(110.0).clip(true)) // description
                    .header(26.0, |mut header| {
                        header.col(|ui| {
                            let (r, resp) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::click());
                            icons::checkbox(ui.painter(), r, header_check, p.accent, p.border);
                            if resp.clicked() {
                                toggle_all = true;
                            }
                        });
                        // Sortable column labels, with IDM's column order and titles.
                        let cur = self.sort;
                        let desc = self.desc;
                        let mut h = |header: &mut egui_extras::TableRow, label: &str, k: SortKey| {
                            header.col(|ui| {
                                let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
                                let sorted = cur == k;
                                ui.painter().text(
                                    rect.left_center() + Vec2::new(2.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    label,
                                    egui::FontId::proportional(12.0),
                                    if sorted { p.accent } else { p.text },
                                );
                                if sorted {
                                    // Painted rather than a glyph: the default font has no
                                    // reliable triangle character, and a missing one shows as a box.
                                    let c = egui::Pos2::new(
                                        rect.left() + 2.0 + ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), p.text).size().x + 8.0,
                                        rect.center().y,
                                    );
                                    let d = 3.5;
                                    let pts = if desc {
                                        vec![egui::Pos2::new(c.x - d, c.y - d * 0.7), egui::Pos2::new(c.x + d, c.y - d * 0.7), egui::Pos2::new(c.x, c.y + d * 0.7)]
                                    } else {
                                        vec![egui::Pos2::new(c.x - d, c.y + d * 0.7), egui::Pos2::new(c.x + d, c.y + d * 0.7), egui::Pos2::new(c.x, c.y - d * 0.7)]
                                    };
                                    ui.painter().add(egui::Shape::convex_polygon(pts, p.accent, egui::Stroke::NONE));
                                }
                                if resp.clicked() {
                                    sort_click = Some(k);
                                }
                            });
                        };
                        h(&mut header, "File Name", SortKey::Name);
                        h(&mut header, "Size", SortKey::Size);
                        h(&mut header, "Status", SortKey::Status);
                        h(&mut header, "Time Left", SortKey::TimeLeft);
                        h(&mut header, "Transfer Rate", SortKey::Rate);
                        h(&mut header, "Last Try", SortKey::LastTry);
                        h(&mut header, "Address", SortKey::Address);
                    })
                    .body(|body| {
                        body.rows(42.0, rows.len(), |mut row| {
                            let idx = row.index();
                            let it: &Item = &snap.items[rows[idx]];
                            let id = it.id;
                            let is_sel = self.selected.contains(&id);
                            let rate = self.snap.rates.get(&id).copied();

                            if is_sel {
                                row.set_selected(true);
                            }

                            // select checkbox
                            row.col(|ui| {
                                let (r, resp) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::click());
                                icons::checkbox(ui.painter(), r, Some(is_sel), p.accent, p.border);
                                if resp.clicked() {
                                    check_click = Some(id);
                                }
                            });

                            // File name, with the type icon and a progress bar underneath
                            row.col(|ui| {
                                let resp = ui.allocate_response(ui.available_size(), egui::Sense::click());
                                let rect = resp.rect;
                                if is_sel {
                                    ui.painter().rect_filled(rect, 0.0, p.accent.gamma_multiply(0.20));
                                }
                                let icon_rect = egui::Rect::from_min_size(rect.min + Vec2::new(4.0, 9.0), Vec2::splat(18.0));
                                icons::file_card(ui.painter(), icon_rect, it.category(), self.prefs.dark);
                                let mut text_x = rect.left() + 28.0;
                                if it.scheduled && it.status != Status::Done {
                                    // A small clock: this one waits for the scheduled queue.
                                    icons::clock(ui.painter(), egui::Rect::from_min_size(egui::Pos2::new(text_x, rect.top() + 6.0), Vec2::splat(14.0)), p.text_dim);
                                    text_x += 17.0;
                                }
                                ui.painter().text(egui::Pos2::new(text_x, rect.top() + 6.0), egui::Align2::LEFT_TOP, it.file_name(), egui::FontId::proportional(13.0), p.text);
                                let bar_x = rect.left() + 28.0;
                                let bar = egui::Rect::from_min_size(egui::Pos2::new(bar_x, rect.bottom() - 8.0), Vec2::new((rect.right() - bar_x - 4.0).max(0.0), 4.0));
                                if let Some(f) = it.fraction() {
                                    let col = p.status_color(&it.status);
                                    icons::progress_bar(ui.painter(), bar, f, col, p.border);
                                    ui.painter().text(
                                        egui::Pos2::new(bar.right(), bar.top() - 1.0),
                                        egui::Align2::RIGHT_BOTTOM,
                                        format!("{:.0}%", f * 100.0),
                                        egui::FontId::proportional(10.0),
                                        p.text_dim,
                                    );
                                }
                                if resp.clicked() {
                                    let (c, s) = ui.input(|i| (i.modifiers.ctrl || i.modifiers.command, i.modifiers.shift));
                                    click = Some((id, c, s));
                                }
                                if resp.double_clicked() {
                                    open_req = Some(id);
                                }
                                if resp.secondary_clicked() {
                                    ctx_menu = Some(id);
                                }
                            });

                            row.col(|ui| {
                                let t = match it.size {
                                    Some(sz) if it.status != Status::Done && it.downloaded > 0 => format!("{} of {}", size_human(it.downloaded), size_human(sz)),
                                    Some(sz) => size_human(sz),
                                    None => "Unknown".to_string(),
                                };
                                ui.label(RichText::new(t).size(12.0).color(p.text_dim));
                            });

                            row.col(|ui| {
                                let c = p.status_color(&it.status);
                                let (dr, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                                icons::dot(ui.painter(), dr, c);
                                let n = it.connections;
                                let label = match &it.note {
                                    Some(note) if it.status == Status::Running => {
                                        let mut c = note.chars();
                                        c.next().map(|f| format!("{}{}…", f.to_uppercase(), c.as_str())).unwrap_or_default()
                                    }
                                    _ if it.seeding() => {
                                        let t = it.torrent.as_ref().unwrap();
                                        format!("Seeding · ratio {:.2} · {} peer{}", it.ratio().unwrap_or(0.0), t.peers, if t.peers == 1 { "" } else { "s" })
                                    }
                                    _ if it.status == Status::Running && it.torrent.is_some() => {
                                        let p = it.torrent.as_ref().map_or(0, |t| t.peers);
                                        format!("Downloading · {p} peer{}", if p == 1 { "" } else { "s" })
                                    }
                                    _ if it.status == Status::Running && n > 1 => format!("{} ({n})", it.status.label()),
                                    _ => it.status.label().to_string(),
                                };
                                ui.label(RichText::new(label).size(12.0).color(c)).on_hover_text(if n > 1 { format!("{n} connections") } else { String::new() });
                            });

                            row.col(|ui| {
                                let txt = match (&it.status, it.size, rate) {
                                    (Status::Running, Some(sz), Some(r)) if r > 1.0 && sz > it.downloaded => eta_human((sz - it.downloaded) as f64 / r),
                                    _ => String::new(),
                                };
                                ui.label(RichText::new(txt).size(12.0).color(p.text_dim));
                            });

                            row.col(|ui| {
                                let up = self.snap.up_rates.get(&it.id).copied().filter(|r| *r > 1.0);
                                let mut txt = if it.status == Status::Running { rate.map(rate_human).unwrap_or_default() } else { String::new() };
                                if let Some(u) = up {
                                    // A torrent's upload, beside its download (or alone, when seeding).
                                    txt = if txt.is_empty() { format!("↑ {}", rate_human(u)) } else { format!("{txt}  ↑ {}", rate_human(u)) };
                                }
                                let active = it.status == Status::Running || up.is_some();
                                ui.label(RichText::new(txt).size(12.0).color(if active { p.accent } else { p.text_dim }));
                            });

                            // Last Try, as IDM shows it.
                            row.col(|ui| {
                                let txt = it.last_try.map(|t| since_human(t, now)).unwrap_or_default();
                                ui.label(RichText::new(txt).size(11.5).color(p.text_dim));
                            });

                            row.col(|ui| {
                                let (txt, col) = match &it.status {
                                    Status::Failed(e) => (e.clone(), p.err),
                                    _ => (it.request.url.clone(), p.text_dim),
                                };
                                let mut tip = it.request.url.clone();
                                if let Some(r) = &it.request.referer {
                                    tip.push_str("\nReferer: ");
                                    tip.push_str(r);
                                }
                                if it.added > 0 {
                                    tip.push_str(&format!("\nAdded {}", since_human(it.added, now)));
                                }
                                tip.push_str(&format!("\nSaved to {}", it.path.display()));
                                ui.label(RichText::new(txt).size(11.0).color(col)).on_hover_text(tip);
                            });
                        });
                    });

                if let Some(k) = sort_click {
                    if self.sort == k {
                        self.desc = !self.desc;
                    } else {
                        self.sort = k;
                        self.desc = false;
                    }
                }
                if toggle_all {
                    let visible: HashSet<u64> = rows.iter().map(|&i| snap.items[i].id).collect();
                    if header_check == Some(true) {
                        self.selected.retain(|id| !visible.contains(id));
                    } else {
                        self.selected.extend(visible);
                    }
                }
                self.vm.rows = rows;
                if let Some(id) = check_click {
                    self.toggle_select(id);
                }
                if let Some((id, ctrl, shift)) = click {
                    if shift {
                        self.select_range(id);
                    } else if ctrl {
                        self.toggle_select(id);
                    } else {
                        self.select_only(id);
                    }
                }
                if let Some(id) = open_req {
                    // IDM: a finished download opens; an unfinished one shows its progress.
                    if self.item(id).is_some_and(|i| i.status == Status::Done) {
                        self.open_id(id, false);
                    } else {
                        self.open_props(id);
                    }
                }
                if let Some(id) = ctx_menu {
                    if !self.selected.contains(&id) {
                        self.select_only(id);
                    }
                    ctx.memory_mut(|m| m.data.insert_temp(egui::Id::new("ctxmenu-for"), (id, ctx.input(|i| i.pointer.hover_pos()).unwrap_or_default())));
                }
                if let Some((id, pos)) = ctx.memory(|m| m.data.get_temp::<(u64, egui::Pos2)>(egui::Id::new("ctxmenu-for"))) {
                    match self.item(id).cloned() {
                        Some(it) => {
                            let area = egui::Area::new(egui::Id::new("ctxmenu")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    ui.set_min_width(220.0);
                                    let mut pick = |ui: &mut egui::Ui, label: &str, cmd: CtxCmd| {
                                        if ui.button(label).clicked() {
                                            ctx_cmd = Some(cmd);
                                        }
                                    };
                                    if it.status == Status::Done {
                                        pick(ui, "Open file", CtxCmd::Open);
                                    }
                                    pick(ui, "Open containing folder", CtxCmd::Folder);
                                    ui.separator();
                                    if it.status.is_active() {
                                        pick(ui, "Pause", CtxCmd::Stop);
                                    }
                                    if it.status.is_stopped() {
                                        pick(ui, "Resume", CtxCmd::Start);
                                    }
                                    if it.status == Status::Done && it.torrent.is_some() {
                                        if it.seeding() {
                                            pick(ui, "Stop seeding", CtxCmd::Seed(false));
                                        } else {
                                            pick(ui, "Seed again", CtxCmd::Seed(true));
                                        }
                                    }
                                    pick(ui, "Restart from the beginning…", CtxCmd::Restart);
                                    if it.status != Status::Done && it.torrent.is_none() && it.request.media.is_none() {
                                        pick(ui, "Refresh download address…", CtxCmd::Refresh);
                                    }
                                    ui.separator();
                                    if it.scheduled {
                                        pick(ui, "Remove from the scheduled queue", CtxCmd::Unqueue);
                                    } else if it.status != Status::Done {
                                        pick(ui, "Add to the scheduled queue", CtxCmd::Queue);
                                    }
                                    pick(ui, "Copy address", CtxCmd::CopyUrl(it.request.url.clone()));
                                    pick(ui, "Properties…", CtxCmd::Props);
                                    ui.separator();
                                    pick(ui, "Remove from list", CtxCmd::Remove);
                                    pick(ui, "Remove and delete the file…", CtxCmd::RemoveFile);
                                });
                            });
                            // Close on any click elsewhere, or once a command was picked.
                            let clicked_away = ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && ctx_menu.is_none();
                            if ctx_cmd.is_some() || clicked_away || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                                ctx.memory_mut(|m| m.data.remove::<(u64, egui::Pos2)>(egui::Id::new("ctxmenu-for")));
                            }
                        }
                        None => ctx.memory_mut(|m| m.data.remove::<(u64, egui::Pos2)>(egui::Id::new("ctxmenu-for"))),
                    }
                }
                match ctx_cmd {
                    Some(CtxCmd::Open) => self.open_selected(false),
                    Some(CtxCmd::Folder) => self.open_selected(true),
                    Some(CtxCmd::Stop) => self.stop_selected(),
                    Some(CtxCmd::Start) => self.start_selected(),
                    Some(CtxCmd::Restart) => self.restart_selected(),
                    Some(CtxCmd::Refresh) => {
                        if let Some(id) = self.focus {
                            self.open_refresh(id);
                        }
                    }
                    Some(CtxCmd::Seed(on)) => {
                        let ids: Vec<u64> = self.selected_items().iter().filter(|i| i.status == Status::Done && i.torrent.is_some()).map(|i| i.id).collect();
                        self.bulk(ids, if on { "seed" } else { "stop-seeding" }, move |n| if on { format!("Sharing {n} torrent(s) again") } else { format!("Stopped sharing {n} torrent(s)") });
                    }
                    Some(CtxCmd::Queue) => self.set_queued(true),
                    Some(CtxCmd::Unqueue) => self.set_queued(false),
                    Some(CtxCmd::Props) => {
                        if let Some(id) = self.focus {
                            self.open_props(id);
                        }
                    }
                    Some(CtxCmd::CopyUrl(u)) => {
                        ctx.copy_text(u);
                        self.toast("Address copied");
                    }
                    Some(CtxCmd::Remove) => {
                        let ids = self.selected_ids();
                        self.remove_ids(&ids, false);
                    }
                    // Deleting files is asked about first, in the same dialog as Remove.
                    Some(CtxCmd::RemoveFile) => self.ask_remove(),
                    None => {}
                }
            });
    }
}

enum CtxCmd {
    Open,
    Folder,
    Stop,
    Start,
    Restart,
    Refresh,
    Queue,
    Unqueue,
    Props,
    CopyUrl(String),
    Remove,
    RemoveFile,
    /// Start (`true`) or stop sharing finished torrents.
    Seed(bool),
}

// ── dialogs ──────────────────────────────────────────────────────────────

fn window(title: &str) -> egui::Window<'_> {
    egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
}

fn label(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).color(p.text_dim).size(12.0));
}

/// A segment map: the whole file as a bar, each range's downloaded part filled, the ranges a
/// connection is working on marked at their write position.
fn segment_map(ui: &mut egui::Ui, p: &Palette, it: &Item, segments: &[reidm_core::Segment]) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, p.border.gamma_multiply(0.5));
    let total = it.size.unwrap_or(0);
    if it.status == Status::Done {
        painter.rect_filled(rect, 3.0, p.ok);
        return;
    }
    if total == 0 || segments.is_empty() {
        if let Some(f) = it.fraction() {
            painter.rect_filled(egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * f, rect.height())), 3.0, p.accent);
        }
        return;
    }
    let x = |b: u64| rect.left() + rect.width() * (b.min(total) as f32 / total as f32);
    for s in segments {
        let end = if s.end == u64::MAX { total } else { s.end + 1 };
        let filled = if s.finished { end } else { (s.start + s.done).min(end) };
        if filled > s.start {
            painter.rect_filled(egui::Rect::from_x_y_ranges(x(s.start)..=x(filled), rect.y_range()), 0.0, if s.active { p.accent } else { p.accent.gamma_multiply(0.75) });
        }
        // Range boundaries, so the split is visible.
        painter.line_segment([egui::Pos2::new(x(s.start), rect.top()), egui::Pos2::new(x(s.start), rect.bottom())], egui::Stroke::new(1.0, p.panel_alt));
        if s.active {
            painter.line_segment([egui::Pos2::new(x(filled), rect.top() - 2.0), egui::Pos2::new(x(filled), rect.bottom() + 2.0)], egui::Stroke::new(2.0, p.warn));
        }
    }
    painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
}

impl App {
    fn add_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.add.open {
            return;
        }
        let mut open = true;
        let mut submit: Option<bool> = None; // Some(later)
        window("Add a download").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(540.0);
            label(ui, p, "Addresses — one per line, so you can paste a whole batch");
            ui.add_sized(
                Vec2::new(520.0, 96.0),
                egui::TextEdit::multiline(&mut self.add.urls).hint_text("https://example.com/file.zip\nhttps://example.com/another.iso").desired_rows(4),
            );
            let count = self.add.count();
            if count > 1 {
                ui.label(RichText::new(format!("{count} URLs")).color(p.accent).size(11.0));
            }
            ui.add_space(6.0);
            label(ui, p, "Save as (single URL only)");
            ui.add_sized(Vec2::new(520.0, 26.0), egui::TextEdit::singleline(&mut self.add.name).hint_text("(leave blank to use the name from the server)"));
            ui.add_space(6.0);
            label(ui, p, "Save to");
            ui.horizontal(|ui| {
                ui.add_sized(Vec2::new(430.0, 26.0), egui::TextEdit::singleline(&mut self.add.dir));
                if ui.button("Browse…").clicked() {
                    if let Some(d) = rfd::FileDialog::new().set_directory(&self.add.dir).pick_folder() {
                        self.add.dir = d.to_string_lossy().into_owned();
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Import list…").clicked() {
                    self.import_from_file();
                }
                if ui.button("Batch with a pattern…").clicked() {
                    self.batch.open = true;
                }
                label(ui, p, "or paste links — the clipboard is read when this opens");
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add_sized(Vec2::new(150.0, 28.0), egui::Button::new(RichText::new(if count > 1 { "Start all" } else { "Start download" }).strong())).clicked() {
                    submit = Some(false);
                }
                if ui.add_sized(Vec2::new(130.0, 28.0), egui::Button::new("Download later")).on_hover_text("Add to the scheduled queue without starting").clicked() {
                    submit = Some(true);
                }
                if ui.add_sized(Vec2::new(80.0, 28.0), egui::Button::new("Cancel")).clicked() {
                    self.add.open = false;
                }
            });
        });
        if let Some(later) = submit {
            let urls = reidm_core::grabber::parse_url_list(&self.add.urls);
            let dir = Some(self.add.dir.trim().to_string()).filter(|d| !d.is_empty() && self.snap.service.settings.as_ref().is_none_or(|s| s.default_dir != *d));
            if urls.is_empty() {
                self.toast("Enter an http or https address first");
            } else if urls.len() == 1 {
                let name = Some(self.add.name.trim().to_string()).filter(|n| !n.is_empty());
                let body = serde_json::json!({ "url": urls[0], "filename": name, "dir": dir, "later": later });
                self.add = AddState::default();
                self.cmd.run(move |c| {
                    let r = c.post("/api/downloads", body);
                    reply(move |app: &mut App| match r {
                        Ok(_) => app.toast(if later { "Added to the scheduled queue" } else { "Download started" }),
                        Err(e) => app.toast(format!("Could not add it: {e}")),
                    })
                });
            } else {
                self.queue_urls(urls, later);
                self.add = AddState::default();
            }
        }
        if !open {
            self.add.open = false;
        }
    }

    fn batch_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.batch.open {
            return;
        }
        let preview = self.batch.expand();
        let mut open = true;
        let mut submit: Option<bool> = None;
        window("Add batch download").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(560.0);
            label(ui, p, "Address with an asterisk (*) where the number or letter goes");
            ui.add_sized(Vec2::new(540.0, 26.0), egui::TextEdit::singleline(&mut self.batch.pattern).hint_text("https://example.com/photos/img*.jpg"));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                label(ui, p, "Replace * from");
                ui.add_sized(Vec2::new(70.0, 24.0), egui::TextEdit::singleline(&mut self.batch.from));
                label(ui, p, "to");
                ui.add_sized(Vec2::new(70.0, 24.0), egui::TextEdit::singleline(&mut self.batch.to));
                label(ui, p, "zero-pad to");
                ui.add(egui::DragValue::new(&mut self.batch.width).range(0..=8));
                label(ui, p, "digits");
            });
            label(ui, p, "Numbers (1 to 50) or letters (a to z). Zero-padding to 3 digits gives 001, 002 …");
            ui.add_space(8.0);
            match &preview {
                Ok(urls) if !urls.is_empty() => {
                    ui.label(RichText::new(format!("{} addresses", urls.len())).color(p.accent));
                    ui.label(RichText::new(&urls[0]).size(11.5).color(p.text_dim));
                    if urls.len() > 2 {
                        ui.label(RichText::new("…").size(11.5).color(p.text_dim));
                    }
                    if urls.len() > 1 {
                        ui.label(RichText::new(urls.last().unwrap()).size(11.5).color(p.text_dim));
                    }
                }
                Ok(_) => {}
                Err(e) if !self.batch.pattern.is_empty() => {
                    ui.label(RichText::new(e).color(p.err).size(12.0));
                }
                Err(_) => {}
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ok = preview.as_ref().is_ok_and(|u| !u.is_empty());
                if ui.add_enabled(ok, egui::Button::new(RichText::new("Start all").strong())).clicked() {
                    submit = Some(false);
                }
                if ui.add_enabled(ok, egui::Button::new("Download later")).clicked() {
                    submit = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    self.batch.open = false;
                }
            });
        });
        if let (Some(later), Ok(urls)) = (submit, preview) {
            self.queue_urls(urls, later);
            self.batch = BatchState::default();
        }
        if !open {
            self.batch.open = false;
        }
    }

    fn options_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.show_options {
            return;
        }
        let mut open = true;
        let mut save = false;
        let mut limiter = false;
        let mut integration = false;
        window("Options").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(560.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.opt_tab, OptTab::General, "General");
                ui.selectable_value(&mut self.opt_tab, OptTab::Folders, "Folders");
                ui.selectable_value(&mut self.opt_tab, OptTab::Torrents, "Torrents");
            });
            ui.separator();
            if self.opt_tab == OptTab::Folders {
                self.folders_tab(ui, p);
            } else if self.opt_tab == OptTab::Torrents {
                self.torrents_tab(ui, p);
            } else {
            ui.label(RichText::new("Downloads").strong());
            ui.add_space(6.0);
            label(ui, p, "Default download folder");
            ui.horizontal(|ui| {
                ui.add_sized(Vec2::new(380.0, 26.0), egui::TextEdit::singleline(&mut self.opt.dir));
                if ui.button("Browse…").clicked() {
                    if let Some(d) = rfd::FileDialog::new().set_directory(&self.opt.dir).pick_folder() {
                        self.opt.dir = d.to_string_lossy().into_owned();
                    }
                }
            });
            ui.add_space(8.0);
            label(ui, p, "Connections per download");
            ui.add(egui::Slider::new(&mut self.opt.conn, crate::options::CONNECTIONS));
            label(ui, p, "When a connection finishes early it takes over half of the slowest one's remaining part, so more connections rarely hurt. 8 is a good default.");
            ui.add_space(8.0);
            label(ui, p, "Downloads at the same time");
            ui.add(egui::Slider::new(&mut self.opt.max, crate::options::AT_ONCE));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Speed limiter…").clicked() {
                    limiter = true;
                }
                if ui.button("Scheduler…").clicked() {
                    self.open_scheduler();
                }
            });
            ui.add_space(12.0);
            ui.label(RichText::new("Window").strong());
            ui.add_space(4.0);
            ui.checkbox(&mut self.prefs.dark, "Dark theme");
            ui.checkbox(&mut self.prefs.notify_complete, "Show a window when a download completes");
            ui.checkbox(&mut self.prefs.clipboard, "Fill Add URL with links from the clipboard");
            ui.add_space(12.0);
            ui.label(RichText::new("Browser").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                match self.snap.service.extension.clone().filter(|e| e.seconds_ago < 180) {
                    Some(e) => ui.label(RichText::new(format!("Connected: reIDM extension {} in {}", e.version, e.browser)).color(p.ok)),
                    None => ui.label(RichText::new("reIDM's extension is not connected").color(p.warn)),
                };
                if ui.button("Set up…").clicked() {
                    integration = true;
                }
            });
            ui.add_space(12.0);
            ui.label(RichText::new("Updates").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Check now").clicked() {
                    self.check_for_updates();
                }
                label(ui, p, "Uses the GitHub Releases API; nothing is downloaded or installed by itself.");
            });
            }
            ui.add_space(14.0);
            let loaded = self.opt_base.is_some();
            if !loaded {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Loading the service's settings… they can be saved once they have loaded.").color(p.warn));
                });
            }
            ui.horizontal(|ui| {
                // Without the settings as they are, Save would have nothing to compare with and
                // could only write the defaults shown over the user's own.
                if ui.add_enabled(loaded, egui::Button::new("Save").min_size(Vec2::new(90.0, 28.0))).clicked() {
                    save = true;
                }
                if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Cancel")).clicked() {
                    self.show_options = false;
                }
            });
        });
        if save {
            // The window's own preferences save at once; the service's, only what changed.
            self.prefs.save();
            let patch = self.opt_base.as_ref().map(|b| self.opt.changes(b)).unwrap_or_default();
            self.show_options = false;
            if patch.is_empty() {
                self.toast("Options saved");
            } else {
                self.save_settings(serde_json::Value::Object(patch), |app, r| match r {
                    Ok(()) => app.toast("Options saved"),
                    Err(e) => {
                        // Back to the dialog, as edited, to put it right.
                        app.show_options = true;
                        app.toast(format!("Could not save: {e}"));
                    }
                });
            }
        }
        if limiter {
            self.open_limiter();
        }
        if integration {
            self.show_integration = true;
        }
        if !open {
            self.show_options = false;
        }
    }

    /// Options › Torrents: how long to share, how fast, and how peers find reIDM.
    fn torrents_tab(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.label(RichText::new("When a torrent is complete").strong());
        let seed_label = |r: f64| if r == 0.0 { "Stop: don't seed".to_string() } else if r < 0.0 { "Seed for as long as reIDM runs".into() } else { format!("Seed until it has shared {r}× its size") };
        egui::ComboBox::from_id_salt("seed").width(330.0).selected_text(seed_label(self.opt.seed)).show_ui(ui, |ui| {
            for r in [0.0, 0.5, 1.0, 2.0, 5.0, -1.0] {
                ui.selectable_value(&mut self.opt.seed, r, seed_label(r));
            }
        });
        label(ui, p, "Seeding is sharing what you downloaded with others, which is how torrents work. Stop or restart it for one torrent from its right-click menu.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label("Upload limit");
            ui.add(egui::DragValue::new(&mut self.opt.up_kb).range(0..=10_000_000).suffix(" KB/s"));
            label(ui, p, if self.opt.up_kb == 0 { "(none)" } else { "" });
        });
        label(ui, p, "The download limit is the speed limiter's, shared with every other download.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label("Listening port");
            ui.add(egui::DragValue::new(&mut self.opt.port).range(0..=65535));
        });
        label(ui, p, "Where other peers connect to you; reIDM asks your router to forward it. 0 picks any free port. Takes effect when reIDM restarts.");
        ui.add_space(6.0);
        ui.checkbox(&mut self.opt.dht, "Find peers through the DHT, not only through trackers");
        label(ui, p, "Needed for most magnet links. Takes effect when reIDM restarts.");
    }

    /// Options › Folders: where each type of file goes, and where unfinished downloads wait.
    fn folders_tab(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.checkbox(&mut self.opt.sort, "Save each download in the folder for its type");
        label(ui, p, "As IDM does: videos in Video, archives in Compressed, installers in Programs… A folder left empty is the one under the download folder shown in grey; anything else goes to the download folder itself.");
        ui.add_space(8.0);
        let defaults: Vec<String> = self.opt.folders.iter().map(|(n, _)| self.default_folder(n)).collect();
        egui::Grid::new("folders").num_columns(4).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
            for ((name, dir), default) in self.opt.folders.iter_mut().zip(&defaults) {
                let name: &'static str = name;
                let on = self.opt.sort || name == "General";
                ui.add_enabled_ui(on, |ui| ui.label(if name == "General" { "Everything else" } else { name }));
                ui.add_enabled_ui(on, |ui| ui.add_sized(Vec2::new(330.0, 22.0), egui::TextEdit::singleline(dir).hint_text(default.as_str())));
                if ui.add_enabled(on, egui::Button::new("Browse…")).clicked() {
                    let start = if dir.is_empty() { default.clone() } else { dir.clone() };
                    if let Some(d) = rfd::FileDialog::new().set_directory(&start).pick_folder() {
                        *dir = d.to_string_lossy().into_owned();
                    }
                }
                if ui.add_enabled(!dir.is_empty(), egui::Button::new("Default")).on_hover_text("Back to the folder under the download folder").clicked() {
                    dir.clear();
                }
                ui.end_row();
            }
        });
        ui.add_space(12.0);
        ui.label(RichText::new("Temporary folder").strong());
        label(ui, p, "Unfinished downloads wait here, and move to their folder when complete — so the download folder only ever holds finished files. On another drive than the download folder, the move is a copy.");
        let temp_default = reidm_core::default_data_dir().join("parts").to_string_lossy().into_owned();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.opt.temp).hint_text(temp_default.as_str()).desired_width(330.0));
            if ui.button("Browse…").clicked() {
                let start = if self.opt.temp.is_empty() { temp_default.clone() } else { self.opt.temp.clone() };
                if let Some(d) = rfd::FileDialog::new().set_directory(&start).pick_folder() {
                    self.opt.temp = d.to_string_lossy().into_owned();
                }
            }
            if ui.add_enabled(!self.opt.temp.is_empty(), egui::Button::new("Default")).clicked() {
                self.opt.temp.clear();
            }
        });
    }

    fn about_dialog(&mut self, ctx: &egui::Context) {
        if !self.about {
            return;
        }
        let mut open = true;
        let mut do_check = false;
        window("About reIDM").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.label(RichText::new(format!("reIDM {}", env!("CARGO_PKG_VERSION"))).strong().size(16.0));
            ui.add_space(6.0);
            ui.label("A cross-platform download manager that takes over your browser's downloads and speaks the IDM browser-extension protocol.");
            ui.add_space(8.0);
            if let Some(u) = &self.update {
                if u.checking {
                    ui.label(RichText::new("Checking for updates…").color(egui::Color32::from_gray(140)).size(11.0));
                } else if !u.version.is_empty() {
                    ui.label(RichText::new(format!("Version {} is available.", u.version)).color(egui::Color32::from_rgb(0x2A, 0x84, 0xE0)).size(12.0));
                } else if let Some(e) = &u.error {
                    ui.label(RichText::new(format!("Update check failed: {e}")).color(egui::Color32::from_rgb(0xB0, 0x6A, 0x00)).size(11.0));
                }
            }
            if !self.snap.service.version.is_empty() && self.snap.service.version != env!("CARGO_PKG_VERSION") {
                ui.label(RichText::new(format!("The download service running is version {}; restart it to use this one.", self.snap.service.version)).color(egui::Color32::from_rgb(0xB0, 0x6A, 0x00)).size(11.0));
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Not affiliated with, endorsed by, or derived from Tonec Inc.").color(egui::Color32::from_gray(140)).size(11.0));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Check for updates").clicked() {
                    do_check = true;
                }
                if ui.button("Close").clicked() {
                    self.about = false;
                }
            });
        });
        if do_check {
            self.check_for_updates();
        }
        if !open {
            self.about = false;
        }
    }

    /// The site-grabber window: scan a page, pick what you want, queue it.
    fn grabber_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.show_grabber {
            return;
        }
        let mut open = true;
        let mut scan = false;
        let mut export = false;
        let mut import = false;
        let mut queue: Option<(Vec<String>, bool)> = None;
        let found = std::mem::take(&mut self.grab.found);
        let mut selected = std::mem::take(&mut self.grab.selected);

        egui::Window::new("Site Grabber")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(760.0, 520.0))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                label(ui, p, "Page address");
                ui.horizontal(|ui| {
                    let r = ui.add_sized(Vec2::new(430.0, 26.0), egui::TextEdit::singleline(&mut self.grab.url).hint_text("https://example.com/downloads/"));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.add_enabled(!self.grab.scanning, egui::Button::new("Scan")).clicked() || enter) && !self.grab.scanning {
                        scan = true;
                    }
                    if self.grab.scanning {
                        ui.spinner();
                    }
                });
                label(ui, p, "For pages that build their links with script, or need you signed in, use the browser extension's Resource grabber instead.");

                ui.add_space(6.0);
                egui::CollapsingHeader::new("Filters").default_open(false).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        label(ui, p, "Follow links");
                        ui.add(egui::DragValue::new(&mut self.grab.depth).range(0..=3));
                        label(ui, p, if self.grab.depth == 0 { "this page only" } else { "levels deep" });
                        ui.separator();
                        ui.checkbox(&mut self.grab.same_host, "Same host only");
                        ui.checkbox(&mut self.grab.sizes, "Ask for file sizes (slower)");
                    });
                    ui.horizontal(|ui| {
                        label(ui, p, "Types");
                        ui.add_sized(Vec2::new(300.0, 22.0), egui::TextEdit::singleline(&mut self.grab.types).hint_text("zip, iso, mp4 … (blank = all common file types)"));
                    });
                    ui.horizontal(|ui| {
                        label(ui, p, "Include");
                        ui.add_sized(Vec2::new(230.0, 22.0), egui::TextEdit::singleline(&mut self.grab.include).hint_text("/pub/"));
                        label(ui, p, "Exclude");
                        ui.add_sized(Vec2::new(230.0, 22.0), egui::TextEdit::singleline(&mut self.grab.exclude).hint_text("sample"));
                    });
                });

                if let Some(e) = &self.grab.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).color(p.err).size(12.0));
                }

                ui.add_space(8.0);
                ui.separator();

                if found.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label(RichText::new("Scan a page to list the files it links to.\nNothing is downloaded until you say so.").color(p.text_dim));
                    });
                } else {
                    let all_on = selected.iter().all(|s| *s);
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut all_on.clone(), format!("{} files found", found.len())).clicked() {
                            for s in selected.iter_mut() {
                                *s = !all_on;
                            }
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Export list…").clicked() {
                                export = true;
                            }
                            if ui.button("Import list…").clicked() {
                                import = true;
                            }
                        });
                    });
                    ui.add_space(4.0);
                    // Only the rows in view are laid out: a deep scan can find thousands.
                    egui::ScrollArea::vertical().max_height(280.0).show_rows(ui, 22.0, found.len(), |ui, range| {
                        for (i, f) in found.iter().enumerate().skip(range.start).take(range.len()) {
                            ui.horizontal(|ui| {
                                if i < selected.len() {
                                    ui.checkbox(&mut selected[i], "");
                                }
                                let icon_rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::splat(16.0));
                                let cat = Category::of(&f.ext.to_ascii_uppercase());
                                icons::file_card(ui.painter(), icon_rect, cat, self.prefs.dark);
                                ui.add_space(20.0);
                                match f.size {
                                    Some(sz) => ui.label(RichText::new(size_human(sz)).size(11.5).color(p.text_dim)),
                                    None => ui.label(RichText::new("          ").size(11.5)),
                                };
                                ui.label(RichText::new(&f.filename).size(12.0).color(p.text))
                                    .on_hover_text(if f.found_on.is_empty() { f.url.clone() } else { format!("{}\nfound on {}", f.url, f.found_on) });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(RichText::new(&f.ext).size(10.5).color(p.text_dim));
                                });
                            });
                        }
                    });
                }

                ui.add_space(8.0);
                ui.separator();
                ui.horizontal(|ui| {
                    // Counted each frame, collected only when used.
                    let n = selected.iter().filter(|s| **s).count();
                    let chosen = || found.iter().zip(selected.iter()).filter(|(_, s)| **s).map(|(f, _)| f.url.clone()).collect::<Vec<String>>();
                    if ui.add_enabled(n > 0, egui::Button::new(format!("Download {n} selected"))).clicked() {
                        queue = Some((chosen(), false));
                    }
                    if ui.add_enabled(n > 0, egui::Button::new("Download later")).clicked() {
                        queue = Some((chosen(), true));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            self.show_grabber = false;
                        }
                    });
                });
            });

        if export {
            let all_urls: Vec<String> = found.iter().map(|f| f.url.clone()).collect();
            self.export_found(&all_urls);
        }
        self.grab.found = found;
        self.grab.selected = selected;
        if scan {
            self.start_grab();
        }
        if let Some((urls, later)) = queue {
            self.queue_urls(urls, later);
        }
        if import {
            self.import_from_file();
        }
        if !open {
            self.show_grabber = false;
        }
    }

    /// Ask before what can't be undone. Esc or closing the dialog cancels; Enter takes the
    /// harmless choice, never the one that deletes.
    fn confirm_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(c) = &self.confirm else { return };
        let name = |ids: &[u64]| match ids {
            [one] => format!("“{}”", self.item(*one).map(|i| i.file_name().to_string()).unwrap_or_default()),
            many => format!("{} downloads", many.len()),
        };
        let (title, text, detail) = match c {
            Confirm::Remove(ids) => ("Remove", format!("Remove {} from the list?", name(ids)), "Removing from the list keeps the files. Deleting the files can't be undone.".to_string()),
            Confirm::Restart(ids) => {
                let done = ids.iter().filter(|id| self.item(**id).is_some_and(|i| i.status == Status::Done)).count();
                let got: u64 = ids.iter().filter_map(|id| self.item(*id)).filter(|i| i.status != Status::Done).map(|i| i.downloaded).sum();
                let mut d = String::new();
                if got > 0 {
                    d.push_str(&format!("The {} already downloaded will be thrown away. ", size_human(got)));
                }
                if done > 0 {
                    d.push_str("A finished download is fetched again and replaces its file on disk.");
                }
                ("Restart", format!("Start {} again from the beginning?", name(ids)), d)
            }
            Confirm::ClearCompleted(ids) => ("Remove completed", format!("Remove all {} completed downloads from the list?", ids.len()), "The files stay where they are; only the list entries go. This can't be undone.".to_string()),
            Confirm::Close => (
                "reIDM keeps downloading",
                "Downloads carry on in the background when this window is closed.".to_string(),
                "The browser keeps handing downloads to reIDM, and opening reIDM again shows them. To stop everything, choose Quit reIDM completely (also in the File menu).".to_string(),
            ),
        };
        let (esc, enter) = ctx.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Enter)));
        let mut open = true;
        let mut choice: Option<&'static str> = None;
        window(title).open(&mut open).show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.label(RichText::new(&text).strong());
            if !detail.is_empty() {
                ui.add_space(4.0);
                label(ui, p, &detail);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let danger = |t: &str| egui::Button::new(RichText::new(t).color(egui::Color32::WHITE)).fill(p.err);
                match c {
                    Confirm::Remove(_) => {
                        if ui.button(RichText::new("Remove from list").strong()).clicked() || enter {
                            choice = Some("remove");
                        }
                        if ui.add(danger("Remove and delete the files")).clicked() {
                            choice = Some("purge");
                        }
                    }
                    Confirm::Restart(_) => {
                        if ui.add(danger("Restart")).clicked() {
                            choice = Some("restart");
                        }
                    }
                    Confirm::ClearCompleted(_) => {
                        if ui.add(danger("Remove them")).clicked() {
                            choice = Some("clear");
                        }
                    }
                    Confirm::Close => {
                        if ui.button(RichText::new("Keep downloading in the background").strong()).clicked() || enter {
                            choice = Some("close");
                        }
                        if ui.add(danger("Quit reIDM completely")).clicked() {
                            choice = Some("quit");
                        }
                    }
                }
                if ui.button("Cancel").clicked() || esc {
                    choice = Some("cancel");
                }
            });
        });
        if !open {
            choice = Some("cancel");
        }
        let Some(choice) = choice else { return };
        let Some(c) = self.confirm.take() else { return };
        match (c, choice) {
            (Confirm::Remove(ids), "remove") => self.remove_ids(&ids, false),
            (Confirm::Remove(ids), "purge") => self.remove_ids(&ids, true),
            (Confirm::Restart(ids), "restart") => self.bulk(ids, "restart", |n| format!("Restarted {n} download(s) from the beginning")),
            (Confirm::ClearCompleted(ids), "clear") => {
                for id in &ids {
                    self.selected.remove(id);
                }
                self.bulk(ids, "remove", |n| format!("Removed {n} completed download(s) from the list"));
            }
            (Confirm::Close, "close") => {
                self.prefs.close_explained = true;
                self.prefs.save();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            (Confirm::Close, "quit") => self.quit_completely(ctx),
            _ => {}
        }
    }

    /// One download's details: addresses, sizes, its connections as a segment map, and its own
    /// speed limit. IDM shows this as the download progress dialog.
    fn props_window(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(st) = &self.props else { return };
        let id = st.id;
        let Some(it) = self.item(id).cloned() else {
            self.props = None;
            return;
        };
        // The list row is live; the full copy (segments, files) is at most a second old.
        let detail = st.detail.clone();
        let rate = self.snap.rates.get(&id).copied();
        let mut open = true;
        let mut apply_limit: Option<Option<u64>> = None;
        let mut cmd: Option<&'static str> = None;
        let mut refresh = false;
        let mut copy = false;
        let (mut limit_on, mut limit_kb) = (st.limit_on, st.limit_kb);
        egui::Window::new(it.file_name())
            .id(egui::Id::new("props"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(ctx.content_rect().center() - Vec2::new(280.0, 200.0))
            .show(ctx, |ui| {
                ui.set_min_width(560.0);
                egui::Grid::new("props-grid").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    let row = |ui: &mut egui::Ui, k: &str, v: String| {
                        label(ui, p, k);
                        ui.add(egui::Label::new(RichText::new(v).color(p.text)).truncate());
                        ui.end_row();
                    };
                    label(ui, p, "Address");
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(RichText::new(&it.request.url).color(p.text)).truncate());
                        if ui.small_button("Copy").clicked() {
                            copy = true;
                        }
                    });
                    ui.end_row();
                    if let Some(r) = &it.request.referer {
                        row(ui, "Referer", r.clone());
                    }
                    if let Some(ua) = detail.as_ref().and_then(|d| d.request.user_agent.clone()) {
                        row(ui, "Sent as", ua);
                    }
                    if let Some(m) = &it.request.media {
                        row(ui, "Fetched with", format!("yt-dlp, format {}", m.format));
                    }
                    row(ui, "Saved to", it.path.display().to_string());
                    let status = match &it.status {
                        Status::Failed(e) => format!("Error: {e}"),
                        s => s.label().to_string(),
                    };
                    row(ui, "Status", status);
                    let size = match it.size {
                        Some(sz) => format!("{} ({} bytes)", size_human(sz), sz),
                        None => "Unknown — the server did not say".into(),
                    };
                    row(ui, "File size", size);
                    let got = match (it.size, it.fraction()) {
                        (Some(_), Some(f)) => format!("{} ({:.1}%)", size_human(it.downloaded), f * 100.0),
                        _ => size_human(it.downloaded),
                    };
                    row(ui, "Downloaded", got);
                    if it.status == Status::Running {
                        let r = rate.unwrap_or(0.0);
                        let left = it.size.filter(|s| *s > it.downloaded && r > 1.0).map(|s| eta_human((s - it.downloaded) as f64 / r)).unwrap_or_default();
                        row(ui, "Transfer rate", format!("{}   {}", rate_human(r), if left.is_empty() { String::new() } else { format!("time left {left}") }));
                    }
                    if let Some(t) = it.finished_at {
                        row(ui, "Completed", since_human(t, now_secs()));
                    }
                    if let Some(t) = &it.torrent {
                        row(ui, "Torrent", format!("{} · info hash {}", t.name, t.info_hash));
                        row(ui, "Shared", format!("{} uploaded · ratio {:.2}{}", size_human(t.uploaded), it.ratio().unwrap_or(0.0), if it.seeding() { " · seeding" } else { "" }));
                        row(ui, "Peers", format!("{} connected", t.peers));
                        row(ui, "Resume capability", "Yes — a torrent carries on where it stopped".into());
                    } else if it.request.media.is_some() {
                        row(ui, "Resume capability", "Yes — yt-dlp carries on where it stopped".into());
                    } else {
                        row(ui, "Resume capability", if it.resumable { "Yes".into() } else if it.size.is_some() { "No — a stop means starting over".into() } else { "Unknown".into() });
                        row(ui, "Connections", format!("{} working on {} part(s)", it.connections, it.parts.max(1)));
                    }
                });
                ui.add_space(8.0);
                segment_map(ui, p, &it, detail.as_ref().map(|d| d.segments()).unwrap_or_default());
                // A torrent's files, each with its own progress.
                if let Some(t) = detail.as_ref().and_then(|d| d.torrent.as_ref()).filter(|t| t.files.len() > 1 || t.files.iter().any(|f| !f.selected)) {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Files").strong());
                    // Only the files in view are laid out: a torrent can hold tens of thousands.
                    egui::ScrollArea::vertical().max_height(160.0).show_rows(ui, 18.0, t.files.len(), |ui, range| {
                        egui::Grid::new("torrent-files").num_columns(3).spacing(Vec2::new(10.0, 3.0)).min_row_height(15.0).show(ui, |ui| {
                            for f in &t.files[range] {
                                ui.label(RichText::new(&f.path).size(12.0).color(if f.selected { p.text } else { p.text_dim }));
                                ui.label(RichText::new(size_human(f.size)).size(12.0).color(p.text_dim));
                                let txt = if !f.selected { "skipped".to_string() } else if f.size == 0 { "100%".into() } else { format!("{:.0}%", f.done as f64 * 100.0 / f.size as f64) };
                                ui.label(RichText::new(txt).size(12.0));
                                ui.end_row();
                            }
                        });
                    });
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut limit_on, "Limit this download to");
                    ui.add_enabled(limit_on, egui::DragValue::new(&mut limit_kb).range(1..=1_000_000).suffix(" KB/s"));
                    if ui.button("Apply").clicked() {
                        apply_limit = Some(limit_on.then(|| limit_kb as u64 * 1024));
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if it.status.is_active() && ui.button("Pause").clicked() {
                        cmd = Some("pause");
                    }
                    if it.status.is_stopped() && ui.button("Resume").clicked() {
                        cmd = Some("resume");
                    }
                    if it.status == Status::Done && ui.button("Open").clicked() {
                        cmd = Some("open");
                    }
                    if ui.button("Open folder").clicked() {
                        cmd = Some("folder");
                    }
                    if it.status != Status::Done && ui.button("Refresh address…").clicked() {
                        refresh = true;
                    }
                    if ui.button("Restart…").clicked() {
                        cmd = Some("restart");
                    }
                });
            });
        if let Some(st) = &mut self.props {
            st.limit_on = limit_on;
            st.limit_kb = limit_kb;
        }
        if copy {
            ctx.copy_text(it.request.url.clone());
            self.toast("Address copied");
        }
        if let Some(l) = apply_limit {
            self.cmd.run(move |c| {
                let r = c.post(&format!("/api/downloads/{id}/limit"), serde_json::json!({ "limit": l }));
                reply(move |app: &mut App| match r {
                    Ok(_) => app.toast(match l {
                        Some(b) => format!("This download is limited to {}", rate_human(b as f64)),
                        None => "This download's own limit is off".into(),
                    }),
                    Err(e) => app.toast(format!("Could not set the limit: {e}")),
                })
            });
        }
        if let Some(c) = cmd {
            if c == "restart" {
                self.confirm = Some(Confirm::Restart(vec![id]));
            } else {
                self.action(id, c);
            }
        }
        if refresh {
            self.open_refresh(id);
        }
        if !open {
            self.props = None;
        }
    }

    fn refresh_window(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(r) = &mut self.refresh else { return };
        let id = r.id;
        let name = self.snap.item(id).map(|i| i.file_name().to_string()).unwrap_or_default();
        let mut open = true;
        let mut submit = false;
        let mut capture = false;
        let mut cancel_capture = false;
        window("Refresh download address").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(540.0);
            ui.label(RichText::new(&name).strong());
            ui.add_space(4.0);
            label(ui, p, "When a link expires, give the download the file's new address and it carries on where it stopped. reIDM checks it is the same file (the same size) first.");
            ui.add_space(8.0);
            if r.capturing {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Waiting for the browser: open the download page again and click the link. The browser's download comes here instead of starting anew.");
                });
                if ui.button("Stop waiting").clicked() {
                    cancel_capture = true;
                }
            } else {
                label(ui, p, "New address");
                ui.add_sized(Vec2::new(520.0, 26.0), egui::TextEdit::singleline(&mut r.url).hint_text("https://…"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let busy = r.busy.is_some();
                    if ui.add_enabled(!busy && r.url.trim().starts_with("http"), egui::Button::new(RichText::new("Use this address").strong())).clicked() {
                        submit = true;
                    }
                    if busy {
                        ui.spinner();
                        label(ui, p, "checking the new address…");
                    }
                    ui.separator();
                    if ui.add_enabled(!busy, egui::Button::new("Capture it from the browser…")).on_hover_text("The next download the browser hands over becomes this download's address").clicked() {
                        capture = true;
                    }
                });
            }
            if let Some(e) = &r.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(p.err).size(12.0));
            }
        });
        if submit {
            r.error = None;
            r.busy = Some(self.client.refresh_async(id, r.url.trim()));
        }
        if capture {
            r.error = None;
            r.capturing = true;
            self.cmd.run(move |c| {
                let res = c.post(&format!("/api/downloads/{id}/refresh-capture"), serde_json::json!({}));
                res.err().and_then(|e| {
                    reply(move |app: &mut App| {
                        if let Some(r) = app.refresh.as_mut().filter(|r| r.id == id) {
                            r.capturing = false;
                            r.error = Some(e);
                        }
                    })
                })
            });
        }
        let cancel = cancel_capture || (!open && self.refresh.as_ref().is_some_and(|r| r.capturing));
        if cancel_capture {
            if let Some(r) = &mut self.refresh {
                r.capturing = false;
            }
        }
        if cancel {
            self.cmd.run(|c| {
                let r = c.post("/api/refresh-capture/cancel", serde_json::json!({}));
                r.err().and_then(|e| reply(move |app: &mut App| app.toast(format!("Could not stop waiting for the browser: {e}"))))
            });
        }
        if !open {
            self.refresh = None;
        }
    }

    fn scheduler_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.sched.open {
            return;
        }
        let mut open = true;
        let mut save = false;
        let mut queue: Option<bool> = None;
        let in_queue = self.vm.counts.queued;
        let s = &mut self.sched;
        window("Scheduler").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(470.0);
            ui.checkbox(&mut s.enabled, RichText::new("Run the scheduled queue on a schedule").strong());
            ui.add_space(8.0);
            ui.add_enabled_ui(s.enabled, |ui| {
                ui.horizontal(|ui| {
                    label(ui, p, "Start at");
                    ui.add_sized(Vec2::new(70.0, 24.0), egui::TextEdit::singleline(&mut s.start).hint_text("02:00"));
                    ui.add_space(12.0);
                    ui.checkbox(&mut s.use_stop, "Stop at");
                    ui.add_enabled(s.use_stop, egui::TextEdit::singleline(&mut s.stop).desired_width(70.0).hint_text("07:00"));
                });
                label(ui, p, "Local time, 24-hour clock. A stop time earlier than the start time means the next morning.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    label(ui, p, "On");
                    for (i, d) in DAYS.iter().enumerate() {
                        ui.toggle_value(&mut s.days[i], *d);
                    }
                });
            });
            ui.add_space(10.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{in_queue} download(s) in the scheduled queue")).color(p.text_dim));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Stop now").clicked() {
                        queue = Some(false);
                    }
                    if ui.button("Start now").clicked() {
                        queue = Some(true);
                    }
                });
            });
            label(ui, p, "Put downloads in the queue with Download Later, or right-click › Add to the scheduled queue. They start up to the 'downloads at the same time' limit.");
            if let Some(e) = &s.error {
                ui.label(RichText::new(e).color(p.err).size(12.0));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Save")).clicked() {
                    save = true;
                }
                if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Cancel")).clicked() {
                    s.open = false;
                }
            });
        });
        if save {
            let days: Vec<u8> = if s.days.iter().all(|d| *d) { vec![] } else { (0..7u8).filter(|d| s.days[*d as usize]).collect() };
            let sched = Schedule {
                enabled: s.enabled,
                start: Some(s.start.trim().to_string()).filter(|t| !t.is_empty()),
                stop: if s.use_stop { Some(s.stop.trim().to_string()).filter(|t| !t.is_empty()) } else { None },
                days,
            };
            if s.enabled && s.days.iter().all(|d| !*d) {
                s.error = Some("Pick at least one day".into());
            } else {
                let enabled = sched.enabled;
                self.sched.open = false;
                self.save_settings(serde_json::json!({ "schedule": sched }), move |app, r| match r {
                    Ok(()) => app.toast(if enabled { "Schedule saved" } else { "Schedule turned off" }),
                    Err(e) => {
                        // Back to the dialog, as edited, to put it right.
                        app.sched.open = true;
                        app.sched.error = Some(e);
                    }
                });
            }
        }
        if let Some(start) = queue {
            self.queue_now(start);
        }
        if !open {
            self.sched.open = false;
        }
    }

    fn limiter_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.limiter.open {
            return;
        }
        let mut open = true;
        let mut save = false;
        let l = &mut self.limiter;
        window("Speed limiter").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut l.on, "Limit the total download speed to");
                ui.add_enabled(l.on, egui::DragValue::new(&mut l.kb).range(1..=10_000_000).suffix(" KB/s"));
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                label(ui, p, "Presets");
                for (txt, kb) in [("256 KB/s", 256), ("512 KB/s", 512), ("1 MB/s", 1024), ("5 MB/s", 5120), ("10 MB/s", 10240)] {
                    if ui.small_button(txt).clicked() {
                        l.kb = kb;
                        l.on = true;
                    }
                }
            });
            label(ui, p, "Shared by every download and connection, and applied immediately. A single download can have its own lower limit in its Properties.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Save")).clicked() {
                    save = true;
                }
                if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Cancel")).clicked() {
                    l.open = false;
                }
            });
        });
        if save {
            let (on, kb) = (self.limiter.on, self.limiter.kb);
            self.limiter.open = false;
            self.save_settings(serde_json::json!({ "limit_enabled": on, "speed_limit": kb as u64 * 1024 }), move |app, r| match r {
                Ok(()) => app.toast(if on { format!("Total speed limited to {kb} KB/s") } else { "Speed limiter off".into() }),
                Err(e) => {
                    app.limiter.open = true;
                    app.toast(format!("Could not save: {e}"));
                }
            });
        }
        if !open {
            self.limiter.open = false;
        }
    }

    /// How to connect a browser, with what is known about the connection right now.
    fn integration_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.show_integration {
            return;
        }
        let mut open = true;
        let svc = self.snap.service.clone();
        let mut copy: Option<String> = None;
        egui::Window::new("Browser integration")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(560.0);
                match svc.extension.clone().filter(|e| e.seconds_ago < 180) {
                    Some(e) => {
                        ui.label(RichText::new(format!("Connected — reIDM extension {} in {}", e.version, if e.browser.is_empty() { "your browser" } else { &e.browser })).color(p.ok).strong());
                        label(ui, p, "Downloads you start in that browser are taken over by reIDM. Hold Alt while clicking a link to let the browser have it.");
                    }
                    None => {
                        ui.label(RichText::new("The reIDM extension has not connected yet").color(p.warn).strong());
                        label(ui, p, "Install it once in each browser you use (Chrome, Edge, Brave, Vivaldi, Opera, Chromium):");
                    }
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Install").strong());
                for (n, step) in [
                    "Open the browser's extensions page: chrome://extensions, edge://extensions or brave://extensions.",
                    "Turn on Developer mode (top right, or bottom left in Edge).",
                    "Choose Load unpacked and select the folder below.",
                ]
                .iter()
                .enumerate()
                {
                    ui.label(format!("{}. {step}", n + 1));
                }
                ui.add_space(6.0);
                match &svc.extension_dir {
                    Some(dir) => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(RichText::new(dir).monospace().color(p.text)).truncate());
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Open the folder").clicked() {
                                open_external(dir);
                            }
                            if ui.button("Copy the path").clicked() {
                                copy = Some(dir.clone());
                            }
                        });
                        label(ui, p, "reIDM keeps this folder up to date, so the extension updates itself the next time the browser starts.");
                    }
                    None => {
                        ui.label(RichText::new("This installation does not include the extension folder. Run `reidm extension` from a terminal, or download reidm-extension.zip from the release page.").color(p.warn));
                    }
                }
                ui.add_space(8.0);
                ui.label(RichText::new("What it does").strong());
                label(ui, p, "• Takes over every browser download before the browser saves anything — the browser shows nothing and keeps nothing.\n\
                              • Checks reIDM can fetch the file (with your cookies) first; if not, the browser keeps it.\n\
                              • Starts reIDM if it is not running.\n\
                              • Finds video and audio, offers each quality, and grabs every resource on a page.");
                if svc.platform == "windows" && !svc.idm_port {
                    ui.add_space(8.0);
                    label(ui, p, "Note: an installed IDM holds port 1001, which only matters for Tonec's own extension, not reIDM's.");
                }
            });
        if let Some(c) = copy {
            ctx.copy_text(c);
            self.toast("Path copied");
        }
        if !open {
            self.show_integration = false;
        }
    }

    /// IDM's "Download complete" window, one finished download at a time.
    fn complete_window(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(&id) = self.completed.first() else { return };
        let Some(it) = self.item(id).cloned() else {
            self.completed.remove(0);
            return;
        };
        let mut close = false;
        let mut action: Option<&'static str> = None;
        let mut never = false;
        egui::Window::new("Download complete")
            .id(egui::Id::new(("complete", id)))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -40.0))
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::hover());
                    icons::file_card(ui.painter(), r, it.category(), self.prefs.dark);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(it.file_name()).strong());
                        ui.label(RichText::new(format!("{} · {}", it.size.map(size_human).unwrap_or_default(), it.path.parent().map(|d| d.display().to_string()).unwrap_or_default())).color(p.text_dim).size(11.5));
                    });
                });
                if self.completed.len() > 1 {
                    ui.label(RichText::new(format!("and {} more finished", self.completed.len() - 1)).color(p.text_dim).size(11.0));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Open").strong()).clicked() {
                        action = Some("open");
                    }
                    if ui.button("Open folder").clicked() {
                        action = Some("folder");
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Don't show again").clicked() {
                            never = true;
                        }
                    });
                });
            });
        if let Some(a) = action {
            self.action(id, a);
            close = true;
        }
        if never {
            self.prefs.notify_complete = false;
            self.prefs.save();
            self.completed.clear();
            self.toast("Turn it back on in Options › Window");
        } else if close {
            self.completed.retain(|x| *x != id);
        }
    }
}

// ── toolbar button ───────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum Icon {
    Plus,
    Play,
    Pause,
    PauseAll,
    Cross,
    CrossAll,
    Gear,
    Clock,
    QueueStart,
    QueueStop,
    Grabber,
    Gauge,
}

/// An IDM-style toolbar button: a large icon with a small caption underneath. Two-line
/// captions (e.g. "Delete\nCompleted") are supported, which is why the height is fixed
/// rather than derived from the text.
fn tool(ui: &mut egui::Ui, p: &Palette, label: &str, icon: Icon, primary: bool) -> egui::Response {
    let lines: Vec<&str> = label.split('\n').collect();
    let widest = lines
        .iter()
        .map(|l| ui.painter().layout_no_wrap((*l).to_string(), egui::FontId::proportional(10.5), p.text).size().x)
        .fold(0.0f32, f32::max);
    let w = (widest + 16.0).max(56.0);
    let h = 46.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), egui::Sense::click());
    let fg = if primary { p.accent_text } else { p.text };

    if primary {
        let bg = if resp.hovered() { p.accent.gamma_multiply(0.85) } else { p.accent };
        ui.painter().rect_filled(rect, 5.0, bg);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, p.accent.gamma_multiply(0.16));
        ui.painter().rect_stroke(rect, 5.0, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    }

    let icon_sz = 20.0;
    let icon_rect = egui::Rect::from_center_size(egui::Pos2::new(rect.center().x, rect.top() + 6.0 + icon_sz / 2.0), Vec2::splat(icon_sz));
    let painter = ui.painter();
    match icon {
        Icon::Plus => icons::plus(painter, icon_rect, fg),
        Icon::Play => icons::play(painter, icon_rect, fg),
        Icon::Pause => icons::pause(painter, icon_rect, fg),
        Icon::PauseAll => icons::pause_all(painter, icon_rect, fg),
        Icon::Cross => icons::cross(painter, icon_rect, fg),
        Icon::CrossAll => icons::cross_all(painter, icon_rect, fg),
        Icon::Gear => icons::gear(painter, icon_rect, fg),
        Icon::Clock => icons::clock(painter, icon_rect, fg),
        Icon::QueueStart => icons::queue_start(painter, icon_rect, fg),
        Icon::QueueStop => icons::queue_stop(painter, icon_rect, fg),
        Icon::Grabber => icons::grabber(painter, icon_rect, fg),
        Icon::Gauge => icons::gauge(painter, icon_rect, fg),
    }
    for (i, line) in lines.iter().enumerate() {
        painter.text(egui::Pos2::new(rect.center().x, rect.top() + 29.0 + i as f32 * 11.0), egui::Align2::CENTER_TOP, *line, egui::FontId::proportional(10.5), fg);
    }
    resp.on_hover_text(label.replace('\n', " "))
}
