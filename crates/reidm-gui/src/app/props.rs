//! A download's Properties.

use super::*;

impl App {
    /// One download's details: addresses, sizes, its connections as a segment map, and its own
    /// speed limit. IDM shows this as the download progress dialog.
    pub(super) fn props_window(&mut self, ctx: &egui::Context, p: &Palette) {
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
                    match it.torrent.as_ref().and_then(|t| t.folder.as_ref()) {
                        Some(f) => row(ui, "Saved in", f.display().to_string()),
                        None => row(ui, "Saved to", it.path.display().to_string()),
                    }
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
}
