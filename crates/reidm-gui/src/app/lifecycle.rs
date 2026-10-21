//! What the window does whether or not it is showing: take in the service's news, tell the
//! user about finished and failed downloads, run the tray icon, follow *When done*, and decide
//! what closing the window means.
//!
//! eframe calls `App::logic` before every frame and also while the window is hidden in the
//! tray (when something asks for a repaint), so all of this lives there rather than in `ui`.

use super::*;
use crate::notify::{self, OnFinish};
use crate::tray::{self, Tray};

impl App {
    pub(super) fn run_logic(&mut self, ctx: &egui::Context) {
        if let Some(s) = self.poller.rx.try_iter().last() {
            self.snap = s;
            self.note_completions();
        }
        for r in self.cmd.replies() {
            r(self);
        }
        // Here rather than in `ui`: the tray's tooltip needs the counts while the window is
        // hidden. It costs a comparison when nothing changed.
        self.vm.update(&self.snap, self.view, &self.search, self.sort, self.desc);
        self.poll_update();
        self.poll_grab();
        self.poll_refresh();
        self.poll_notifications();
        // One check shortly after launch, so a new release is noticed without the user going
        // looking. It is a single GET to api.github.com and reports nothing back to us.
        if !self.checked_on_start && self.snap.connected {
            self.checked_on_start = true;
            self.check_for_updates();
        }
        self.tray_logic(ctx);
        self.when_done_logic(ctx);
        self.clipboard_logic(ctx);
        self.handle_close(ctx);

        // Poll as often as what is on screen deserves.
        let (focused, minimized) = ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.viewport().minimized.unwrap_or(false)));
        let busy = self.vm.counts.running > 0 || self.snap.total_rate > 0.0;
        self.poller.pace.set(Pace::for_window(busy, focused, minimized || self.hidden));
    }

    // ── the tray ─────────────────────────────────────────────────────────

    fn tray_logic(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.tray_probe {
            if let Ok(found) = rx.try_recv() {
                self.tray_available = Some(found);
                self.tray_probe = None;
            }
        }
        // The icon follows Options › General, at once.
        let want = self.prefs.tray && self.tray_available == Some(true);
        if want && self.tray.is_none() && !self.tray_failed {
            match Tray::new(ctx) {
                Ok(t) => self.tray = Some(t),
                Err(e) => {
                    // Carry on without one; closing then closes, as with no tray at all.
                    self.tray_failed = true;
                    self.toast(format!("Could not show reIDM's icon in the notification area: {e}"));
                }
            }
        } else if !want && self.tray.is_some() {
            self.tray = None;
            if self.hidden {
                self.show_window(ctx);
            }
        }
        let Some(t) = &self.tray else { return };
        let commands = t.commands();
        for c in commands {
            match c {
                tray::Command::Show => self.show_window(ctx),
                tray::Command::AddUrl => {
                    self.show_window(ctx);
                    self.open_add();
                }
                tray::Command::PauseAll => self.stop_all(),
                tray::Command::ResumeAll => self.start_all(),
                tray::Command::ToggleLimiter => {
                    // With no rate chosen yet this opens the limiter, which has to be seen.
                    if self.known_settings().is_none_or(|s| !s.limit_enabled && s.speed_limit == 0) {
                        self.show_window(ctx);
                    }
                    self.toggle_limiter();
                }
                tray::Command::WhenDone(a) => self.set_when_done(a),
                tray::Command::Quit => self.ask_quit(ctx),
            }
        }
        let c = &self.vm.counts;
        let state = tray::State {
            tooltip: tray::tooltip(c.running, &rate_human(self.snap.total_rate)),
            limiter: self.snap.service.settings.as_ref().map(|s| (s.limit_enabled, if s.speed_limit > 0 { rate_human(s.speed_limit as f64) } else { String::new() })),
            when_done: self.snap.service.when_done.map(|w| w.action),
            can_pause: c.running + c.waiting > 0,
            can_resume: c.paused + c.failed > 0,
        };
        if let Some(t) = &mut self.tray {
            t.show_state(&state);
        }
    }

    /// Hide the window in the notification area.
    fn hide_window(&mut self, ctx: &egui::Context) {
        self.hidden = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    /// Bring the window back from the tray, or from being minimised, to the front.
    pub(super) fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    /// What closing (or minimising) the window means: into the tray when there is one, else
    /// closed, with a word the first time that downloads carry on.
    fn handle_close(&mut self, ctx: &egui::Context) {
        let (close, minimized) = ctx.input(|i| (i.viewport().close_requested(), i.viewport().minimized.unwrap_or(false)));
        if close && !self.quitting {
            if self.tray.is_some() && self.prefs.close_to_tray {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if self.prefs.tray_notified {
                    self.hide_window(ctx);
                } else {
                    self.confirm = Some(Confirm::Close { tray: true });
                }
            } else if !self.prefs.close_explained && self.snap.connected {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm = Some(Confirm::Close { tray: false });
            }
        }
        if minimized && !self.hidden && self.tray.is_some() && self.prefs.minimize_to_tray {
            self.hide_window(ctx);
        }
    }

    /// *Quit*: everything, the service too. Asks first when that stops downloads.
    pub(super) fn ask_quit(&mut self, ctx: &egui::Context) {
        let running = self.snap.items.iter().filter(|i| i.status.is_active()).count();
        if running == 0 {
            return self.quit_completely(ctx);
        }
        self.show_window(ctx);
        self.confirm = Some(Confirm::Quit(running));
    }

    /// Close the window itself, even with a tray to close into.
    pub(super) fn close_window(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// The one-time word on closing, answered: into the tray, or closed.
    pub(super) fn closed_explained(&mut self, ctx: &egui::Context, tray: bool) {
        self.prefs.close_explained = true;
        if tray {
            self.prefs.tray_notified = true;
            self.prefs.save();
            self.hide_window(ctx);
        } else {
            self.prefs.save();
            self.close_window(ctx);
        }
    }

    // ── finished and failed downloads ────────────────────────────────────

    /// Notice downloads that have just finished or failed, and say so the way Options › General
    /// asks.
    fn note_completions(&mut self) {
        if !self.snap.connected || self.snap.rev == self.noted_rev {
            return;
        }
        self.noted_rev = self.snap.rev;
        let snap = self.snap.clone();
        let mut events = self.watch.update(snap.items.iter().map(|a| a.as_ref()));
        self.completed.retain(|id| snap.index.contains_key(id));
        self.selected.retain(|id| snap.index.contains_key(id));
        if !self.prefs.notify_failures {
            events.retain(|e| matches!(e, notify::Event::Finished(_)));
        }
        if events.is_empty() {
            return;
        }
        let name = |id: u64| snap.item(id).map(|i| i.file_name().to_string());
        let why = |id: u64| match snap.item(id).map(|i| &i.status) {
            Some(Status::Failed(e)) => Some(e.clone()),
            _ => None,
        };
        match self.prefs.on_finish(self.tray.is_some()) {
            OnFinish::Nothing => {}
            OnFinish::Dialog => {
                for m in notify::messages(&events, name, why) {
                    self.fall_back(m);
                }
            }
            OnFinish::Notification => {
                for m in notify::messages(&events, name, why) {
                    // A D-Bus or WinRT round trip: not on the window's thread. The answer
                    // comes back to `poll_notifications`.
                    let (tx, rx) = std::sync::mpsc::channel();
                    let (msg, ctx) = (m.clone(), self.ctx.clone());
                    std::thread::spawn(move || {
                        let _ = tx.send(notify::show(&msg));
                        ctx.request_repaint();
                    });
                    self.notifying.push((m, rx));
                }
            }
        }
    }

    /// A notification that couldn't be shown is said in the window instead: the *Download
    /// complete* window for finished downloads, a message for failed ones.
    fn fall_back(&mut self, m: notify::Message) {
        if m.failed {
            self.toast(format!("{}: {}", m.title, m.body.replace('\n', " — ")));
        } else {
            for id in m.ids {
                if !self.completed.contains(&id) {
                    self.completed.push(id);
                }
            }
        }
    }

    fn poll_notifications(&mut self) {
        let mut i = 0;
        while i < self.notifying.len() {
            match self.notifying[i].1.try_recv() {
                Ok(Ok(())) => {
                    self.notifying.swap_remove(i);
                }
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let (m, _) = self.notifying.swap_remove(i);
                    self.fall_back(m);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => i += 1,
            }
        }
    }
}
