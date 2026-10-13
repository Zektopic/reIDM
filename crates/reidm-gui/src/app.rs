//! The reIDM desktop window: toolbar, category tree, download table, status bar and dialogs.

use crate::api::{Api, Snapshot};
use crate::icons;
use crate::model::{eta_human, rate_human, size_human, Category, Item, Status, CATEGORIES};
use crate::theme::{self, Palette};
use egui::{Align, Color32, Layout, RichText, Vec2};
use egui_extras::{Column, TableBuilder};
use std::time::{Duration, Instant};

#[derive(PartialEq, Clone, Copy)]
enum SortKey {
    Name,
    Size,
    Status,
    Rate,
}

struct Toast {
    text: String,
    until: Instant,
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
    /// The theme currently installed in egui, so we restyle only on an actual change.
    applied_theme: Option<bool>,
    toast: Option<Toast>,

    // dialogs
    show_add: bool,
    add_url: String,
    add_name: String,
    add_start: bool,
    show_options: bool,
    opt_dir: String,
    opt_conn: usize,
    confirm_remove: Option<u64>,
    about: bool,
    settings_loaded: bool,
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
            add_start: true,
            show_options: false,
            opt_dir: String::new(),
            opt_conn: 8,
            confirm_remove: None,
            about: false,
            settings_loaded: false,
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), until: Instant::now() + Duration::from_secs(4) });
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
            };
            if self.desc { o.reverse() } else { o }
        });
        v
    }

    fn count(&self, c: Category) -> usize {
        self.snap.items.iter().filter(|i| c.matches(i)).count()
    }

    /// Selection helpers -------------------------------------------------

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

    /// Actions -----------------------------------------------------------

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

    fn remove_id(&mut self, id: u64, purge: bool) {
        self.api.remove(id, purge);
        self.selected.retain(|x| *x != id);
        self.toast(if purge { "Download and file removed" } else { "Download removed from the list" });
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
        let ctx = ui.ctx().clone();
        // `self.dark` is the source of truth; restyle whenever it changes.
        if self.applied_theme != Some(self.dark) {
            theme::apply(&ctx, self.dark);
            self.applied_theme = Some(self.dark);
        }
        let p = theme::palette_for(self.dark);

        // Panel order matters: outermost first, central panel last.
        self.menu_bar(ui, &p);
        self.toolbar(ui, &p);
        self.status_bar(ui, &p);
        self.sidebar(ui, &p);
        self.table(ui, &p);

        self.add_dialog(&ctx, &p);
        self.options_dialog(&ctx, &p);
        self.about_dialog(&ctx);
        self.confirm_dialog(&ctx);

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

        // Keep the UI live while downloads are moving.
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::palette_for(self.dark).bg.to_normalized_gamma_f32()
    }
}

// ── chrome ───────────────────────────────────────────────────────────────

