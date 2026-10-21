//! The scheduler and the speed limiter.

use super::*;

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

impl App {
    pub(super) fn scheduler_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.sched.open {
            return;
        }
        let mut open = true;
        let mut save = false;
        let mut queue: Option<bool> = None;
        let in_queue = self.vm.counts.queued;
        let s = &mut self.sched;
        window("Scheduler").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(470.0);
            ui.checkbox(&mut s.enabled, RichText::new("Run the scheduled queue on a schedule").strong());
            ui.add_space(8.0);
            ui.add_enabled_ui(s.enabled, |ui| {
                ui.horizontal(|ui| {
                    label(ui, p, "Start at");
                    ui.add_sized(Vec2::new(70.0, 24.0), egui::TextEdit::singleline(&mut s.start).hint_text("02:00"));
                    ui.add_space(12.0);
                    ui.checkbox(&mut s.use_stop, "Stop at");
                    ui.add_enabled(s.use_stop, egui::TextEdit::singleline(&mut s.stop).desired_width(70.0).hint_text("07:00"));
                });
                label(ui, p, "Local time, 24-hour clock. A stop time earlier than the start time means the next morning.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    label(ui, p, "On");
                    for (i, d) in DAYS.iter().enumerate() {
                        ui.toggle_value(&mut s.days[i], *d);
                    }
                });
            });
            ui.add_space(10.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{in_queue} download(s) in the scheduled queue")).color(p.text_dim));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Stop now").clicked() {
                        queue = Some(false);
                    }
                    if ui.button("Start now").clicked() {
                        queue = Some(true);
                    }
                });
            });
            label(ui, p, "Put downloads in the queue with Download Later, or right-click › Add to the scheduled queue. They start up to the 'downloads at the same time' limit.");
            if let Some(e) = &s.error {
                ui.label(RichText::new(e).color(p.err).size(12.0));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Save")).clicked() {
                    save = true;
                }
                if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Cancel")).clicked() {
                    s.open = false;
                }
            });
        });
        if save {
            let days: Vec<u8> = if s.days.iter().all(|d| *d) { vec![] } else { (0..7u8).filter(|d| s.days[*d as usize]).collect() };
            let sched = Schedule {
                enabled: s.enabled,
                start: Some(s.start.trim().to_string()).filter(|t| !t.is_empty()),
                stop: if s.use_stop { Some(s.stop.trim().to_string()).filter(|t| !t.is_empty()) } else { None },
                days,
            };
            if s.enabled && s.days.iter().all(|d| !*d) {
                s.error = Some("Pick at least one day".into());
            } else {
                let enabled = sched.enabled;
                self.sched.open = false;
                self.save_settings(serde_json::json!({ "schedule": sched }), move |app, r| match r {
                    Ok(()) => app.toast(if enabled { "Schedule saved" } else { "Schedule turned off" }),
                    Err(e) => {
                        // Back to the dialog, as edited, to put it right.
                        app.sched.open = true;
                        app.sched.error = Some(e);
                    }
                });
            }
        }
        if let Some(start) = queue {
            self.queue_now(start);
        }
        if !open {
            self.sched.open = false;
        }
    }

    pub(super) fn limiter_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.limiter.open {
            return;
        }
        let mut open = true;
        let mut save = false;
        let l = &mut self.limiter;
        window("Speed limiter").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut l.on, "Limit the total download speed to");
                ui.add_enabled(l.on, egui::DragValue::new(&mut l.kb).range(1..=10_000_000).suffix(" KB/s"));
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                label(ui, p, "Presets");
                for (txt, kb) in [("256 KB/s", 256), ("512 KB/s", 512), ("1 MB/s", 1024), ("5 MB/s", 5120), ("10 MB/s", 10240)] {
                    if ui.small_button(txt).clicked() {
                        l.kb = kb;
                        l.on = true;
                    }
                }
            });
            label(ui, p, "Shared by every download and connection, and applied immediately. A single download can have its own lower limit in its Properties.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Save")).clicked() {
                    save = true;
                }
                if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Cancel")).clicked() {
                    l.open = false;
                }
            });
        });
        if save {
            let (on, kb) = (self.limiter.on, self.limiter.kb);
            self.limiter.open = false;
            self.save_settings(serde_json::json!({ "limit_enabled": on, "speed_limit": kb as u64 * 1024 }), move |app, r| match r {
                Ok(()) => app.toast(if on { format!("Total speed limited to {kb} KB/s") } else { "Speed limiter off".into() }),
                Err(e) => {
                    app.limiter.open = true;
                    app.toast(format!("Could not save: {e}"));
                }
            });
        }
        if !open {
            self.limiter.open = false;
        }
    }
}
