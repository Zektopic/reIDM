//! The reIDM desktop window.
//!
//! Laid out like the classic IDM main window: a menu bar, a toolbar of icon-over-label
//! buttons, a category tree down the left, the download table in the middle, and a status
//! bar. Features reIDM does not have (queues, the site grabber) are shown greyed with a
//! tooltip rather than quietly omitted, so the window makes no promises it cannot keep.

use crate::api::{Api, Snapshot};
use crate::icons;
use crate::model::{eta_human, now_secs, rate_human, since_human, size_human, Category, Found, GrabOptions, Item, Status, CATEGORIES};
use crate::theme::{self, Palette};
use egui::{Align, Layout, RichText, Vec2};
use egui_extras::{Column, TableBuilder};
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

pub struct App {
    api: Api,
    snap: Snapshot,
    cat: Category,
    selected: Vec<u64>,
    anchor: Option<u64>,
    search: String,
    sort: SortKey,
    desc: bool,
    dark: bool,
    applied_theme: Option<bool>,
    toast: Option<Toast>,

    // dialogs
    show_add: bool,
    add_url: String,
    add_name: String,
    show_options: bool,
    opt_dir: String,
    opt_conn: usize,
    confirm_remove: Option<u64>,
    about: bool,

    // updates
    update: Option<UpdateOffer>,
    update_rx: Option<std::sync::mpsc::Receiver<Result<Option<reidm_core::update::Release>, String>>>,
    /// IDM's "Categories > Downloads" node is collapsible; start open.
    cats_open: bool,
    downloads_open: bool,
    /// An update check is kicked off once, a moment after startup.
    checked_on_start: bool,

    // site grabber
    show_grabber: bool,
    grab: GrabState,
}

/// The site-grabber window's state. Every field defaults to empty/off, so the derive is
/// exactly what the hand-written `Default` said.
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
    rx: Option<std::sync::mpsc::Receiver<Result<Vec<Found>, String>>>,
}


impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, dark: bool) -> Self {
        theme::apply(&cc.egui_ctx, dark);
        Self {
            api: Api::start(),
            snap: Snapshot::default(),
            cat: Category::All,
            selected: Vec::new(),
            anchor: None,
            search: String::new(),
            sort: SortKey::Name,
            desc: false,
            dark,
            applied_theme: None,
            toast: None,
            show_add: false,
            add_url: String::new(),
            add_name: String::new(),
            show_options: false,
            opt_dir: String::new(),
            opt_conn: 8,
            confirm_remove: None,
            about: false,
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
        self.toast = Some(Toast { text: text.into(), until: Instant::now() + Duration::from_secs(4) });
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
                let asset = rel
                    .asset_for(std::env::consts::OS)
                    .map(|a| (a.name.clone(), a.browser_download_url.clone()));
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

    fn visible(&self) -> Vec<&Item> {
        let q = self.search.trim().to_ascii_lowercase();
        let mut v: Vec<&Item> = self
            .snap
            .items
            .iter()
            .filter(|i| self.cat.matches(i))
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

    fn count(&self, c: Category) -> usize {
        self.snap.items.iter().filter(|i| c.matches(i)).count()
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

    fn remove_id(&mut self, id: u64, purge: bool) {
        self.api.remove(id, purge);
        self.selected.retain(|x| *x != id);
        self.toast(if purge { "Download and file removed" } else { "Download removed from the list" });
    }

    /// Queue every URL in a line- or file-shaped list, and say what happened.
    fn queue_urls(&mut self, urls: Vec<String>) {
        if urls.is_empty() {
            return self.toast("No URLs to add");
        }
        match self.api.add_batch(&urls) {
            Ok(results) => {
                let count = |k: &str| results.iter().filter(|r| r.outcome.as_deref() == Some(k)).count();
                let new = count("new");
                let resumed = count("existing");
                let have = count("already_complete");
                let failed = results.iter().filter(|r| r.error.is_some()).count();
                let mut parts = Vec::new();
                if new > 0 {
                    parts.push(format!("{new} queued"));
                }
                if resumed > 0 {
                    parts.push(format!("{resumed} resumed"));
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
                self.toast(parts.join(", "));
            }
            Err(e) => self.toast(format!("could not queue: {e}")),
        }
    }

    fn queue_url_list(&mut self, text: &str) {
        let urls: Vec<String> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter(|l| l.starts_with("http://") || l.starts_with("https://") || l.starts_with("ftp://"))
            .map(str::to_string)
            .collect();
        self.queue_urls(urls);
    }

    /// Read URLs from a text file (or a clipboard paste is just typed into the dialog).
    fn import_from_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("URL list", &["txt", "list", "csv"])
            .set_title("Import a list of URLs")
            .pick_file()
        else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => self.queue_url_list(&text),
            Err(e) => self.toast(format!("could not read {}: {e}", path.display())),
        }
    }

    /// Write the URLs of every download in the list to a text file.
    fn export_download_list(&mut self) {
        let mut urls: Vec<String> = self.snap.items.iter().map(|i| i.request.url.clone()).collect();
        urls.sort();
        urls.dedup();
        if urls.is_empty() {
            return self.toast("There is nothing to export");
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("URL list", &["txt"])
            .set_file_name("reidm-urls.txt")
            .set_title("Export the download list")
            .save_file()
        else {
            return;
        };
        let mut text = String::from("# reIDM download list\n");
        for u in &urls {
            text.push_str(u);
            text.push('\n');
        }
        match std::fs::write(&path, text) {
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
            types: self
                .grab
                .types
                .split(',')
                .map(|t| t.trim().trim_start_matches('.').to_ascii_lowercase())
                .filter(|t| !t.is_empty())
                .collect(),
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


    fn open_selected(&mut self, reveal: bool) {
        let Some(id) = self.selected.last().copied() else {
            return self.toast("Select a download first");
        };
        let Some(it) = self.snap.items.iter().find(|i| i.id == id) else { return };
        if !matches!(it.status, Status::Done) && !reveal {
            return self.toast("That download has not finished yet");
        }
        self.api.action(id, if reveal { "folder" } else { "open" });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(s) = self.api.try_recv() {
            self.snap = s;
        }
        self.poll_update();
        self.poll_grab();
        // One check shortly after launch, so a new release is noticed without the user going
        // looking. It is a single GET to api.github.com and reports nothing back to us.
        if !self.checked_on_start && self.snap.connected {
            self.checked_on_start = true;
            self.check_for_updates();
        }

        let ctx = ui.ctx().clone();
        if self.applied_theme != Some(self.dark) {
            theme::apply(&ctx, self.dark);
            self.applied_theme = Some(self.dark);
        }
        let p = theme::palette_for(self.dark);

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
        self.options_dialog(&ctx, &p);
        self.about_dialog(&ctx);
        self.confirm_dialog(&ctx);
        self.grabber_window(&ctx, &p);

        if let Some(t) = &self.toast {
            if Instant::now() < t.until {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -48.0))
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
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
        theme::palette_for(self.dark).bg.to_normalized_gamma_f32()
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
            let _ = &url;
            self.about = false;
            self.api.open_url(&url);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::top("menu")
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(8, 4)))
            .show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    // IDM's own menu order, with the parts reIDM does not have left out.
                    ui.menu_button("Tasks", |ui| {
                        if ui.button("Add URL…").clicked() {
                            self.show_add = true;
                            ui.close();
                        }
                        ui.separator();
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
                        if ui.button("Exit").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.menu_button("File", |ui| {
                        if ui.button("Add URL…").clicked() {
                            self.show_add = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Open download").clicked() {
                            self.open_selected(false);
                            ui.close();
                        }
                        if ui.button("Open containing folder").clicked() {
                            self.open_selected(true);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Delete from list").clicked() {
                            self.confirm_remove = self.selected.last().copied();
                            ui.close();
                        }
                        if ui.button("Delete completed").clicked() {
                            self.delete_completed();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Import URL list…").clicked() {
                            self.import_from_file();
                            ui.close();
                        }
                        if ui.button("Export URL list…").clicked() {
                            self.export_download_list();
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
                        ui.separator();
                        for c in CATEGORIES {
                            if ui.button(c.label()).clicked() {
                                self.cat = c;
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("View", |ui| {
                        ui.checkbox(&mut self.dark, "Dark theme");
                        ui.separator();
                        ui.checkbox(&mut self.cats_open, "Show categories");
                        ui.separator();
                        if ui.button("Site Grabber").clicked() {
                            self.show_grabber = true;
                            ui.close();
                        }
                    });
                    ui.menu_button("Help", |ui| {
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
                        let (txt, col) = if self.snap.connected {
                            ("● Service running", p.ok)
                        } else {
                            ("● Service unavailable", p.err)
                        };
                        ui.label(RichText::new(txt).color(col).size(12.0));
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

                    if tool(ui, p, "Add URL", Icon::Plus, true, true).clicked() {
                        self.show_add = true;
                    }
                    sep(ui);
                    if tool(ui, p, "Resume", Icon::Play, false, true).clicked() {
                        self.start_selected();
                    }
                    if tool(ui, p, "Stop", Icon::Pause, false, true).clicked() {
                        self.stop_selected();
                    }
                    if tool(ui, p, "Stop All", Icon::PauseAll, false, true).clicked() {
                        self.stop_all();
                    }
                    sep(ui);
                    if tool(ui, p, "Delete", Icon::Cross, false, true).clicked() {
                        self.confirm_remove = self.selected.last().copied();
                    }
                    if tool(ui, p, "Delete\nCompleted", Icon::CrossAll, false, true).clicked() {
                        self.delete_completed();
                    }
                    sep(ui);
                    if tool(ui, p, "Options", Icon::Gear, false, true).clicked() {
                        if let Some((dir, conn)) = self.api.fetch_settings() {
                            self.opt_dir = dir;
                            self.opt_conn = conn;
                        }
                        self.show_options = true;
                    }
                    if tool(ui, p, "Site\nGrabber", Icon::Grabber, false, true).clicked() {
                        self.show_grabber = true;
                    }
                    // Present in IDM, not implemented here; `tool` takes no click sense when
                    // disabled, and the tooltip says so.
                    let _ = tool(ui, p, "Scheduler", Icon::Clock, false, false);

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
            .default_size(196.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(6, 8)))
            .show(ui, |ui| {
                ui.label(RichText::new("CATEGORIES").size(10.0).color(p.text_dim).strong());
                ui.add_space(2.0);

                // IDM nests its categories under "Categories > Downloads"; mirror that with
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
                    ui.painter().text(r.min + Vec2::new(50.0, 11.0), egui::Align2::LEFT_CENTER, "Downloads", egui::FontId::proportional(12.5), p.text);
                    if resp.clicked() {
                        self.downloads_open = !self.downloads_open;
                    }

                    if self.downloads_open {
                        for c in CATEGORIES {
                            let n = self.count(c);
                            let on = self.cat == c;
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 23.0), egui::Sense::click());
                            if on {
                                ui.painter().rect_filled(rect, 3.0, p.accent.gamma_multiply(0.22));
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, 3.0, p.accent.gamma_multiply(0.09));
                            }
                            let tc = if on { p.accent } else { p.text };
                            let icon_rect = egui::Rect::from_min_size(rect.min + Vec2::new(34.0, 4.0), Vec2::splat(15.0));
                            icons::file_card(ui.painter(), icon_rect, c, self.dark);
                            ui.painter().text(
                                rect.min + Vec2::new(56.0, 11.5),
                                egui::Align2::LEFT_CENTER,
                                c.label(),
                                egui::FontId::proportional(12.5),
                                tc,
                            );
                            ui.painter().text(
                                rect.right_center() - Vec2::new(8.0, 0.0),
                                egui::Align2::RIGHT_CENTER,
                                n.to_string(),
                                egui::FontId::proportional(11.5),
                                p.text_dim,
                            );
                            if resp.clicked() {
                                self.cat = c;
                            }
                        }
                    }
                }

                ui.add_space(6.0);
                // Present in IDM, absent here. Greyed with a reason rather than hidden, so the
                // window does not look like it silently lost features.
                for (name, why) in [
                    ("Queues", "reIDM downloads immediately; there is no queue scheduler yet"),
                    ("Grabber projects", "the site grabber is not implemented in reIDM"),
                    ("Quotas", "download quotas are not implemented in reIDM"),
                ] {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), egui::Sense::hover());
                    icons::chevron(ui.painter(), egui::Rect::from_min_size(r.min + Vec2::new(2.0, 4.0), Vec2::splat(12.0)), p.text_dim.gamma_multiply(0.6), false);
                    ui.painter().text(
                        r.min + Vec2::new(18.0, 10.0),
                        egui::Align2::LEFT_CENTER,
                        name,
                        egui::FontId::proportional(12.0),
                        p.text_dim.gamma_multiply(0.7),
                    );
                    resp.on_hover_text(why);
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
                    ui.label(RichText::new(parts.join("  ·  ")).color(p.text_dim).size(12.0));

                    if self.snap.connected && self.snap.platform == "windows" && !self.snap.idm_port {
                        ui.separator();
                        let warn = "Browser capture: port 1001 is in use by another program";
                        ui.label(RichText::new("⚠").color(p.warn).size(12.0)).on_hover_text(format!(
                            "{warn}. Close the other download manager, or use the native-messaging host, \
                             which works regardless."
                        ));
                        ui.label(RichText::new(warn).color(p.warn).size(12.0));
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.snap.total_rate > 0.0 {
                            ui.label(RichText::new(rate_human(self.snap.total_rate)).color(p.accent).size(12.0).strong());
                            ui.label(RichText::new("Total rate:").color(p.text_dim).size(12.0));
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
                let mut open_req = false;
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
                                "No downloads yet — use Add URL, or let the browser extension hand one over."
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
                    .column(Column::initial(104.0).at_least(84.0)) // status
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
                                    let c = egui::Pos2::new(rect.left() + 2.0 + ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), p.text).size().x + 8.0, rect.center().y);
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
                                icons::file_card(ui.painter(), icon_rect, it.category(), self.dark);
                                let text_x = rect.left() + 28.0;
                                ui.painter().text(
                                    egui::Pos2::new(text_x, rect.top() + 6.0),
                                    egui::Align2::LEFT_TOP,
                                    it.file_name(),
                                    egui::FontId::proportional(13.0),
                                    p.text,
                                );
                                let bar = egui::Rect::from_min_size(
                                    egui::Pos2::new(text_x, rect.bottom() - 8.0),
                                    Vec2::new((rect.right() - text_x - 4.0).max(0.0), 4.0),
                                );
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
                                    open_req = true;
                                }
                                if resp.secondary_clicked() {
                                    ctx_menu = Some(id);
                                }
                            });

                            row.col(|ui| {
                                let t = match it.size {
                                    Some(sz) if it.status != Status::Done && it.downloaded > 0 => {
                                        format!("{} of {}", size_human(it.downloaded), size_human(sz))
                                    }
                                    Some(sz) => size_human(sz),
                                    None => "Unknown".to_string(),
                                };
                                ui.label(RichText::new(t).size(12.0).color(p.text_dim));
                            });

                            row.col(|ui| {
                                let c = p.status_color(&it.status);
                                let (dr, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                                icons::dot(ui.painter(), dr, c);
                                ui.label(RichText::new(it.status.label()).size(12.0).color(c));
                            });

                            row.col(|ui| {
                                let txt = match (&it.status, it.size, rate) {
                                    (Status::Running, Some(sz), Some(r)) if r > 1.0 && sz > it.downloaded => {
                                        eta_human((sz - it.downloaded) as f64 / r)
                                    }
                                    _ => String::new(),
                                };
                                ui.label(RichText::new(txt).size(12.0).color(p.text_dim));
                            });

                            row.col(|ui| {
                                let txt = if it.status == Status::Running { rate.map(rate_human).unwrap_or_default() } else { String::new() };
                                ui.label(
                                    RichText::new(txt)
                                        .size(12.0)
                                        .color(if it.status == Status::Running { p.accent } else { p.text_dim }),
                                );
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
                if open_req {
                    self.open_selected(false);
                }
                if let Some(id) = ctx_menu {
                    if !self.selected.contains(&id) {
                        self.select_only(id);
                    }
                    let it = self.snap.items.iter().find(|i| i.id == id).cloned();
                    if let Some(it) = it {
                        let pos = ctx.input(|i| i.pointer.hover_pos()).unwrap_or_default();
                        egui::Area::new(egui::Id::new("ctxmenu"))
                            .fixed_pos(pos)
                            .order(egui::Order::Foreground)
                            .show(ctx, |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    ui.set_min_width(200.0);
                                    if ui.button("Open file").clicked() {
                                        ctx_cmd = Some(CtxCmd::Open);
                                        ui.close();
                                    }
                                    if ui.button("Open containing folder").clicked() {
                                        ctx_cmd = Some(CtxCmd::Folder);
                                        ui.close();
                                    }
                                    ui.separator();
                                    if it.status.is_active() && ui.button("Stop").clicked() {
                                        ctx_cmd = Some(CtxCmd::Stop);
                                        ui.close();
                                    }
                                    if it.status.is_stopped() && ui.button("Resume").clicked() {
                                        ctx_cmd = Some(CtxCmd::Start);
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.button("Copy URL").clicked() {
                                        ctx_cmd = Some(CtxCmd::CopyUrl(it.request.url.clone()));
                                        ui.close();
                                    }
                                    if ui.button("Remove from list").clicked() {
                                        ctx_cmd = Some(CtxCmd::Remove);
                                        ui.close();
                                    }
                                    if ui.button("Remove and delete file").clicked() {
                                        ctx_cmd = Some(CtxCmd::RemoveFile);
                                        ui.close();
                                    }
                                });
                            });
                    }
                }
                match ctx_cmd {
                    Some(CtxCmd::Open) => self.open_selected(false),
                    Some(CtxCmd::Folder) => self.open_selected(true),
                    Some(CtxCmd::Stop) => self.stop_selected(),
                    Some(CtxCmd::Start) => self.start_selected(),
                    Some(CtxCmd::CopyUrl(u)) => {
                        ctx.copy_text(u);
                        self.toast("URL copied");
                    }
                    Some(CtxCmd::Remove) => {
                        if let Some(id) = self.selected.last().copied() {
                            self.remove_id(id, false)
                        }
                    }
                    Some(CtxCmd::RemoveFile) => {
                        if let Some(id) = self.selected.last().copied() {
                            self.remove_id(id, true)
                        }
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
    CopyUrl(String),
    Remove,
    RemoveFile,
}

// ── dialogs ──────────────────────────────────────────────────────────────

impl App {
    fn add_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.show_add {
            return;
        }
        let mut open = true;
        egui::Window::new("Add a download")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(520.0);
                ui.label(RichText::new("Addresses — one per line, so you can paste a whole batch").color(p.text_dim).size(12.0));
                ui.add_sized(
                    Vec2::new(500.0, 96.0),
                    egui::TextEdit::multiline(&mut self.add_url)
                        .hint_text("https://example.com/file.zip\nhttps://example.com/another.iso")
                        .desired_rows(4),
                );
                let count = reidm_core::grabber::parse_url_list(&self.add_url).len();
                if count > 1 {
                    ui.label(RichText::new(format!("{count} URLs")).color(p.accent).size(11.0));
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Save as (single URL only)").color(p.text_dim).size(12.0));
                ui.add_sized(
                    Vec2::new(500.0, 26.0),
                    egui::TextEdit::singleline(&mut self.add_name).hint_text("(leave blank to use the name from the server)"),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Import list…").clicked() {
                        self.import_from_file();
                    }
                    ui.label(RichText::new("Read URLs from a text file").color(p.text_dim).size(11.0));
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let ok = ui.add_sized(Vec2::new(140.0, 28.0), egui::Button::new(if count > 1 { "Queue all" } else { "Start download" }));
                    let cancel = ui.add_sized(Vec2::new(80.0, 28.0), egui::Button::new("Cancel"));
                    if ok.clicked() {
                        let text = self.add_url.clone();
                        let urls = reidm_core::grabber::parse_url_list(&text);
                        if urls.is_empty() {
                            self.toast("Enter an http or https address first");
                        } else if urls.len() == 1 {
                            let name = self.add_name.trim();
                            self.api.add(&urls[0], if name.is_empty() { None } else { Some(name.to_string()) });
                            self.add_url.clear();
                            self.add_name.clear();
                            self.show_add = false;
                            self.toast("Download queued");
                        } else {
                            self.queue_urls(urls);
                            self.add_url.clear();
                            self.show_add = false;
                        }
                    }
                    if cancel.clicked() {
                        self.show_add = false;
                    }
                });
            });
        if !open {
            self.show_add = false;
        }
    }

    fn options_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.show_options {
            return;
        }
        let mut open = true;
        let mut save = false;
        egui::Window::new("Options")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(480.0);
                ui.label(RichText::new("Downloads").strong());
                ui.add_space(6.0);
                ui.label(RichText::new("Default download folder").color(p.text_dim).size(12.0));
                ui.horizontal(|ui| {
                    ui.add_sized(Vec2::new(360.0, 26.0), egui::TextEdit::singleline(&mut self.opt_dir));
                    if ui.button("Browse…").clicked() {
                        if let Some(d) = rfd::FileDialog::new().set_directory(&self.opt_dir).pick_folder() {
                            self.opt_dir = d.to_string_lossy().into_owned();
                        }
                    }
                });
                ui.add_space(10.0);
                ui.label(RichText::new("Connections per download").color(p.text_dim).size(12.0));
                ui.add(egui::Slider::new(&mut self.opt_conn, 1..=32));
                ui.label(
                    RichText::new("More connections can be faster on some servers and slower on others. 8 is a good default.")
                        .color(p.text_dim)
                        .size(11.0),
                );
                ui.add_space(12.0);
                ui.label(RichText::new("Appearance").strong());
                ui.add_space(4.0);
                ui.checkbox(&mut self.dark, "Dark theme");
                ui.add_space(12.0);
                ui.label(RichText::new("Updates").strong());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Check now").clicked() {
                        self.check_for_updates();
                    }
                    ui.label(
                        RichText::new("Uses the GitHub Releases API. Set REIDM_GH_TOKEN to read a private repository.")
                            .color(p.text_dim)
                            .size(11.0),
                    );
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
            self.api.set_settings(&self.opt_dir, self.opt_conn);
            self.show_options = false;
            self.toast("Options saved");
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
        egui::Window::new("About reIDM")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.label(RichText::new(format!("reIDM {}", env!("CARGO_PKG_VERSION"))).strong().size(16.0));
                ui.add_space(6.0);
                ui.label("A cross-platform download manager that speaks the IDM browser-extension protocol.");
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
        let found = std::mem::take(&mut self.grab.found);
        let mut selected = std::mem::take(&mut self.grab.selected);

        egui::Window::new("Site Grabber")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(760.0, 520.0))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(RichText::new("Page address").color(p.text_dim).size(12.0));
                ui.horizontal(|ui| {
                    let r = ui.add_sized(
                        Vec2::new(430.0, 26.0),
                        egui::TextEdit::singleline(&mut self.grab.url).hint_text("https://example.com/downloads/"),
                    );
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.add_enabled(!self.grab.scanning, egui::Button::new("Scan")).clicked() || enter)
                        && !self.grab.scanning
                    {
                        scan = true;
                    }
                    if self.grab.scanning {
                        ui.spinner();
                    }
                });

                ui.add_space(6.0);
                egui::CollapsingHeader::new("Filters")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Follow links").color(p.text_dim).size(12.0));
                            ui.add(egui::DragValue::new(&mut self.grab.depth).range(0..=3));
                            ui.label(
                                RichText::new(if self.grab.depth == 0 { "this page only" } else { "levels deep" })
                                    .color(p.text_dim)
                                    .size(11.0),
                            );
                            ui.separator();
                            ui.checkbox(&mut self.grab.same_host, "Same host only");
                            ui.checkbox(&mut self.grab.sizes, "Ask for file sizes (slower)");
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Types").color(p.text_dim).size(12.0));
                            ui.add_sized(
                                Vec2::new(300.0, 22.0),
                                egui::TextEdit::singleline(&mut self.grab.types)
                                    .hint_text("zip, iso, mp4 … (blank = all common file types)"),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Include").color(p.text_dim).size(12.0));
                            ui.add_sized(Vec2::new(230.0, 22.0), egui::TextEdit::singleline(&mut self.grab.include).hint_text("/pub/"));
                            ui.label(RichText::new("Exclude").color(p.text_dim).size(12.0));
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
                        ui.label(
                            RichText::new("Scan a page to list the files it links to.\nNothing is downloaded until you say so.")
                                .color(p.text_dim),
                        );
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
                                let cat = crate::model::Category::of(&f.ext.to_ascii_uppercase());
                                icons::file_card(ui.painter(), icon_rect, cat, self.dark);
                                ui.add_space(20.0);
                                match f.size {
                                    Some(sz) => ui.label(RichText::new(size_human(sz)).size(11.5).color(p.text_dim)),
                                    None => ui.label(RichText::new("          ").size(11.5)),
                                };
                                ui.label(RichText::new(&f.filename).size(12.0).color(p.text)).on_hover_text(
                                    if f.found_on.is_empty() { f.url.clone() } else { format!("{}\nfound on {}", f.url, f.found_on) },
                                );
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
                    let chosen = selected.iter().filter(|s| **s).count();
                    let all: Vec<String> = found.iter().map(|f| f.url.clone()).collect();
                    if ui
                        .add_enabled(chosen > 0, egui::Button::new(format!("Download {chosen} selected")))
                        .clicked()
                    {
                        let urls: Vec<String> =
                            found.iter().zip(selected.iter()).filter(|(_, s)| **s).map(|(f, _)| f.url.clone()).collect();
                        self.queue_urls(urls);
                    }
                    if ui.add_enabled(!all.is_empty(), egui::Button::new("Download all")).clicked() {
                        self.queue_urls(all.clone());
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            self.show_grabber = false;
                        }
                    });
                });
            });

        self.grab.found = found;
        self.grab.selected = selected;
        if scan {
            self.start_grab();
        }
        if export {
            self.export_download_list();
        }
        if import {
            self.import_from_file();
        }
        if !open {
            self.show_grabber = false;
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_remove else { return };
        let name = self.snap.items.iter().find(|i| i.id == id).map(|i| i.file_name()).unwrap_or_default();
        let mut open = true;
        egui::Window::new("Remove download")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(390.0);
                ui.label(format!("Remove “{name}” from the list?"));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Remove from list only").clicked() {
                        self.remove_id(id, false);
                        self.confirm_remove = None;
                    }
                    if ui.button("Remove and delete the file").clicked() {
                        self.remove_id(id, true);
                        self.confirm_remove = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_remove = None;
                    }
                });
            });
        if !open {
            self.confirm_remove = None;
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
    Grabber,
}

