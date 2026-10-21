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
            Confirm::Close { tray: true } => (
                "reIDM keeps downloading",
                "reIDM is still running in the notification area.".to_string(),
                "Downloads carry on, and the browser keeps handing them to reIDM. Click reIDM's icon to open this window again, or right-click it to quit.".to_string(),
            ),
            Confirm::Close { tray: false } => (
                "reIDM keeps downloading",
                "Downloads carry on in the background when this window is closed.".to_string(),
                "The browser keeps handing downloads to reIDM, and opening reIDM again shows them. To stop everything, choose Quit reIDM completely (also in the File menu).".to_string(),
            ),
            Confirm::Quit(n) => (
                "Quit reIDM",
                format!("Quit reIDM while {} running?", if *n == 1 { "a download is".to_string() } else { format!("{n} downloads are") }),
                "They stop now, and carry on from where they were the next time reIDM starts. Downloads from the browser won't be caught until then.".to_string(),
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
                    Confirm::Close { tray } => {
                        if ui.button(RichText::new(if *tray { "OK" } else { "Keep downloading in the background" }).strong()).clicked() || enter {
                            choice = Some("close");
                        }
                        if ui.add(danger("Quit reIDM completely")).clicked() {
                            choice = Some("quit");
                        }
                    }
                    Confirm::Quit(_) => {
                        if ui.add(danger("Quit")).clicked() {
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
            (Confirm::Close { tray }, "close") => self.closed_explained(ctx, tray),
            (Confirm::Close { .. } | Confirm::Quit(_), "quit") => self.quit_completely(ctx),
            _ => {}
        }
    }
}