impl App {
    fn menu_bar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::top("menu")
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(8, 4)))
            .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Downloads", |ui| {
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
                        self.api.stop_all();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.dark, "Dark theme");
                    if ui.button("Refresh now").clicked() {
                        ui.close();
                    }
                });
                ui.menu_button("Help", |ui| {
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
                        ui.add_space(4.0);
                        let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, 20.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 0.0, p.border);
                        ui.add_space(4.0);
                    };

                    if tool(ui, p, "Add URL", Icon::Plus, true).clicked() {
                        self.show_add = true;
                    }
                    sep(ui);
                    if tool(ui, p, "Start", Icon::Play, false).clicked() {
                        self.start_selected();
                    }
                    if tool(ui, p, "Stop", Icon::Pause, false).clicked() {
                        self.stop_selected();
                    }
                    if tool(ui, p, "Delete", Icon::Cross, false).clicked() {
                        self.confirm_remove = self.selected.last().copied();
                    }
                    sep(ui);
                    if tool(ui, p, "Open", Icon::Folder, false).clicked() {
                        self.open_selected(false);
                    }
                    if tool(ui, p, "Folder", Icon::Folder, false).clicked() {
                        self.open_selected(true);
                    }
                    sep(ui);
                    if tool(ui, p, "Options", Icon::Gear, false).clicked() {
                        if !self.settings_loaded {
                            self.show_options = true;
                        } else {
                            self.show_options = true;
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (sr, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
                        icons::search(ui.painter(), sr, p.text_dim);
                        let mut s = std::mem::take(&mut self.search);
                        ui.add_sized(Vec2::new(190.0, 24.0), egui::TextEdit::singleline(&mut s).hint_text("Search…"));
                        self.search = s;
                    });
                });
            });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::left("cats")
            .default_size(186.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(6, 8)))
            .show(ui, |ui| {
                ui.label(RichText::new("CATEGORIES").size(10.0).color(p.text_dim).strong());
                ui.add_space(4.0);
                for c in CATEGORIES {
                    let n = self.count(c);
                    let on = self.cat == c;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), egui::Sense::click());
                    if on {
                        ui.painter().rect_filled(rect, 3.0, p.accent.gamma_multiply(0.22));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 3.0, p.accent.gamma_multiply(0.09));
                    }
                    let tc = if on { p.accent } else { p.text };
                    ui.painter().text(
                        rect.left_center() + Vec2::new(10.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        c.label(),
                        egui::FontId::proportional(13.0),
                        tc,
                    );
                    ui.painter().text(
                        rect.right_center() - Vec2::new(10.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        n.to_string(),
                        egui::FontId::proportional(12.0),
                        p.text_dim,
                    );
                    if resp.clicked() {
                        self.cat = c;
                    }
                }
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::bottom("status")
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(10, 5)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let running = self.snap.items.iter().filter(|i| i.status == Status::Running).count();
                    let done = self.snap.items.iter().filter(|i| i.status == Status::Done).count();
                    let failed = self.snap.items.iter().filter(|i| matches!(i.status, Status::Failed(_))).count();

                    let mut parts = vec![format!("{} total", self.snap.items.len())];
                    if running > 0 {
                        parts.push(format!("{running} downloading"));
                    }
                    if done > 0 {
                        parts.push(format!("{done} complete"));
                    }
                    if failed > 0 {
                        parts.push(format!("{failed} failed"));
                    }
                    ui.label(RichText::new(parts.join("  ·  ")).color(p.text_dim).size(12.0));

                    // On Windows the extension dials port 1001 first, so if another download
                    // manager owns it the user should know why the browser is not handing
                    // downloads over — rather than having to find it in a log file.
                    if self.snap.connected && self.snap.platform == "windows" && !self.snap.idm_port {
                        ui.separator();
                        let warn = "Browser capture: port 1001 is in use by another program".to_string();
                        ui.label(RichText::new("⚠").color(p.warn).size(12.0))
                            .on_hover_text(format!(
                                "{warn}. Close the other download manager, or use the native-messaging host, \
                                 which works regardless."
                            ));
                        ui.label(RichText::new(warn).color(p.warn).size(12.0));
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.snap.total_rate > 0.0 {
                            ui.label(RichText::new(rate_human(self.snap.total_rate)).color(p.accent).size(12.0).strong());
                            ui.label(RichText::new("Total rate:").color(p.text_dim).size(12.0));
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
                let mut open_req = false;
                let mut ctx_menu: Option<u64> = None;
                let mut ctx_cmd: Option<CtxCmd> = None;

                if rows.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.label(RichText::new(if self.snap.items.is_empty() {
                            "No downloads yet — use Add URL, or let the browser extension hand one over."
                        } else {
                            "Nothing in this category."
                        })
                        .color(p.text_dim));
                    });
                    return;
                }

                // header sort clicks
                let set_sort = |k: SortKey, desc: &mut bool, sort: &mut SortKey| {
                    if *sort == k {
                        *desc = !*desc;
                    } else {
                        *sort = k;
                    }
                };

                let builder = TableBuilder::new(ui)
                    .striped(false)
                    .resizable(true)
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .column(Column::initial(320.0).at_least(150.0).clip(true)) // file name
                    .column(Column::initial(90.0).at_least(70.0)) // size
                    .column(Column::initial(100.0).at_least(80.0)) // status
                    .column(Column::initial(88.0).at_least(70.0)) // time left
                    .column(Column::initial(96.0).at_least(78.0)) // rate
                    .column(Column::remainder().at_least(120.0).clip(true)); // description

                let (mut sort_click, mut desc) = (None, self.desc);

                builder
                    .header(24.0, |mut header| {
                        let mut h = |header: &mut egui_extras::TableRow, label: &str, k: SortKey| {
                            header.col(|ui| {
                                let arrow = if self.sort == k { if self.desc { " ▲" } else { " ▼" } } else { "" };
                                if ui.add(egui::Label::new(RichText::new(format!("{label}{arrow}")).strong().size(12.0)).sense(egui::Sense::click())).clicked() {
                                    sort_click = Some(k);
                                }
                            });
                        };
                        h(&mut header, "File Name", SortKey::Name);
                        h(&mut header, "Size", SortKey::Size);
                        h(&mut header, "Status", SortKey::Status);
                        h(&mut header, "Time Left", SortKey::Rate);
                        h(&mut header, "Transfer Rate", SortKey::Rate);
                        h(&mut header, "Description", SortKey::Name);
                    })
                    .body(|body| {
                        body.rows(40.0, rows.len(), |mut row| {
                            let idx = row.index();
                            let it = rows[idx];
                            let id = it.id;
                            let is_sel = self.selected.contains(&id);
                            let rate = self.snap.rates.get(&id).copied();

                            row.set_selected(is_sel);
                            if is_sel {
                                row.set_selected(true);
                            }

                            // File name (+ progress bar underneath)
                            row.col(|ui| {
                                let resp = ui.allocate_response(ui.available_size(), egui::Sense::click());
                                let rect = resp.rect;
                                if is_sel {
                                    ui.painter().rect_filled(rect, 0.0, p.accent.gamma_multiply(0.20));
                                }
                                let bar = egui::Rect::from_min_size(rect.left_bottom() + Vec2::new(2.0, -7.0), Vec2::new(rect.width() - 4.0, 4.0));
                                if let Some(f) = it.fraction() {
                                    let col = p.status_color(&it.status);
                                    icons::progress_bar(ui.painter(), bar, f, col, p.border);
                                }
                                ui.painter().text(
                                    rect.left_top() + Vec2::new(4.0, 5.0),
                                    egui::Align2::LEFT_TOP,
                                    it.file_name(),
                                    egui::FontId::proportional(13.0),
                                    p.text,
                                );
                                if let Some(f) = it.fraction() {
                                    ui.painter().text(
                                        rect.left_bottom() + Vec2::new(4.0, -10.0),
                                        egui::Align2::LEFT_BOTTOM,
                                        format!("{:.0}%", f * 100.0),
                                        egui::FontId::proportional(10.0),
                                        p.text_dim,
                                    );
                                }
                                if resp.clicked() {
                                    click = Some((id, ui.input(|i| i.modifiers.ctrl || i.modifiers.command), ui.input(|i| i.modifiers.shift)));
                                }
                                if resp.double_clicked() {
                                    open_req = true;
                                }
                                if resp.secondary_clicked() {
                                    ctx_menu = Some(id);
                                }
                            });

                            // Size
                            row.col(|ui| {
                                let t = match it.size {
                                    Some(s) => size_human(s),
                                    None => "Unknown".to_string(),
                                };
                                let t = if it.size.is_some() && it.status != Status::Done && it.downloaded > 0 {
                                    format!("{} of {}", size_human(it.downloaded), size_human(it.size.unwrap()))
                                } else {
                                    t
                                };
                                ui.label(RichText::new(t).size(12.0).color(p.text_dim));
                            });

                            // Status
                            row.col(|ui| {
                                let c = p.status_color(&it.status);
                                let (dr, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                                icons::dot(ui.painter(), dr, c);
                                ui.label(RichText::new(it.status.label()).size(12.0).color(c));
                            });

                            // Time left
                            row.col(|ui| {
                                let txt = match (&it.status, it.size, rate) {
                                    (Status::Running, Some(sz), Some(r)) if r > 1.0 && sz > it.downloaded => {
                                        eta_human((sz - it.downloaded) as f64 / r)
                                    }
                                    _ => String::new(),
                                };
                                ui.label(RichText::new(txt).size(12.0).color(p.text_dim));
                            });

                            // Transfer rate
                            row.col(|ui| {
                                let txt = if it.status == Status::Running { rate.map(rate_human).unwrap_or_default() } else { String::new() };
                                ui.label(RichText::new(txt).size(12.0).color(if it.status == Status::Running { p.accent } else { p.text_dim }));
                            });

                            // Description (url or error)
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
                                ui.label(RichText::new(txt).size(11.0).color(col)).on_hover_text(tip);
                            });
                        });
                    });

                if let Some(k) = sort_click {
                    set_sort(k, &mut desc, &mut self.sort);
                    self.desc = desc;
                }

                // row click handling
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
                                    ui.set_min_width(190.0);
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
                ui.set_min_width(460.0);
                ui.label(RichText::new("Address").color(p.text_dim).size(12.0));
                let r = ui.add_sized(Vec2::new(440.0, 26.0), egui::TextEdit::singleline(&mut self.add_url).hint_text("https://example.com/file.zip"));
                ui.add_space(8.0);
                ui.label(RichText::new("Save as").color(p.text_dim).size(12.0));
                ui.add_sized(Vec2::new(440.0, 26.0), egui::TextEdit::singleline(&mut self.add_name).hint_text("(leave blank to use the name from the server)"));
                ui.add_space(8.0);
                ui.checkbox(&mut self.add_start, "Start the download immediately");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let ok = ui.add_sized(Vec2::new(110.0, 28.0), egui::Button::new("Start download"));
                    let cancel = ui.add_sized(Vec2::new(80.0, 28.0), egui::Button::new("Cancel"));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) || ok.clicked() {
                        let url = self.add_url.trim().to_string();
                        if url.is_empty() {
                            self.toast("Enter an address first");
                        } else {
                            let name = self.add_name.trim();
                            self.api.add(&url, if name.is_empty() { None } else { Some(name.to_string()) });
                            self.add_url.clear();
                            self.add_name.clear();
                            self.show_add = false;
                            self.toast("Download queued");
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
        // Seed once from the daemon.
        if !self.settings_loaded {
            if self.snap.connected {
                if let Some(s) = self.api.fetch_settings() {
                    self.opt_dir = s.0;
                    self.opt_conn = s.1;
                    self.settings_loaded = true;
                }
            }
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
                ui.add(egui::Slider::new(&mut self.opt_conn, 1..=32).clamping(egui::SliderClamping::Always));
                ui.label(
                    RichText::new("More connections can be faster on some servers and slower on others. 8 is a good default.")
                        .color(p.text_dim)
                        .size(11.0),
                );
                ui.add_space(12.0);
                ui.label(RichText::new("Appearance").strong());
                ui.add_space(4.0);
                ui.checkbox(&mut self.dark, "Dark theme");
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
        egui::Window::new("About reIDM").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
            ui.set_min_width(400.0);
            ui.label(RichText::new(format!("reIDM {}", env!("CARGO_PKG_VERSION"))).strong().size(16.0));
            ui.add_space(6.0);
            ui.label("A cross-platform download manager that speaks the IDM browser-extension protocol.");
            ui.add_space(8.0);
            ui.label(RichText::new("Not affiliated with, endorsed by, or derived from Tonec Inc.").color(egui::Color32::from_gray(140)).size(11.0));
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                self.about = false;
            }
        });
        if !open {
            self.about = false;
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_remove else { return };
        let name = self.snap.items.iter().find(|i| i.id == id).map(|i| i.file_name()).unwrap_or_default();
        let mut open = true;
        egui::Window::new("Remove download").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
            ui.set_min_width(380.0);
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

enum Icon {
    Plus,
    Play,
    Pause,
    Cross,
    Folder,
    Gear,
}

fn tool(ui: &mut egui::Ui, p: &Palette, label: &str, icon: Icon, primary: bool) -> egui::Response {
    let pad = 7.0;
    let icon_sz = 15.0;
    let text_w = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.5), p.text).size().x;
    let w = text_w + icon_sz + pad * 2.0 + 6.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), egui::Sense::click());

    let (bg, bd) = if primary {
        if resp.hovered() {
            (p.accent.gamma_multiply(0.85), p.accent)
        } else {
            (p.accent, p.accent)
        }
    } else if resp.hovered() {
        (p.accent.gamma_multiply(0.16), p.border)
    } else {
        (Color32::TRANSPARENT, Color32::TRANSPARENT)
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 5.0, bg);
    }
    if bd != Color32::TRANSPARENT {
        ui.painter().rect_stroke(rect, 5.0, egui::Stroke::new(1.0, bd), egui::StrokeKind::Inside);
    }

    let fg = if primary { p.accent_text } else { p.text };
    let icon_rect = egui::Rect::from_min_size(rect.min + Vec2::new(pad, (rect.height() - icon_sz) / 2.0), Vec2::splat(icon_sz));
    let painter = ui.painter();
    match icon {
        Icon::Plus => icons::plus(painter, icon_rect, fg),
        Icon::Play => icons::play(painter, icon_rect, fg),
        Icon::Pause => icons::pause(painter, icon_rect, fg),
        Icon::Cross => icons::cross(painter, icon_rect, fg),
        Icon::Folder => icons::folder(painter, icon_rect, fg),
        Icon::Gear => icons::gear(painter, icon_rect, fg),
    }
    painter.text(
        rect.min + Vec2::new(pad + icon_sz + 6.0, rect.height() / 2.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.5),
        fg,
    );
    resp.on_hover_text(label)
}
