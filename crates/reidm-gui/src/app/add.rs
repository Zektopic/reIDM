//! *Add URL* and *Add batch download*.

use super::*;

impl App {
    pub(super) fn add_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
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
            if self.vm.counts.checksums {
                ui.add_space(4.0);
                egui::CollapsingHeader::new("More options").id_salt("add-more").default_open(!self.add.checksum.is_empty()).show(ui, |ui| {
                    label(ui, p, "Expected checksum (single URL only)");
                    ui.add_enabled(count <= 1, egui::TextEdit::singleline(&mut self.add.checksum).hint_text("sha256:9f86d0…  or just the hex").desired_width(500.0));
                    match crate::checksum::parse(&self.add.checksum) {
                        Err(e) => {
                            ui.label(RichText::new(e).color(p.err).size(12.0));
                        }
                        Ok(Some(c)) => label(ui, p, &format!("The finished file is checked against this {} checksum; a mismatch marks the download failed and keeps the file.", c.algo.label())),
                        Ok(None) => label(ui, p, "From the download page, if it gives one (MD5, SHA-1, SHA-256 or SHA-512)."),
                    }
                });
            }
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
                let checksum = match crate::checksum::parse(&self.add.checksum) {
                    Ok(c) => c.map(|c| c.json()),
                    Err(e) => return self.toast(format!("The expected checksum: {e}")),
                };
                let mut body = serde_json::json!({ "url": urls[0], "filename": name, "dir": dir, "later": later });
                if let Some(c) = checksum {
                    body["checksum"] = c;
                }
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

    pub(super) fn batch_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
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
}