/// An IDM-style toolbar button: a large icon with a small caption underneath. Two-line
/// captions (e.g. "Delete\nCompleted") are supported, which is why the height is fixed
/// rather than derived from the text.
fn tool(ui: &mut egui::Ui, p: &Palette, label: &str, icon: Icon, primary: bool, enabled: bool) -> egui::Response {
    let lines: Vec<&str> = label.split('\n').collect();
    let widest = lines
        .iter()
        .map(|l| ui.painter().layout_no_wrap((*l).to_string(), egui::FontId::proportional(10.5), p.text).size().x)
        .fold(0.0f32, f32::max);
    let w = (widest + 16.0).max(56.0);
    let h = 46.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), if enabled { egui::Sense::click() } else { egui::Sense::hover() });

    let fg = if !enabled {
        p.text_dim.gamma_multiply(0.55)
    } else if primary {
        p.accent_text
    } else {
        p.text
    };

    if primary && enabled {
        let bg = if resp.hovered() { p.accent.gamma_multiply(0.85) } else { p.accent };
        ui.painter().rect_filled(rect, 5.0, bg);
    } else if enabled && resp.hovered() {
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
        Icon::Grabber => icons::grabber(painter, icon_rect, fg),
    }
    for (i, line) in lines.iter().enumerate() {
        painter.text(
            egui::Pos2::new(rect.center().x, rect.top() + 29.0 + i as f32 * 11.0),
            egui::Align2::CENTER_TOP,
            *line,
            egui::FontId::proportional(10.5),
            fg,
        );
    }

    if enabled {
        resp.on_hover_text(label.replace('\n', " "))
    } else {
        resp.on_hover_text("Not implemented in reIDM")
    }
}
