//! Confirmations before what can't be undone.

use super::*;

impl App {
    /// Ask before what can't be undone. Esc or closing the dialog cancels; Enter takes the
    /// harmless choice, never the one that deletes.
    pub(super) fn confirm_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
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
}
