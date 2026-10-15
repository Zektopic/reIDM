//! The reIDM desktop window.
//!
//! Laid out like the classic IDM main window: a menu bar, a toolbar of icon-over-label
//! buttons, a category tree down the left, the download table in the middle, and a status
//! bar. Every IDM feature the window shows is implemented; anything reIDM does not have is left
//! out rather than shown as a dead button.

use crate::api::{open_external, Api, BatchResult, Snapshot};
use crate::icons;
use crate::model::{eta_human, now_secs, rate_human, since_human, size_human, Category, Found, GrabOptions, Item, Schedule, Status, View, CATEGORIES};
use crate::prefs::Prefs;
use crate::theme::{self, Palette};
use egui::{Align, Layout, RichText, Vec2};
use egui_extras::{Column, TableBuilder};
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

#[derive(PartialEq, Clone, Copy)]
enum SortKey {
    Name,
    Size,
    Status,
    Rate,
    LastTry,
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
}

/// IDM's *Add batch download*: an address with one `*` and a range.
struct BatchState {
    open: bool,
    pattern: String,
    from: String,
    to: String,
    width: usize,
}

impl Default for BatchState {
    fn default() -> Self {
        Self { open: false, pattern: String::new(), from: "1".into(), to: "10".into(), width: 0 }
    }
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
}

pub struct App {
    api: Api,
    snap: Snapshot,
    view: View,
    selected: Vec<u64>,
    anchor: Option<u64>,
    search: String,
    sort: SortKey,
    desc: bool,
    prefs: Prefs,
    applied_theme: Option<bool>,
    toast: Option<Toast>,
    last_error_shown: Option<String>,

