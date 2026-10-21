//! *When done*: the choice in the Downloads menu and the tray, the countdown before the service
//! acts, and closing the window when the service exits.

use super::*;
use crate::when_done::Action;

impl App {
    pub(super) fn set_when_done(&mut self, a: Action) {
        self.cmd.run(move |c| {
            let r = c.post("/api/when-done", serde_json::json!({ "action": a.wire() }));
            reply(move |app: &mut App| match r {
                Ok(_) => app.toast(a.chosen()),
                Err(e) => app.toast(format!("Could not change what happens when the downloads are done: {e}")),
            })
        });
    }

    fn cancel_when_done(&mut self) {
        self.cmd.run(|c| {
            let r = c.post("/api/when-done/cancel", serde_json::json!({}));
            reply(move |app: &mut App| match r {
                Ok(_) => app.toast("Cancelled. Nothing happens when the downloads are done."),
                Err(e) => app.toast(format!("Could not cancel: {e}")),
            })
        });
    }

    /// The menu's radio list. Disabled with a service that can't do it.
    pub(super) fn when_done_menu(&mut self, ui: &mut egui::Ui) {
        let current = self.snap.service.when_done.map(|w| w.action);
        ui.add_enabled_ui(current.is_some(), |ui| {
            ui.menu_button("When done", |ui| {
                for a in Action::ALL {
                    if ui.radio(current == Some(a), a.label()).clicked() {
                        self.set_when_done(a);
                        ui.close();
                    }
                }
            })
        })
        .response
        .on_disabled_hover_text("The download service running is too old for this; restart reIDM after updating.");
    }

    pub(super) fn when_done_logic(&mut self, ctx: &egui::Context) {
        if self.snap.service.exiting && !self.quitting {
            // *When done › Exit*: the service is going, and the window with it.
            self.close_window(ctx);
            return;
        }
        let Some(w) = self.snap.service.when_done else { return };
        let Some(at) = w.fires_at.filter(|at| self.countdown_cancelled != Some(*at)) else { return };
        if w.countdown(now_secs()).is_some() {
            // Shown wherever the window was: a shutdown nobody can see coming is a bad surprise.
            if self.hidden || self.countdown_seen != Some(at) {
                self.countdown_seen = Some(at);
                self.show_window(ctx);
            }
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    /// The countdown, with Cancel. Escape cancels too.
    pub(super) fn countdown_window(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(w) = self.snap.service.when_done else { return };
        let Some(at) = w.fires_at.filter(|at| self.countdown_cancelled != Some(*at)) else { return };
        let Some((_, text)) = w.countdown(now_secs()) else { return };
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        window("All downloads are done").order(egui::Order::Foreground).show(ctx, |ui| {
            ui.set_min_width(380.0);
            ui.label(RichText::new(text).strong().size(15.0));
            ui.add_space(4.0);
            label(ui, p, "Started again because something was added? The countdown stops by itself.");
            ui.add_space(10.0);
            let b = ui.add(egui::Button::new(RichText::new("Cancel").strong()).min_size(Vec2::new(110.0, 30.0)));
            b.request_focus();
            if b.clicked() {
                cancel = true;
            }
        });
        if cancel {
            self.countdown_cancelled = Some(at);
            self.cancel_when_done();
        }
    }
}
