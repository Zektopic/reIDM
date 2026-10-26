//! *Refresh download address*.

use super::*;

impl App {
    pub(super) fn refresh_window(&mut self, ctx: &egui::Context, p: &Palette) {
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
}
