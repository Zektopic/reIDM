//! The menu bar.

use super::*;

impl App {
    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui, p: &Palette) {
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
                        if ui.button("Exit").on_hover_text("Close the window; downloads carry on in the background").clicked() {
                            self.close_window(ui.ctx());
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
                            self.ask_quit(ui.ctx());
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
                        self.when_done_menu(ui);
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
                        if ui
                            .add_enabled(self.vm.counts.ordered, egui::Checkbox::new(&mut self.prefs.queue_column, "Show each download's place in the queue (#)"))
                            .on_disabled_hover_text("The download service running keeps no queue order; restart reIDM after updating.")
                            .changed()
                        {
                            self.prefs.save();
                        }
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
}
