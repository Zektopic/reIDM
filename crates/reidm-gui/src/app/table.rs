//! The download list and its right-click menu.

use super::*;

impl App {
    pub(super) fn table(&mut self, ui: &mut egui::Ui, p: &Palette) {
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
                        self.copy_text(ctx, u);
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

pub(super) enum CtxCmd {
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