    // dialogs
    add: AddState,
    batch: BatchState,
    show_options: bool,
    opt_dir: String,
    opt_conn: usize,
    opt_max: usize,
    confirm_remove: Vec<u64>,
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
        Self {
            api: Api::start(),
            snap: Snapshot::default(),
            view: View::Category(Category::All),
            selected: Vec::new(),
            anchor: None,
            search: String::new(),
            sort: SortKey::Name,
            desc: false,
            prefs,
            applied_theme: None,
            toast: None,
            last_error_shown: None,
            add: AddState::default(),
            batch: BatchState::default(),
            show_options: false,
            opt_dir: String::new(),
            opt_conn: 8,
            opt_max: 4,
            confirm_remove: Vec::new(),
            about: false,
            refresh: None,
            sched: SchedEdit::default(),
            limiter: LimiterEdit::default(),
            props: None,
            show_integration: false,
            completed: Vec::new(),
            last_status: HashMap::new(),
            primed: false,
            update: None,
            update_rx: None,
            cats_open: true,
            downloads_open: true,
            checked_on_start: false,
            show_grabber: false,
            grab: GrabState::default(),
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), until: Instant::now() + Duration::from_secs(5) });
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
        if !self.snap.connected {
            return;
        }
        for it in &self.snap.items {
            let before = self.last_status.insert(it.id, it.status.clone());
            let finished_now = it.status == Status::Done && before.is_some_and(|b| b != Status::Done);
            if self.primed && finished_now && self.prefs.notify_complete && !self.completed.contains(&it.id) {
                self.completed.push(it.id);
            }
        }
        let ids: Vec<u64> = self.snap.items.iter().map(|i| i.id).collect();
        self.last_status.retain(|id, _| ids.contains(id));
        self.completed.retain(|id| ids.contains(id));
        self.primed = true;
    }

    fn visible(&self) -> Vec<&Item> {
        let q = self.search.trim().to_ascii_lowercase();
        let mut v: Vec<&Item> = self
            .snap
            .items
            .iter()
            .filter(|i| self.view.matches(i))
            .filter(|i| q.is_empty() || i.file_name().to_ascii_lowercase().contains(&q) || i.request.url.to_ascii_lowercase().contains(&q))
            .collect();
        let rate = |i: &Item| self.snap.rates.get(&i.id).copied().unwrap_or(0.0);
        v.sort_by(|a, b| {
            let o = match self.sort {
                SortKey::Name => a.file_name().to_ascii_lowercase().cmp(&b.file_name().to_ascii_lowercase()),
                SortKey::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
                SortKey::Status => a.status.label().cmp(b.status.label()),
                SortKey::Rate => rate(a).partial_cmp(&rate(b)).unwrap_or(std::cmp::Ordering::Equal),
                SortKey::LastTry => a.last_try.unwrap_or(0).cmp(&b.last_try.unwrap_or(0)),
            };
            if self.desc {
                o.reverse()
            } else {
                o
            }
        });
        v
    }

    fn count(&self, v: View) -> usize {
        self.snap.items.iter().filter(|i| v.matches(i)).count()
    }

    fn item(&self, id: u64) -> Option<&Item> {
        self.snap.items.iter().find(|i| i.id == id)
    }

    // ── selection ────────────────────────────────────────────────────────

    fn select_only(&mut self, id: u64) {
        self.selected = vec![id];
        self.anchor = Some(id);
    }

    fn toggle_select(&mut self, id: u64) {
        if let Some(p) = self.selected.iter().position(|x| *x == id) {
            self.selected.remove(p);
        } else {
            self.selected.push(id);
            self.anchor = Some(id);
        }
    }

    fn select_range(&mut self, id: u64, ordered: &[u64]) {
        let Some(a) = self.anchor else { return self.select_only(id) };
        let (Some(i), Some(j)) = (ordered.iter().position(|x| *x == a), ordered.iter().position(|x| *x == id)) else {
            return self.select_only(id);
        };
        let (lo, hi) = if i <= j { (i, j) } else { (j, i) };
        self.selected = ordered[lo..=hi].to_vec();
    }

    fn selected_items(&self) -> Vec<&Item> {
        self.snap.items.iter().filter(|i| self.selected.contains(&i.id)).collect()
    }

    // ── commands ─────────────────────────────────────────────────────────

    fn start_selected(&mut self) {
        let ids: Vec<u64> = self.selected_items().iter().filter(|i| i.status.is_stopped()).map(|i| i.id).collect();
        if ids.is_empty() {
            return self.toast("Select a paused or failed download to start");
        }
        for id in ids {
            self.api.action(id, "resume");
        }
    }

    fn stop_selected(&mut self) {
        let ids: Vec<u64> = self.selected_items().iter().filter(|i| i.status.is_active()).map(|i| i.id).collect();
        if ids.is_empty() {
            return self.toast("Select a running download to stop");
        }
        for id in ids {
            self.api.action(id, "pause");
        }
    }

    fn stop_all(&mut self) {
        self.api.stop_all();
    }

    fn restart_selected(&mut self) {
        let ids: Vec<u64> = self.selected.clone();
        if ids.is_empty() {
            return self.toast("Select a download to restart");
        }
        for id in ids {
            if let Err(e) = self.api.try_action(id, "restart") {
                return self.toast(format!("could not restart: {e}"));
            }
        }
        self.toast("Restarted from the beginning");
    }

    fn set_queued(&mut self, queued: bool) {
        let ids = self.selected.clone();
        if ids.is_empty() {
            return self.toast("Select a download first");
        }
        // The service refuses to queue a finished download, so count what it accepted.
        let n = ids.iter().filter(|id| self.api.try_action(**id, if queued { "schedule" } else { "unschedule" }).is_ok()).count();
        self.toast(match (queued, n) {
            (true, 0) => "Finished downloads can't be added to the scheduled queue".to_string(),
            (true, n) => format!("{n} added to the scheduled queue"),
            (false, n) => format!("{n} removed from the scheduled queue"),
        });
    }

    fn queue_now(&mut self, start: bool) {
        match self.api.queue(start) {
            Ok(0) if start => self.toast("Nothing in the scheduled queue is waiting to start"),
            Ok(0) => self.toast("Nothing in the scheduled queue is running"),
            Ok(n) => self.toast(if start { format!("Started the queue: {n} download(s)") } else { format!("Stopped the queue: {n} download(s)") }),
            Err(e) => self.toast(format!("could not {} the queue: {e}", if start { "start" } else { "stop" })),
        }
    }

    fn delete_completed(&mut self) {
        let done: Vec<u64> = self.snap.items.iter().filter(|i| i.status == Status::Done).map(|i| i.id).collect();
        if done.is_empty() {
            return self.toast("There are no completed downloads");
        }
        for id in done.iter() {
            self.api.remove(*id, false);
        }
        self.selected.clear();
        self.toast(format!("Removed {} completed download(s)", done.len()));
    }

    fn remove_ids(&mut self, ids: &[u64], purge: bool) {
        for id in ids {
            self.api.remove(*id, purge);
        }
        self.selected.retain(|x| !ids.contains(x));
        let n = ids.len();
        self.toast(match (purge, n) {
            (true, 1) => "Download and file removed".to_string(),
            (true, n) => format!("{n} downloads and their files removed"),
            (false, 1) => "Download removed from the list".to_string(),
            (false, n) => format!("{n} downloads removed from the list"),
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
        let r = self.api.add_batch(&urls, later);
        self.report_batch(r, later);
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
                    let r = self.api.add_entries(&entries, false);
                    self.report_batch(r, false);
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
        match self.api.export(format).map_err(|e| e.to_string()).and_then(|text| std::fs::write(&path, text).map_err(|e| e.to_string())) {
            Ok(()) => self.toast(format!("Exported {} download(s) to {}", self.snap.items.len(), path.display())),
            Err(e) => self.toast(format!("could not export: {e}")),
        }
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
        self.grab.rx = Some(self.api.grab_async(&url, &opts));
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
        let Some(id) = self.selected.last().copied() else {
            return self.toast("Select a download first");
        };
        self.open_id(id, reveal);
    }

    fn open_id(&mut self, id: u64, reveal: bool) {
        let Some(it) = self.item(id) else { return };
        if !matches!(it.status, Status::Done) && !reveal {
            return self.toast("That download has not finished yet");
        }
        self.api.action(id, if reveal { "folder" } else { "open" });
    }

    fn open_props(&mut self, id: u64) {
        let limit = self.item(id).and_then(|i| i.speed_limit);
        self.props = Some(PropsState { id, limit_on: limit.is_some(), limit_kb: limit.map(|l| (l / 1024) as u32).unwrap_or(512).max(1) });
    }

    fn open_refresh(&mut self, id: u64) {
        if self.item(id).is_some_and(|i| i.request.media.is_some()) {
            self.toast("A video download finds its own address each time it starts: there is nothing to refresh. Start it again.");
            return;
        }
        let old_url = self.item(id).map(|i| i.request.url.clone()).unwrap_or_default();
        self.refresh = Some(RefreshState { id, url: String::new(), busy: None, error: None, capturing: false, old_url });
    }

    fn open_scheduler(&mut self) {
        let s = self.api.fetch_settings().map(|s| s.schedule).unwrap_or_default();
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
        let s = self.api.fetch_settings().unwrap_or_default();
        self.limiter = LimiterEdit { open: true, on: s.limit_enabled, kb: if s.speed_limit > 0 { (s.speed_limit / 1024) as u32 } else { 1024 } };
    }

    fn open_options(&mut self) {
        if let Some(s) = self.api.fetch_settings() {
            self.opt_dir = s.default_dir;
            self.opt_conn = s.connections.max(1);
            self.opt_max = s.max_concurrent.max(1);
        }
        self.show_options = true;
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
            self.confirm_remove = self.selected.clone();
        }
        if all {
            self.selected = self.visible().iter().map(|i| i.id).collect();
        }
        if enter {
            if let Some(id) = self.selected.last().copied() {
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
        if let Some(s) = self.api.try_recv() {
            self.snap = s;
            self.note_completions();
            if self.snap.last_error != self.last_error_shown {
                self.last_error_shown = self.snap.last_error.clone();
                if let Some(e) = self.snap.last_error.clone().filter(|_| self.snap.connected) {
                    self.toast(e);
                }
            }
        }
        self.poll_update();
        self.poll_grab();
        self.poll_refresh();
        // One check shortly after launch, so a new release is noticed without the user going
        // looking. It is a single GET to api.github.com and reports nothing back to us.
        if !self.checked_on_start && self.snap.connected {
            self.checked_on_start = true;
            self.check_for_updates();
        }

        let ctx = ui.ctx().clone();
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
        self.about_dialog(&ctx);
        self.confirm_dialog(&ctx);
        self.grabber_window(&ctx, &p);
        self.props_window(&ctx, &p);
        self.refresh_window(&ctx, &p);
        self.scheduler_window(&ctx, &p);
        self.limiter_window(&ctx, &p);
        self.integration_window(&ctx, &p);
        self.complete_window(&ctx, &p);

        if let Some(t) = &self.toast {
            if Instant::now() < t.until {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -48.0))
                    .order(egui::Order::Tooltip)
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_max_width(640.0);
                            ui.label(RichText::new(&t.text).color(p.text));
                        });
                    });
            } else {
                self.toast = None;
            }
        }

        ctx.request_repaint_after(Duration::from_millis(200));
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
            self.api.open_url(&url);
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
                            if let Some(id) = self.selected.last().copied() {
                                self.open_props(id);
                            }
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Delete").clicked() {
                            self.confirm_remove = self.selected.clone();
                            ui.close();
                        }
                        if ui.button("Delete completed").clicked() {
                            self.delete_completed();
                            ui.close();
                        }
                    });
                    ui.menu_button("Downloads", |ui| {
                        if ui.button("Start").clicked() {
                            self.start_selected();
                            ui.close();
                        }
                        if ui.button("Stop").clicked() {
                            self.stop_selected();
                            ui.close();
                        }
                        if ui.button("Start all").clicked() {
                            for i in self.snap.items.clone() {
                                if i.status.is_stopped() {
                                    self.api.action(i.id, "resume");
                                }
                            }
                            ui.close();
                        }
                        if ui.button("Stop all").clicked() {
                            self.stop_all();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Restart").clicked() {
                            self.restart_selected();
                            ui.close();
                        }
                        if ui.button("Refresh download address…").clicked() {
                            match self.selected.last().copied() {
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
                            let _ = self.api.update_settings(serde_json::json!({ "limit_enabled": !on }));
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
                        let (txt, col) = if self.snap.connected { ("Service running", p.ok) } else { ("Service unavailable", p.err) };
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
                    if tool(ui, p, "Stop", Icon::Pause, false).clicked() {
                        self.stop_selected();
                    }
                    if tool(ui, p, "Stop All", Icon::PauseAll, false).clicked() {
                        self.stop_all();
                    }
                    sep(ui);
                    if tool(ui, p, "Delete", Icon::Cross, false).clicked() {
                        self.confirm_remove = self.selected.clone();
                        if self.confirm_remove.is_empty() {
                            self.toast("Select the downloads to delete");
                        }
                    }
                    if tool(ui, p, "Delete\nCompleted", Icon::CrossAll, false).clicked() {
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
                    ui.painter().text(r.right_center() - Vec2::new(8.0, 0.0), egui::Align2::RIGHT_CENTER, self.snap.items.len().to_string(), egui::FontId::proportional(11.5), p.text_dim);
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
                            if tree_row(ui, p, 34.0, c.label(), Some(self.count(v)), self.view == v, |pt, r| icons::file_card(pt, r, c, dark)) {
                                self.view = v;
                            }
                        }
                    }
                    for (v, done) in [(View::Unfinished, false), (View::Finished, true)] {
                        let col = if done { p.ok } else { p.accent };
                        if tree_row(ui, p, 16.0, v.label(), Some(self.count(v)), self.view == v, |pt, r| {
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
                if tree_row(ui, p, 4.0, q.label(), Some(self.count(q)), self.view == q, |pt, r| icons::clock(pt, r, qcol)) {
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
                    let active = self.snap.items.iter().filter(|i| i.status == Status::Running).count();
                    let done = self.snap.items.iter().filter(|i| i.status == Status::Done).count();
                    let failed = self.snap.items.iter().filter(|i| matches!(i.status, Status::Failed(_))).count();
                    let queued = self.snap.items.iter().filter(|i| i.scheduled && i.status != Status::Done).count();

                    let mut parts = vec![format!("{} total", self.snap.items.len())];
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
                let rows = self.visible();
                let ordered: Vec<u64> = rows.iter().map(|i| i.id).collect();
                let mut click: Option<(u64, bool, bool)> = None; // id, ctrl, shift
                let mut check_click: Option<u64> = None;
                let mut open_req: Option<u64> = None;
                let mut ctx_menu: Option<u64> = None;
                let mut ctx_cmd: Option<CtxCmd> = None;
                let mut sort_click: Option<SortKey> = None;
                let mut toggle_all = false;
                let now = now_secs();

                if rows.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
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
                    return;
                }

                // Header checkbox state: all / none / some of the visible rows.
                let visible_ids: Vec<u64> = ordered.clone();
                let selected_visible = visible_ids.iter().filter(|id| self.selected.contains(id)).count();
                let header_check = if selected_visible == 0 {
                    Some(false)
                } else if selected_visible == visible_ids.len() {
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
                        h(&mut header, "Time Left", SortKey::Rate);
                        h(&mut header, "Transfer Rate", SortKey::Rate);
                        h(&mut header, "Last Try", SortKey::LastTry);
                        h(&mut header, "Description", SortKey::Name);
                    })
                    .body(|body| {
                        body.rows(42.0, rows.len(), |mut row| {
                            let idx = row.index();
                            let it = rows[idx];
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
                                let n = it.connections();
                                let label = if it.status == Status::Running && n > 1 { format!("{} ({n})", it.status.label()) } else { it.status.label().to_string() };
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
                                let txt = if it.status == Status::Running { rate.map(rate_human).unwrap_or_default() } else { String::new() };
                                ui.label(RichText::new(txt).size(12.0).color(if it.status == Status::Running { p.accent } else { p.text_dim }));
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
                    if header_check == Some(true) {
                        self.selected.retain(|id| !visible_ids.contains(id));
                    } else {
                        for id in &visible_ids {
                            if !self.selected.contains(id) {
                                self.selected.push(*id);
                            }
                        }
                    }
                }
                if let Some(id) = check_click {
                    self.toggle_select(id);
                }
                if let Some((id, ctrl, shift)) = click {
                    if shift {
                        self.select_range(id, &ordered);
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
                                        pick(ui, "Stop", CtxCmd::Stop);
                                    }
                                    if it.status.is_stopped() {
                                        pick(ui, "Resume", CtxCmd::Start);
                                    }
                                    pick(ui, "Restart from the beginning", CtxCmd::Restart);
                                    if it.status != Status::Done {
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
                                    pick(ui, "Remove and delete the file", CtxCmd::RemoveFile);
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
                        if let Some(id) = self.selected.last().copied() {
                            self.open_refresh(id);
                        }
                    }
                    Some(CtxCmd::Queue) => self.set_queued(true),
                    Some(CtxCmd::Unqueue) => self.set_queued(false),
                    Some(CtxCmd::Props) => {
                        if let Some(id) = self.selected.last().copied() {
                            self.open_props(id);
                        }
                    }
                    Some(CtxCmd::CopyUrl(u)) => {
                        ctx.copy_text(u);
                        self.toast("Address copied");
                    }
                    Some(CtxCmd::Remove) => {
                        let ids = self.selected.clone();
                        self.remove_ids(&ids, false);
                    }
                    Some(CtxCmd::RemoveFile) => {
                        let ids = self.selected.clone();
                        self.remove_ids(&ids, true);
                    }
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
fn segment_map(ui: &mut egui::Ui, p: &Palette, it: &Item) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, p.border.gamma_multiply(0.5));
    let total = it.size.unwrap_or(0);
    if it.status == Status::Done {
        painter.rect_filled(rect, 3.0, p.ok);
        return;
    }
    if total == 0 || it.segments.is_empty() {
        if let Some(f) = it.fraction() {
            painter.rect_filled(egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * f, rect.height())), 3.0, p.accent);
        }
        return;
    }
    let x = |b: u64| rect.left() + rect.width() * (b.min(total) as f32 / total as f32);
    for s in &it.segments {
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
            let count = reidm_core::grabber::parse_url_list(&self.add.urls).len();
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
                match self.api.add(&urls[0], name, dir, later) {
                    Ok(()) => {
                        self.toast(if later { "Added to the scheduled queue" } else { "Download started" });
                        self.add = AddState::default();
                    }
                    Err(e) => self.toast(format!("could not add it: {e}")),
                }
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
        use reidm_core::lists::{expand_pattern, Sequence};
        let seq = match (self.batch.from.trim().parse::<u64>(), self.batch.to.trim().parse::<u64>()) {
            (Ok(from), Ok(to)) => Ok(Sequence::Numbers { from, to, width: self.batch.width }),
            _ => {
                let one = |s: &str| {
                    let mut c = s.trim().chars();
                    match (c.next(), c.next()) {
                        (Some(ch), None) if ch.is_ascii_alphabetic() => Some(ch),
                        _ => None,
                    }
                };
                match (one(&self.batch.from), one(&self.batch.to)) {
                    (Some(from), Some(to)) => Ok(Sequence::Letters { from, to }),
                    _ => Err("From and To must both be numbers, or both be single letters".to_string()),
                }
            }
        };
        let preview = seq.and_then(|s| expand_pattern(&self.batch.pattern, &s).map_err(|e| e.to_string()));
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
                label(ui, p, "wildcard size");
                ui.add(egui::DragValue::new(&mut self.batch.width).range(0..=8));
            });
            label(ui, p, "Numbers (1 to 50) or letters (a to z). Wildcard size pads numbers with zeros: 3 gives 001, 002 …");
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
            ui.set_min_width(500.0);
            ui.label(RichText::new("Downloads").strong());
            ui.add_space(6.0);
            label(ui, p, "Default download folder");
            ui.horizontal(|ui| {
                ui.add_sized(Vec2::new(380.0, 26.0), egui::TextEdit::singleline(&mut self.opt_dir));
                if ui.button("Browse…").clicked() {
                    if let Some(d) = rfd::FileDialog::new().set_directory(&self.opt_dir).pick_folder() {
                        self.opt_dir = d.to_string_lossy().into_owned();
                    }
                }
            });
            ui.add_space(8.0);
            label(ui, p, "Connections per download");
            ui.add(egui::Slider::new(&mut self.opt_conn, 1..=32));
            label(ui, p, "When a connection finishes early it takes over half of the slowest one's remaining part, so more connections rarely hurt. 8 is a good default.");
            ui.add_space(8.0);
            label(ui, p, "Downloads at the same time");
            ui.add(egui::Slider::new(&mut self.opt_max, 1..=16));
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
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Save")).clicked() {
                    save = true;
                }
                if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Cancel")).clicked() {
                    self.show_options = false;
                }
            });
        });
        if save {
            let r = self.api.update_settings(serde_json::json!({
                "default_dir": self.opt_dir,
                "connections": self.opt_conn,
                "max_concurrent": self.opt_max,
            }));
            self.prefs.save();
            match r {
                Ok(()) => {
                    self.show_options = false;
                    self.toast("Options saved");
                }
                Err(e) => self.toast(format!("could not save: {e}")),
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
                    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                        for (i, f) in found.iter().enumerate() {
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
                    let chosen: Vec<String> = found.iter().zip(selected.iter()).filter(|(_, s)| **s).map(|(f, _)| f.url.clone()).collect();
                    if ui.add_enabled(!chosen.is_empty(), egui::Button::new(format!("Download {} selected", chosen.len()))).clicked() {
                        queue = Some((chosen.clone(), false));
                    }
                    if ui.add_enabled(!chosen.is_empty(), egui::Button::new("Download later")).clicked() {
                        queue = Some((chosen, true));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            self.show_grabber = false;
                        }
                    });
                });
            });

        let all_urls: Vec<String> = found.iter().map(|f| f.url.clone()).collect();
        self.grab.found = found;
        self.grab.selected = selected;
        if scan {
            self.start_grab();
        }
        if let Some((urls, later)) = queue {
            self.queue_urls(urls, later);
        }
        if export {
            self.export_found(&all_urls);
        }
        if import {
            self.import_from_file();
        }
        if !open {
            self.show_grabber = false;
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        if self.confirm_remove.is_empty() {
            return;
        }
        let ids = self.confirm_remove.clone();
        let what = match ids.as_slice() {
            [one] => format!("“{}”", self.item(*one).map(|i| i.file_name()).unwrap_or_default()),
            many => format!("{} downloads", many.len()),
        };
        let mut open = true;
        window("Delete").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(390.0);
            ui.label(format!("Remove {what} from the list?"));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Remove from list only").clicked() {
                    self.remove_ids(&ids, false);
                    self.confirm_remove.clear();
                }
                if ui.button("Remove and delete the files").clicked() {
                    self.remove_ids(&ids, true);
                    self.confirm_remove.clear();
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_remove.clear();
                }
            });
        });
        if !open {
            self.confirm_remove.clear();
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
                    if let Some(ua) = &it.request.user_agent {
                        row(ui, "Sent as", ua.clone());
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
                    if it.request.media.is_some() {
                        row(ui, "Resume capability", "Yes — yt-dlp carries on where it stopped".into());
                    } else {
                        row(ui, "Resume capability", if it.resumable { "Yes".into() } else if it.size.is_some() { "No — a stop means starting over".into() } else { "Unknown".into() });
                        row(ui, "Connections", format!("{} working on {} part(s)", it.connections(), it.segments.len().max(1)));
                    }
                });
                ui.add_space(8.0);
                segment_map(ui, p, &it);
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
                    if ui.button("Restart").clicked() {
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
            match self.api.set_limit(id, l) {
                Ok(()) => self.toast(match l {
                    Some(b) => format!("This download is limited to {}", rate_human(b as f64)),
                    None => "This download's own limit is off".into(),
                }),
                Err(e) => self.toast(format!("could not set the limit: {e}")),
            }
        }
        if let Some(c) = cmd {
            if c == "restart" {
                if let Err(e) = self.api.try_action(id, c) {
                    self.toast(format!("could not restart: {e}"));
                }
            } else {
                self.api.action(id, c);
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
        let name = self.snap.items.iter().find(|i| i.id == id).map(|i| i.file_name()).unwrap_or_default();
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
            r.busy = Some(self.api.refresh_async(id, r.url.trim()));
        }
        if capture {
            r.error = None;
            match self.api.try_action(id, "refresh-capture") {
                Ok(()) => r.capturing = true,
                Err(e) => r.error = Some(e),
            }
        }
        if cancel_capture {
            r.capturing = false;
            self.api.cancel_refresh();
        }
        if !open {
            if self.refresh.as_ref().is_some_and(|r| r.capturing) {
                self.api.cancel_refresh();
            }
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
        let in_queue = self.snap.items.iter().filter(|i| i.scheduled && i.status != Status::Done).count();
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
                match self.api.update_settings(serde_json::json!({ "schedule": sched })) {
                    Ok(()) => {
                        self.sched.open = false;
                        self.toast(if sched.enabled { "Schedule saved" } else { "Schedule turned off" });
                    }
                    Err(e) => self.sched.error = Some(e),
                }
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
            let r = self.api.update_settings(serde_json::json!({ "limit_enabled": self.limiter.on, "speed_limit": self.limiter.kb as u64 * 1024 }));
            match r {
                Ok(()) => {
                    self.limiter.open = false;
                    self.toast(if self.limiter.on { format!("Total speed limited to {} KB/s", self.limiter.kb) } else { "Speed limiter off".into() });
                }
                Err(e) => self.toast(format!("could not save: {e}")),
            }
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
            self.api.action(id, a);
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
