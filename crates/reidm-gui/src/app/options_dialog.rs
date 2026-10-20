//! The Options dialog.

use super::*;

impl App {
    pub(super) fn options_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
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
    pub(super) fn torrents_tab(&mut self, ui: &mut egui::Ui, p: &Palette) {
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
    pub(super) fn folders_tab(&mut self, ui: &mut egui::Ui, p: &Palette) {
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
}
