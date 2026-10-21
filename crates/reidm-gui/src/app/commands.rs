//! What the window asks the service to do, and the bookkeeping around it.

use super::*;

impl App {
    // ── update checking ──────────────────────────────────────────────────

    /// Kick off a check on a worker thread: it talks to the network and must not block a frame.
    pub(super) fn check_for_updates(&mut self) {
        if self.update_rx.is_some() {
            return;
        }
        if let Some(u) = &mut self.update {
            u.checking = true;
            u.error = None;
        } else {
            self.update = Some(UpdateOffer {
                version: String::new(),
                url: String::new(),
                asset: None,
                notes: String::new(),
                dismissed: false,
                checking: true,
                error: None,
            });
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.update_rx = Some(rx);
        std::thread::spawn(move || {
            let token = reidm_core::update::token_from_env();
            let r = reidm_core::update::check(reidm_core::update::REPO, env!("CARGO_PKG_VERSION"), token.as_deref())
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(r);
        });
    }

    pub(super) fn poll_update(&mut self) {
        let Some(rx) = &self.update_rx else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.update_rx = None;
        match result {
            Ok(Some(rel)) => {
                let asset = rel.asset_for(std::env::consts::OS).map(|a| (a.name.clone(), a.browser_download_url.clone()));
                if let Some(u) = &mut self.update {
                    u.version = rel.version().to_string();
                    u.url = rel.html_url.clone();
                    u.asset = asset;
                    u.notes = rel.body.clone();
                    u.checking = false;
                    u.dismissed = false;
                }
            }
            Ok(None) => {
                self.update = None;
                self.toast(format!("reIDM {} is up to date", env!("CARGO_PKG_VERSION")));
            }
            Err(e) => {
                if let Some(u) = &mut self.update {
                    u.checking = false;
                    u.error = Some(e);
                }
            }
        }
    }

    // ── commands ─────────────────────────────────────────────────────────
    //
    // Each runs on the command thread; its result comes back as a toast.

    /// One action on many downloads, in one request.
    pub(super) fn bulk(&mut self, ids: Vec<u64>, action: &'static str, done: impl FnOnce(usize) -> String + Send + 'static) {
        if ids.is_empty() {
            return;
        }
        self.cmd.run(move |c| {
            let r = c.bulk(&ids, action);
            reply(move |app: &mut App| match r {
                Ok((n, None)) => app.toast(done(n)),
                Ok((0, Some(e))) => app.toast(format!("Could not {}: {e}", action.replace('-', " "))),
                Ok((n, Some(e))) => app.toast(format!("{} — {e}", done(n))),
                Err(e) => app.toast(format!("Could not {}: {e}", action.replace('-', " "))),
            })
        });
    }

    /// A single action whose failure is worth saying (open, seed…).
    pub(super) fn action(&self, id: u64, what: &'static str) {
        self.cmd.run(move |c| {
            let r = c.post(&format!("/api/downloads/{id}/{what}"), serde_json::json!({}));
            r.err().and_then(|e| reply(move |app: &mut App| app.toast(format!("Could not {}: {e}", what.replace('-', " ")))))
        });
    }

    pub(super) fn start_selected(&mut self) {
        let ids: Vec<u64> = self.selected_items().iter().filter(|i| i.status.is_stopped()).map(|i| i.id).collect();
        if ids.is_empty() {
            return self.toast("Select a paused or failed download to resume");
        }
        self.bulk(ids, "resume", |n| format!("Resumed {n} download(s)"));
    }

    pub(super) fn stop_selected(&mut self) {
        let ids: Vec<u64> = self.selected_items().iter().filter(|i| i.status.is_active()).map(|i| i.id).collect();
        if ids.is_empty() {
            return self.toast("Select a running download to pause");
        }
        self.bulk(ids, "pause", |n| format!("Paused {n} download(s)"));
    }

    pub(super) fn stop_all(&mut self) {
        let ids: Vec<u64> = self.snap.items.iter().filter(|i| i.status.is_active()).map(|i| i.id).collect();
        if ids.is_empty() {
            return self.toast("Nothing is downloading");
        }
        self.bulk(ids, "pause", |n| format!("Paused {n} download(s)"));
    }

    pub(super) fn start_all(&mut self) {
        let ids: Vec<u64> = self.snap.items.iter().filter(|i| i.status.is_stopped()).map(|i| i.id).collect();
        if ids.is_empty() {
            return self.toast("Nothing is paused");
        }
        self.bulk(ids, "resume", |n| format!("Resumed {n} download(s)"));
    }

    /// Ask first: restarting throws away what has arrived, and for a finished download replaces
    /// the file.
    pub(super) fn restart_selected(&mut self) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return self.toast("Select a download to restart");
        }
        self.confirm = Some(Confirm::Restart(ids));
    }

    pub(super) fn set_queued(&mut self, queued: bool) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return self.toast("Select a download first");
        }
        self.bulk(ids, if queued { "schedule" } else { "unschedule" }, move |n| match (queued, n) {
            (true, 0) => "Finished downloads can't be added to the scheduled queue".to_string(),
            (true, n) => format!("{n} added to the scheduled queue"),
            (false, n) => format!("{n} removed from the scheduled queue"),
        });
    }

    pub(super) fn queue_now(&mut self, start: bool) {
        self.cmd.run(move |c| {
            let r = c.post(&format!("/api/queue/{}", if start { "start" } else { "stop" }), serde_json::json!({}));
            let r = r.map(|t| serde_json::from_str::<serde_json::Value>(&t).ok().and_then(|v| v["count"].as_u64()).unwrap_or(0));
            reply(move |app: &mut App| match r {
                Ok(0) if start => app.toast("Nothing in the scheduled queue is waiting to start"),
                Ok(0) => app.toast("Nothing in the scheduled queue is running"),
                Ok(n) => app.toast(if start { format!("Started the queue: {n} download(s)") } else { format!("Stopped the queue: {n} download(s)") }),
                Err(e) => app.toast(format!("Could not {} the queue: {e}", if start { "start" } else { "stop" })),
            })
        });
    }

    /// Move the selected unfinished downloads in the queue, as one block in their own order.
    pub(super) fn move_selected(&mut self, to: crate::queue::To) {
        let picked: HashSet<u64> = self.selected.iter().copied().filter(|id| self.vm.places.contains_key(id)).collect();
        if picked.is_empty() {
            return self.toast("Select a download that hasn't finished: only those wait in the queue");
        }
        if !crate::queue::can_move(&self.vm.queue, &picked, to) {
            return;
        }
        let ids: Vec<u64> = self.vm.queue.iter().copied().filter(|id| picked.contains(id)).collect();
        self.cmd.run(move |c| {
            let r: Result<serde_json::Value, String> = c.post_json("/api/downloads/bulk", serde_json::json!({ "action": "move", "to": to.wire(), "ids": ids }));
            let refused = r.as_ref().ok().and_then(|v| v["results"].as_array().into_iter().flatten().find_map(|r| r["error"].as_str().map(str::to_string)));
            match (r, refused) {
                (Err(e), _) | (Ok(_), Some(e)) => reply(move |app: &mut App| app.toast(format!("Could not move them: {e}"))),
                _ => None,
            }
        });
    }

    /// Ask first, with the count: this empties the history, with no undo.
    pub(super) fn delete_completed(&mut self) {
        let done: Vec<u64> = self.snap.items.iter().filter(|i| i.status == Status::Done).map(|i| i.id).collect();
        if done.is_empty() {
            return self.toast("There are no completed downloads");
        }
        self.confirm = Some(Confirm::ClearCompleted(done));
    }

    pub(super) fn ask_remove(&mut self) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return self.toast("Select the downloads to remove");
        }
        self.confirm = Some(Confirm::Remove(ids));
    }

    /// Stop the background service too, and close the window. Downloads carry on from where
    /// they stopped the next time reIDM starts.
    pub(super) fn quit_completely(&mut self, ctx: &egui::Context) {
        self.prefs.close_explained = true;
        self.prefs.save();
        let ctx = ctx.clone();
        self.cmd.run(move |c| {
            let r = c.post("/api/shutdown", serde_json::json!({}));
            reply(move |app: &mut App| match r {
                Ok(_) => app.close_window(&ctx),
                Err(e) => app.toast(format!("Could not stop the background service: {e}")),
            })
        });
    }

    pub(super) fn remove_ids(&mut self, ids: &[u64], purge: bool) {
        for id in ids {
            self.selected.remove(id);
        }
        self.bulk(ids.to_vec(), if purge { "purge" } else { "remove" }, move |n| match (purge, n) {
            (true, 1) => "Download and file removed".to_string(),
            (true, n) => format!("{n} downloads and their files removed"),
            (false, 1) => "Download removed from the list".to_string(),
            (false, n) => format!("{n} downloads removed from the list"),
        });
    }

    /// Set the service's settings, then say how it went.
    pub(super) fn save_settings(&self, patch: serde_json::Value, done: impl FnOnce(&mut App, Result<(), String>) + Send + 'static) {
        self.cmd.run(move |c| {
            let r = c.post("/api/settings", patch).map(|_| ());
            reply(move |app: &mut App| done(app, r))
        });
    }

    pub(super) fn report_batch(&mut self, results: Result<Vec<BatchResult>, String>, later: bool) {
        match results {
            Ok(results) => {
                let count = |k: &str| results.iter().filter(|r| r.outcome.as_deref() == Some(k)).count();
                let new = count("new");
                let resumed = count("existing");
                let have = count("already_complete");
                let failed = results.iter().filter(|r| r.error.is_some()).count();
                let mut parts = Vec::new();
                if new > 0 {
                    parts.push(if later { format!("{new} added to the queue") } else { format!("{new} queued") });
                }
                if resumed > 0 {
                    parts.push(format!("{resumed} {}", if later { "already listed" } else { "resumed" }));
                }
                if have > 0 {
                    parts.push(format!("{have} already downloaded"));
                }
                if failed > 0 {
                    parts.push(format!("{failed} failed"));
                }
                if let Some(first) = results.iter().find(|r| r.error.is_some()) {
                    parts.push(format!("— {}: {}", first.url, first.error.clone().unwrap_or_default()));
                }
                self.toast(if parts.is_empty() { "Nothing to add".to_string() } else { parts.join(", ") });
            }
            Err(e) => self.toast(format!("could not queue: {e}")),
        }
    }

    /// Queue every URL in a line- or file-shaped list, and say what happened.
    pub(super) fn queue_urls(&mut self, urls: Vec<String>, later: bool) {
        if urls.is_empty() {
            return self.toast("No URLs to add");
        }
        self.queue_batch(serde_json::json!({ "urls": urls, "later": later }), later);
    }

    pub(super) fn queue_batch(&mut self, body: serde_json::Value, later: bool) {
        self.toast("Adding…");
        self.cmd.run(move |c| {
            // A big batch can keep the service busy for a while.
            let r = c.with_timeout(Duration::from_secs(600)).batch(body);
            reply(move |app: &mut App| app.report_batch(r, later))
        });
    }

    /// Open the Add URL dialog, with whatever links are on the clipboard.
    pub(super) fn open_add(&mut self) {
        if self.add.urls.trim().is_empty() && self.prefs.clipboard {
            if let Some(text) = clipboard_text() {
                let urls = reidm_core::grabber::parse_url_list(&text);
                if !urls.is_empty() {
                    self.add.urls = urls.join("\n");
                }
            }
        }
        if self.add.dir.is_empty() {
            if let Some(s) = &self.snap.service.settings {
                self.add.dir = s.default_dir.clone();
            }
        }
        self.add.open = true;
    }

    /// Read URLs from a text file or an IDM `.ef2` export.
    pub(super) fn import_from_file(&mut self) {
        self.picker.open(&self.ctx, PickFor::Import, Ask::Open { title: "Import downloads", filter: ("URL lists and IDM exports", &["txt", "list", "csv", "ef2"]) });
    }

    /// Queue the downloads in a URL list or an IDM `.ef2` export.
    pub(super) fn import_path(&mut self, path: &std::path::Path) {
        match std::fs::read(path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                if reidm_core::lists::looks_like_ef2(&text) {
                    let entries = reidm_core::lists::parse_ef2(&text);
                    if entries.is_empty() {
                        return self.toast("That IDM export holds no downloads");
                    }
                    self.queue_batch(serde_json::json!({ "items": entries, "later": false }), false);
                } else {
                    self.queue_urls(reidm_core::grabber::parse_url_list(&text), false);
                }
            }
            Err(e) => self.toast(format!("could not read {}: {e}", path.display())),
        }
    }

    /// Write the download list as plain URLs or as IDM's `.ef2`: first, where.
    pub(super) fn export_download_list(&mut self, ef2: bool) {
        if self.snap.items.is_empty() {
            return self.toast("There is nothing to export");
        }
        let (format, filter): (&str, (&'static str, &'static [&'static str])) = if ef2 { ("ef2", ("IDM export", &["ef2"])) } else { ("txt", ("URL list", &["txt"])) };
        self.picker.open(&self.ctx, PickFor::Export { ef2 }, Ask::Save { title: "Export the download list", name: format!("reidm-downloads.{format}"), filter });
    }

    fn export_list_to(&mut self, ef2: bool, path: std::path::PathBuf) {
        let format = if ef2 { "ef2" } else { "txt" };
        let n = self.snap.items.len();
        self.cmd.run(move |c| {
            let r = c.with_timeout(Duration::from_secs(120)).get(&format!("/api/export?format={format}")).and_then(|text| std::fs::write(&path, text).map_err(|e| e.to_string()));
            reply(move |app: &mut App| match r {
                Ok(()) => app.toast(format!("Exported {n} download(s) to {}", path.display())),
                Err(e) => app.toast(format!("Could not export: {e}")),
            })
        });
    }

    /// Export just the grabber's results as a URL list.
    pub(super) fn export_found(&mut self, urls: Vec<String>) {
        self.picker.open(&self.ctx, PickFor::ExportFound(urls), Ask::Save { title: "Export the files found", name: "grabbed-urls.txt".into(), filter: ("URL list", &["txt"]) });
    }

    /// A picker closed with a path: do what it was opened for.
    pub(super) fn poll_picker(&mut self) {
        let Some((purpose, path)) = self.picker.poll() else { return };
        let dir = path.to_string_lossy().into_owned();
        match purpose {
            PickFor::AddDir => self.add.dir = dir,
            PickFor::DownloadDir => self.options.form.dir = dir,
            PickFor::CategoryDir(name) => {
                if let Some((_, d)) = self.options.form.folders.iter_mut().find(|(n, _)| *n == name) {
                    *d = dir;
                }
            }
            PickFor::TempDir => self.options.form.temp = dir,
            PickFor::WelcomeDir => self.save_settings(serde_json::json!({ "default_dir": dir }), |app, r| {
                if let Err(e) = r {
                    app.toast(format!("Could not change the download folder: {e}"));
                }
            }),
            PickFor::Import => self.import_path(&path),
            PickFor::Export { ef2 } => self.export_list_to(ef2, path),
            PickFor::ExportFound(urls) => match std::fs::write(&path, reidm_core::grabber::url_list_text(&urls)) {
                Ok(()) => self.toast(format!("Exported {} URL(s)", urls.len())),
                Err(e) => self.toast(format!("could not write {}: {e}", path.display())),
            },
        }
    }

    // ── site grabber ─────────────────────────────────────────────────────

    pub(super) fn start_grab(&mut self) {
        let url = self.grab.url.trim().to_string();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.grab.error = Some("Enter an http or https page address".into());
            return;
        }
        let opts = GrabOptions {
            depth: self.grab.depth,
            types: self.grab.types.split(',').map(|t| t.trim().trim_start_matches('.').to_ascii_lowercase()).filter(|t| !t.is_empty()).collect(),
            same_host_only: self.grab.same_host,
            include: self.grab.include.lines().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
            exclude: self.grab.exclude.lines().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
            sizes: self.grab.sizes,
        };
        self.grab.rx = Some(self.client.grab_async(&url, &opts));
        self.grab.scanning = true;
        self.grab.error = None;
    }

    pub(super) fn poll_grab(&mut self) {
        let Some(rx) = &self.grab.rx else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.grab.rx = None;
        self.grab.scanning = false;
        match result {
            Ok(files) => {
                self.grab.selected = vec![true; files.len()];
                self.grab.found = files;
                if self.grab.found.is_empty() {
                    self.grab.error = Some("No downloadable files found on that page".into());
                }
            }
            Err(e) => {
                self.grab.found.clear();
                self.grab.selected.clear();
                self.grab.error = Some(e);
            }
        }
    }

    pub(super) fn poll_refresh(&mut self) {
        let mut done: Option<Result<(), String>> = None;
        if let Some(r) = &mut self.refresh {
            if let Some(rx) = &r.busy {
                if let Ok(res) = rx.try_recv() {
                    r.busy = None;
                    done = Some(res);
                }
            }
        }
        match done {
            Some(Ok(())) => {
                self.refresh = None;
                self.toast("New address accepted — the download carries on from where it was");
            }
            Some(Err(e)) => {
                if let Some(r) = &mut self.refresh {
                    r.error = Some(e);
                }
            }
            None => {}
        }
        // Capturing from the browser: it ends when the daemon stops waiting.
        let finished = self.refresh.as_ref().filter(|r| r.capturing).and_then(|r| {
            let still_armed = self.snap.service.refresh_armed == Some(r.id);
            let now_url = self.item(r.id).map(|i| i.request.url.clone()).unwrap_or_default();
            if now_url != r.old_url {
                Some(true)
            } else if !still_armed && self.snap.connected {
                Some(false)
            } else {
                None
            }
        });
        match finished {
            Some(true) => {
                self.refresh = None;
                self.toast("The browser's link is in — the download carries on from where it was");
            }
            Some(false) => {
                if let Some(r) = &mut self.refresh {
                    r.capturing = false;
                    r.error = Some("Stopped waiting for the browser (it waits three minutes). Try again, or paste the address.".into());
                }
            }
            None => {}
        }
    }

    pub(super) fn open_selected(&mut self, reveal: bool) {
        let Some(id) = self.focus.filter(|id| self.selected.contains(id)) else {
            return self.toast("Select a download first");
        };
        self.open_id(id, reveal);
    }

    pub(super) fn open_id(&mut self, id: u64, reveal: bool) {
        let Some(it) = self.item(id) else { return };
        if !matches!(it.status, Status::Done) && !reveal {
            return self.toast("That download has not finished yet");
        }
        self.action(id, if reveal { "folder" } else { "open" });
    }

    pub(super) fn open_props(&mut self, id: u64) {
        let limit = self.item(id).and_then(|i| i.speed_limit);
        self.props = Some(PropsState { id, limit_on: limit.is_some(), limit_kb: limit.map(|l| (l / 1024) as u32).unwrap_or(512).max(1), detail: None, fetched: None, fetching: false, algo: crate::checksum::Algo::Sha256, expected: String::new(), verifying: None, verified: None });
    }

    /// Keep Properties' full copy of its download fresh: about once a second, and never more
    /// than one request at a time.
    pub(super) fn refresh_props(&mut self) {
        let Some(st) = &mut self.props else { return };
        if st.fetching || st.fetched.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        st.fetching = true;
        let id = st.id;
        self.cmd.run(move |c| {
            let d = c.detail(id).ok();
            reply(move |app: &mut App| {
                if let Some(st) = app.props.as_mut().filter(|st| st.id == id) {
                    st.fetching = false;
                    st.fetched = Some(Instant::now());
                    if d.is_some() {
                        st.detail = d;
                    }
                }
            })
        });
    }

    pub(super) fn open_refresh(&mut self, id: u64) {
        if self.item(id).is_some_and(|i| i.request.media.is_some()) {
            self.toast("A video download finds its own address each time it starts: there is nothing to refresh. Start it again.");
            return;
        }
        let old_url = self.item(id).map(|i| i.request.url.clone()).unwrap_or_default();
        self.refresh = Some(RefreshState { id, url: String::new(), busy: None, error: None, capturing: false, old_url });
    }

    /// The service's settings as last polled (at most a few seconds old), or why there are none.
    pub(super) fn known_settings(&self) -> Option<crate::model::Settings> {
        self.snap.service.settings.clone()
    }

    pub(super) fn open_scheduler(&mut self) {
        let Some(s) = self.known_settings().map(|s| s.schedule) else {
            return self.toast("The scheduler's settings haven't loaded yet: is the download service running?");
        };
        let mut days = [false; 7];
        for d in &s.days {
            if let Some(slot) = days.get_mut(*d as usize) {
                *slot = true;
            }
        }
        if s.days.is_empty() {
            days = [true; 7];
        }
        self.sched = SchedEdit {
            open: true,
            enabled: s.enabled,
            start: s.start.clone().unwrap_or_else(|| "02:00".into()),
            use_stop: s.stop.is_some(),
            stop: s.stop.clone().unwrap_or_else(|| "07:00".into()),
            days,
            error: None,
        };
    }

    pub(super) fn open_limiter(&mut self) {
        let Some(s) = self.known_settings() else {
            return self.toast("The speed limiter's settings haven't loaded yet: is the download service running?");
        };
        self.limiter = LimiterEdit { open: true, on: s.limit_enabled, kb: if s.speed_limit > 0 { (s.speed_limit / 1024) as u32 } else { 1024 } };
    }

    /// Turn the limiter on or off from the menu. With no rate chosen yet, "on" would limit
    /// nothing, so that opens the limiter's settings instead.
    pub(super) fn toggle_limiter(&mut self) {
        let Some(s) = self.known_settings() else { return self.open_limiter() };
        if !s.limit_enabled && s.speed_limit == 0 {
            return self.open_limiter();
        }
        let on = !s.limit_enabled;
        self.save_settings(serde_json::json!({ "limit_enabled": on }), move |app, r| match r {
            Ok(()) => app.toast(if on { "Speed limiter on" } else { "Speed limiter off" }),
            Err(e) => app.toast(format!("Could not change the speed limiter: {e}")),
        });
    }

    /// *Add YouTube playlist*, with a playlist link from the clipboard already filled in.
    pub(super) fn open_playlist(&mut self) {
        let s = self.known_settings().unwrap_or_default();
        let dir = |n: &str| s.folders.dirs.get(n).cloned().unwrap_or_else(|| s.default_dir.clone());
        let (video, music) = if s.folders.sort { (dir("Video"), dir("Music")) } else { (s.default_dir.clone(), s.default_dir.clone()) };
        let clip = arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()).map(|t| t.trim().to_string()).filter(|t| t.starts_with("http") && (t.contains("list=") || t.contains("/@") || t.contains("/channel/")));
        self.playlist.open_with(clip, video, music);
    }
}
