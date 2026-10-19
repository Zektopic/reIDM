# reIDM service + desktop app audit

Scope: `crates/reidm/src` (main.rs, web.rs, ws.rs, session.rs, install.rs, launch.rs), `crates/reidm-host`,
`crates/reidm-gui/src`, plus the `reidm-core` functions those handlers call (`Manager::list/add_*/save/remove`).
Static review only: nothing was built or run, and no ports were bound. Branch `feature/torrents` @ a5ad036.
Dependency facts checked in `~/.cargo/registry`: ureq 3.4.2 (`read_to_string` caps bodies at 10 MB),
axum 0.7.9 (the `Json` extractor's default body limit is 2 MB), egui/eframe 0.36.2 (`Slider` clamps by default;
`accesskit` is a default feature that `default-features = false` turns off), serde_json without `preserve_order`.

Items marked **(uncertain)** are inferred from code paths and were not reproduced.

Severity: **Critical** = breaks the product at realistic scale or loses data · **High** = serious
security, responsiveness or correctness problem · **Medium** · **Low**.

---

## Critical

### C1. The GUI can't load a list with more than about 7,000 downloads, and then reports "Service unavailable" for good
- **Where:** `crates/reidm-gui/src/api.rs:602` (`r.body_mut().read_to_string()` is capped at 10 MB by ureq 3,
  `ureq-3.4.2/src/body/mod.rs:30,292`); `api.rs:219-221` (on any error it drops the token and shows "disconnected").
- **Scenario:** a finished download with 8 segments serializes to about **1.36 KB** of compact JSON (measured with a
  realistic URL, referer, UA and path). At about 7,300 items, `/api/downloads` passes 10 MB. Every poll then fails
  with a body-limit error. The window shows "Service unavailable", the list is empty, and all buttons stop working.
  With 32 connections or long signed CDN URLs, this happens at roughly 3,000 items. One torrent with tens of
  thousands of files (`torrent.files` is in every list item) can trigger it alone.
- **Fix:** short term, `r.body_mut().with_config().limit(256 << 20).read_to_string()`. The real fix is F-PERF below
  (a revision/delta API and a list item without `segments` and `torrent.files`).
- **Test:** an integration test that adds 12,000 items to a temporary `Manager`, serves `web::serve` on an ephemeral
  port, and asserts that `api::fetch` succeeds. Add a unit test on the item DTO size (for example, under 400 B
  for a list row).

### C2. Batch adds rewrite all of `downloads.json` once per URL, holding the DB lock on a Tokio worker (O(n²) disk I/O)
- **Where:** `reidm-core/src/lib.rs:1089-1096` (`add_many_with` calls `add_with` per item), `lib.rs:1079`
  (`self.save()` per add), `lib.rs:834-842` (`save` pretty-serializes the **whole** DB and `fs::write`s it **while
  holding `db`**), `lib.rs:1047` (builds a `HashSet<PathBuf>` of every item's path on every add), `lib.rs:1005`
  (a linear duplicate scan). Called synchronously from async handlers `web.rs:538-546` (`/ext/add-batch`) and
  `web.rs:605-613` (`/api/downloads/batch`). The GUI import (`app.rs:566`), the grabber, the batch pattern
  (`app.rs:1968`) and `reidm import/batch` all use it.
- **Scenario:** importing 5,000 URLs into an empty list writes about Σk×700 B ≈ **8.7 GB**. Into a list of
  10,000 items (about 19 MB pretty JSON), it writes about **100 GB** and takes hours. During that time the `db` mutex
  is held for every serialize+write, so `/api/downloads`, progress checkpoints and every other handler stall. A
  Tokio worker is also blocked. The GUI's 10 s agent timeout fires, and the user sees "could not queue: timeout"
  while the import keeps going.
- **Fix:** `add_many_with` takes `db` once, keeps a `HashMap<url_key, id>` index (also fixes the O(n) scan in
  `add_inner`/`add_checked`/`refresh_address`), inserts every item, then calls `save()` **once** and `pump()` once.
  Change `save()` to: serialize under the lock with `to_vec` (compact), drop the lock, then write in
  `spawn_blocking`. Better still, use a debounced writer: a dirty flag plus one background task that writes at most
  every 1–2 s, and on shutdown. Run the handler body in `spawn_blocking`.
- **Test:** a core test that `add_many_with` of 5,000 URLs finishes in under 1 s and writes `downloads.json` exactly
  once (count writes through a test hook, or check mtime). An axum test that `/api/downloads` answers in under
  100 ms while a 5,000-item batch is running.

---

## High

### H1. Polling cost: every 300 ms the whole list is cloned, serialized, parsed and deep-copied twice
- **Where:**
  - Daemon: `web.rs:108-117` calls `Manager::list` (`lib.rs:871-885`), which clones every `Item` and every live
    plan's segments under `db`, then `live`, then each plan lock. It then serializes about 13.6 MB for 10k items.
  - GUI: `api.rs:593-604` parses that JSON, and `api.rs:237-238` makes **two** deep clones of the snapshot
    (`snap.clone()` into the mutex, then `send`).
  - `api.rs:197` and `api.rs:216`: `history.retain(|id,_| items.iter().any(..))` is **O(n²)**: 10⁸ comparisons
    per poll at 10k items.
  - `app.rs:334-336`: `last_status.retain(|id,_| ids.contains(id))` is O(n²) on every snapshot.
  - `Api::snapshot()` (`api.rs:256`) deep-clones again.
- **Scenario:** at 10k items, about 45 MB/s of JSON is produced and parsed on loopback, plus about 3×10⁸ comparisons
  per second, with the window idle. On a low-end PC that is one to two cores busy for a download manager doing
  nothing. Laptop fans and battery suffer.
- **Fix:** see F-PERF: a revision counter with `ETag`/`304`, `?since=rev` deltas, and a lightweight row DTO. GUI:
  `Arc<Snapshot>`/`Arc<[Item]>` instead of clones. Use `HashSet<u64>` of ids for the `retain` calls. Keep rate
  history only for `Running` items (today every item gets a `VecDeque` entry, `api.rs:179`).
- **Test:** a micro-benchmark (criterion, or a `#[ignore]` test) of one poll iteration with 10k items, with budgets
  under 5 ms (GUI processing) and under 10 ms (daemon `list`).

### H2. Per-frame work in `app.rs` scales with the whole list, and runs 5 times a second
- **Where:**
  - `app.rs:340-365` (`visible()` filters and **sorts on every frame**; the Name comparator calls `file_name()`
    and `to_ascii_lowercase()` on both sides, which is about 4 allocations × n log n comparisons).
  - `app.rs:1377` (called from `table`), `app.rs:1238,1245,1261` (`count()` for 10 sidebar nodes; each category
    count calls `Item::category()`, which allocates twice: `model.rs:142,177`).
  - `app.rs:1278-1281` (4 more passes in the status bar).
  - `app.rs:1407,1483,401` (`self.selected.contains` on a `Vec` is O(n·s); after Ctrl+A on 10k rows that is 10⁸
    per frame).
  - `app.rs:2632` (the scheduler counts every frame).
- **Scenario:** 10k items: about 560k allocations for the sort, plus about 200k for the category counts, every
  frame, at 5 fps. With everything selected, the frame time goes past 100 ms and the UI stutters.
- **Fix:** keep a derived `ViewModel` cache keyed by `(snapshot_rev, view, search, sort, desc)`. It holds the
  ordered visible ids, per-view counts and status counts, and is rebuilt only when the key changes. Compute
  `name_lower`, `category` and `file_name` once at deserialization, as fields on the GUI `Item`. Store selection
  in a `HashSet<u64>` (keep an anchor for shift-selection). Add an `id → index` `HashMap` for `item(id)`
  (`app.rs:371`).
- **Test:** a unit test that `ViewModel::build` is not called again when the key is unchanged. A benchmark with
  10k items showing a frame build under 2 ms.

### H3. The window repaints every 200 ms and polls every 300 ms forever, even when idle or unfocused
- **Where:** `app.rs:905` (`ctx.request_repaint_after(200ms)`, unconditional), `api.rs:239` (a fixed 300 ms poll,
  regardless of focus or activity).
- **Scenario:** with nothing downloading, a low-end PC still runs a full UI pass (with H2's cost) 5 times a second.
  The poller also keeps fetching the full list 3 times a second.
- **Fix:**
  - Pass `egui::Context` to the poller and call `ctx.request_repaint()` only when the snapshot revision changes.
  - Repaint on a 1 s timer only while something is running or a toast or spinner is visible, so the relative
    "Last Try" times still update.
  - Make polling adaptive: 300–500 ms when something is `Running`/`Queued` and the window is focused; 2–5 s when
    idle, unfocused or minimized (`ctx.input(|i| i.viewport().focused/minimized)`, shared via an `AtomicU64`
    interval).
  - **(uncertain)** Whether eframe 0.36 skips `ui()` while minimized depends on the platform; do not rely on it.
- **Test:** a headless `egui::Context` test that one `update` with an unchanged snapshot requests no repaint.
  Log the poll interval in a debug build.

### H4. The GUI keeps a stale API token after the service restarts, so every command fails with 401
- **Where:** `api.rs:260-266` (`Api::token` caches the first token forever; nothing clears it). Only the poller
  clears its own copy (`api.rs:220`).
- **Scenario:** the service restarts (crash, an update, the extension's native host starting a fresh one). The
  poller re-reads the token, so the list keeps refreshing. But Pause, Resume, Add, Options Save and Delete all get
  401. `action()` shows "pause failed: " with an empty body, and `post` shows "HTTP 401 Unauthorized". The only
  way out is to restart the GUI.
  Related: when no token file exists, `read_token()` (`api.rs:81-93`) **sleeps up to 15 s on the UI thread**
  (60 × 250 ms) inside any command.
- **Fix:** keep one `Arc<Mutex<Option<String>>>` shared by the poller and the commands. On a 401, clear it, re-read
  the file once without sleeping, and retry once. Never sleep in a path the UI thread calls.
- **Test:** a mock HTTP server that rotates its token; assert that `Api::post` succeeds after the rotation, and
  that `Api::post` returns within 50 ms when the token file is missing.

### H5. Blocking network and file I/O on the egui thread
- **Where:** these call the synchronous `Api::post`/`remove`/`fetch_settings`/`export` on the UI thread (10 s
  agent timeout):
  - `delete_completed` loops over every finished item (`app.rs:466-476`). With 5,000 done items that is 5,000
    sequential requests, each of which makes the daemon rewrite `downloads.json` (C2). The window freezes for
    minutes.
  - `remove_ids` (`app.rs:478-481`), from Delete, the confirm dialog and the context menu.
  - `restart_selected` (`app.rs:430-441`) and Properties › Restart (`app.rs:2541`). The daemon's `restart` waits up
    to **15 s** for the transfer to stop (`lib.rs:1212-1217`), so each restart can freeze the window for 10 s and
    then report a false timeout error.
  - `set_queued` loop (`app.rs:449`), `queue_now` (`app.rs:458`), the limiter toggle (`app.rs:1077`).
  - `open_scheduler`/`open_limiter`/`open_options`/`open_playlist` (`app.rs:727-783`) call `fetch_settings()`,
    even though the poller already holds `snap.service.settings` (at most 3 s old).
  - Saves: Options (`app.rs:2064`), scheduler (`2693`), limiter (`2745`), per-download limit (`2531`),
    refresh-capture (`2608`), Add (`1880`), `queue_urls`/import (`527`, `566`), and export (`590`; the whole list,
    then `fs::write`).
  - `rfd::FileDialog` is modal and blocking (`app.rs:551,582,598,1845,2003,2138,2156`, `torrent.rs:77`). That is
    acceptable on Windows and macOS. On Linux with the xdg portal, the egui loop stops and the window may be
    flagged "not responding" **(uncertain)**. `rfd::AsyncFileDialog` avoids this.
  - `torrent.rs:78` reads the .torrent and base64-encodes it on the UI thread (small, fine).
- **Fix:** one command worker thread with an `mpsc` queue of commands. Results come back as messages the frame
  polls; show a toast or spinner while they are pending. Add bulk endpoints such as
  `POST /api/downloads/bulk {ids:[…], action:"pause|resume|remove|purge|schedule|unschedule"}` and
  `DELETE /api/downloads?status=done`, so "Delete completed" is one request and one save. Open dialogs from
  `snap.service.settings` and refresh in the background.
- **Test:** a mock server that delays every response by 2 s. Drive `App::ui` headlessly, click Delete completed,
  and assert the frame returns in under 16 ms.

### H6. Options › Save can reset folders, torrent settings and concurrency to defaults
- **Where:**
  - `app.rs:754-774`: the dialog opens even when `fetch_settings()` fails, with constructor defaults
    (`app.rs:224-234`: empty `opt_folders`, `opt_temp = ""`, seed 1.0, port 6881, DHT on, 8 connections,
    4 at a time).
  - `app.rs:2062-2075`: Save sends **every** field. `category_dirs: {}` means "replace every choice"
    (`web.rs:801`, `lib.rs:796-798`, `docs/API.md` "replaces every choice"), so all custom category folders are
    lost.
  - `app.rs:2014`: the "Downloads at the same time" slider ranges 1..=16, while the API allows 1–64. egui's default
    `SliderClamping::Always` (`egui-0.36.2/src/widgets/slider.rs:74`) silently lowers a value of 32 (set through
    the CLI or API) to 16 on the next Save.
- **Scenario:** the service is busy (C2) or starting up. The user opens Options, changes the theme and presses Save.
  Their Video and Music folders, temporary folder, seeding ratio and torrent port are reset.
- **Fix:** don't open Options, or disable Save, until settings have loaded; show "Loading…" or the error. Send only
  fields the user changed (diff against the loaded copy). Set the slider range to 1..=64 (or use
  `SliderClamping::Never`) to match the API.
- **Test:** a GUI unit test where `fetch_settings` returns `None`: assert Save is disabled. A diff test where
  changing only `dark` sends no settings POST.

### H7. Files taken from the browser lose Mark-of-the-Web (Windows) and quarantine (macOS)
- **Where:** nowhere in `crates/` writes `:Zone.Identifier` or `com.apple.quarantine` (grep finds nothing).
  Files are created in `reidm-core` and moved by `move_into_place` (`lib.rs:567`). The GUI's Open runs
  `explorer <file>` (`launch.rs:43-51`), which **executes** a downloaded `.exe`/`.lnk`/`.hta`.
- **Scenario:** the extension takes over every browser download. A malicious `.exe` or Office document then
  arrives without MOTW. SmartScreen, Office Protected View, and the "file came from the internet" prompts are all
  bypassed. The browser alone would have tagged the file, so reIDM lowers the user's protection.
- **Fix:** on Windows, after the move, write the ADS `path:Zone.Identifier` containing
  `[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=…\r\nHostUrl=…`, or call `IAttachmentExecute::Save`. On macOS, set the
  `com.apple.quarantine` xattr (`0081;<hex time>;reIDM;<uuid>`). Do the same for yt-dlp and torrent outputs. Let
  "Open" go through the shell's default verb only after MOTW is set.
- **Test:** a Windows CI test that downloads from a local HTTP server and asserts `file:Zone.Identifier` exists with
  `ZoneId=3`. A macOS test with `xattr -p com.apple.quarantine`.

### H8. The IDM WebSocket accepts unauthenticated pause, resume and **remove+purge** control messages
- **Where:** `session.rs:289-304` (`T_CTL_REMOVE` with `arg(1)!=0` calls `mgr.remove(id, true)`, which deletes the
  finished file). `ws.rs:24-29` accepts any connection with no `Origin` header. The ports are 61001 and 1001 (and
  1001 is the port Tonec's extension dials). `reidm-host/src/main.rs:158-164` relays *any* message from Tonec's
  extension unfiltered. The CLI uses this path (`main.rs:726-728`).
- **Scenario:** any local process, **including another OS user on a shared machine** (the API token protects only
  `/api/*`), can loop over ids 1..N and send `T_CTL_REMOVE` with purge, deleting the user's downloaded files.
  `docs/ARCHITECTURE.md` says other users "could add downloads"; in fact they can also delete files. A
  compromised or updated third-party (Tonec) extension can do the same through the native host.
- **Fix:** remove the `T_CTL_*` handlers from `Session`. Move `reidm pause/resume/rm` to the token-protected HTTP
  API, which also gives the CLI error reporting: today `reidm rm 999` "succeeds" silently. As a defence in depth,
  drop message types ≥ 1000 in the host's relay.
- **Test:** a `session.rs` unit test asserting that `T_CTL_REMOVE` is ignored (no reply that it was done, item
  still listed). A CLI test that `reidm rm 999` exits non-zero with "no such download".

### H9. A corrupt or empty `downloads.json` stops the service from starting, and writes have no fsync
- **Where:** `lib.rs:679-681` (`serde_json::from_slice(&b).context("corrupt downloads.json")?` makes `Manager::open`
  fail and the daemon exit). `lib.rs:834-841` (tmp write, then rename; no `sync_all` on the file or directory;
  errors ignored with `let _`). The same applies to `save_settings` (`824-832`).
- **Scenario:** a power cut or hard reset soon after a save. On ext4 with delayed allocation, NTFS, or a removable
  drive, the renamed file can be zero-length or truncated. The daemon then refuses to start. The GUI shows
  "Service unavailable" with no explanation (the error only goes to a hidden stderr). The extension's host
  reports "did not answer". If the disk is full, saves fail silently and progress is lost.
- **Fix:** `File::sync_all()` before the rename, and fsync the directory on Unix. Keep `downloads.json.bak` (the
  last good copy). On a parse failure, rename the bad file to `downloads.json.corrupt-<ts>`, load the `.bak` or start
  empty, and report it through `/api/status` so the GUI can show a banner. Log or surface save errors instead of
  `let _`.
- **Test:** write garbage to `downloads.json`, run `Manager::open`, and assert it succeeds, keeps the corrupt copy,
  and reports a warning. Do the same with a valid `.bak`.

---

## Medium

### M1. `/ext/*` skips the `Host` check: DNS rebinding can read `/ext/ping`, and any page can fake "extension connected"
- **Where:** `web.rs:350-358` (`extension_caller` lets a request without `Origin` through and never checks `Host`).
  `web.rs:391-418` (a ping with `?v=` records `ExtSeen`).
- **Scenario:**
  - A page rebinds its hostname to 127.0.0.1 and fetches `http://evil.example:61002/ext/ping` same-origin.
    Same-origin GETs carry no `Origin`, so the response is readable. It reveals `default_dir` (the home path and
    so the user name), every category folder, the temporary folder and the version.
  - Separately, any web page can embed `<img src="http://127.0.0.1:61002/ext/ping?v=x">` (no `Origin` on that
    GET). The GUI then shows "Browser: connected" (`app.rs:1300`) even though the extension isn't installed.
  - Chrome's Private Network Access may block some of this **(uncertain, it varies by version and flags)**.
- **Fix:** apply the same `Host` allow-list as `guard()` to `/ext/*`. Record a check-in only when
  `is_extension_origin(h)` is true, or when a custom header (for example `X-Reidm-Ext: 1`) is present: a custom
  header forces a preflight, which pages can't pass. Return the folder map only to extension origins.
- **Test:** axum `oneshot` tests: `GET /ext/ping` with `Host: evil.example:61002` returns 403; `GET /ext/ping?v=1`
  without Origin or the header does not change `/api/status.extension`.

### M2. Request size limits are implicit and inconsistent
- **Where:**
  - No `DefaultBodyLimit` is configured (`web.rs:870-890`), so axum's 2 MB `Json` limit applies everywhere.
  - Torrent upload is base64 inside JSON (`web.rs:144-157`, `api.rs:470-476`), so a `.torrent` larger than about
    1.5 MB gets a bare `413` (the GUI toast says only "HTTP 413…"). Meanwhile the core accepts a 16 MB `.torrent`
    fetched from the web (`lib.rs:926`).
  - `/ext/add-batch` allows up to 5,000 items (`web.rs:521`), but 5,000 items with cookies are well over 2 MB.
  - `/api/downloads/batch` has no item cap at all (`web.rs:573-618`); only the 2 MB limit bounds it, at about
    20–30k URLs.
- **Fix:** set the limit per route with `DefaultBodyLimit::max(…)`: about 24 MB for `/api/downloads` and
  `/api/torrent/inspect`, about 8 MB for the batch routes, 64 KB elsewhere. Cap batch items (for example 10,000)
  with a clear JSON error. Accept a raw `application/x-bittorrent` body, which avoids the 33% base64 overhead.
  Return a readable message for 413 (`/ext` expects JSON).
- **Test:** POST a 3 MB torrent and assert success. POST 10,001 URLs and assert a 400 with a message.

### M3. Unbounded body reads: a captured `.torrent` link can make the service run out of memory
- **Where:** `lib.rs:925-929`. The `content_length() > 16 MiB` check is skipped for chunked responses or ones
  without a length, and then `r.bytes()` reads everything. `grabber.rs:408-411` has the same pattern.
- **Scenario:** any website link ending in `.torrent` is taken by the extension with `verify`. The server streams an
  endless chunked body, and the service runs out of memory and crashes, losing in-flight progress.
- **Fix:** read with `bytes_stream()` and stop at 16 MiB. Do the same for the grabber at 8 MiB.
- **Test:** a local server that streams 100 MB chunked; assert `add_checked` errors at 16 MiB.

### M4. Remove deletes the part file while connections may still be writing to it
- **Where:** `lib.rs:1220-1226`. `pause` only sets the cancel flag; `remove_file(part)` runs immediately afterwards.
- **Scenario:**
  - On Windows the part file is still open, so `remove_file` fails silently and an orphan `.reidm` part (possibly
    many GB) stays in `parts/`.
  - On Unix the inode is unlinked, but writers keep filling disk space until they notice the cancel.
  - If the transfer was finishing, `move_into_place` can rename the part into place *after*
    "Remove and delete the file", so the file appears anyway **(uncertain race)**.
- **Fix:** make `remove` async and call `stop_and_wait(id)` before deleting, as `restart` does. Alternatively, mark a
  tombstone and do the deletion in the transfer task's exit path.
- **Test:** start a throttled download, remove it with purge, wait, and assert no `.reidm` or final file remains
  (on Windows CI too).

### M5. Blocking std calls in async handlers, and zombie processes
- **Where:**
  - `lib.rs:999` (`create_dir_all` in `add_inner`).
  - `lib.rs:759-771` (`create_dir_all` in `update_settings`; on an unreachable network share this can block for
    tens of seconds).
  - `lib.rs:1226-1244` (file deletes).
  - `save()` everywhere.
  - `web.rs:203`: `launch::open_path` calls `spawn()`, and the `Child` is never waited on (`launch.rs:53,59,64`),
    so each Open or Open folder leaves a zombie on Linux and macOS for the life of the daemon. `open_url` has the
    same problem (`launch.rs:34`).
- **Fix:** run handler bodies that touch the filesystem through `tokio::task::spawn_blocking`. Reap children with a
  detached thread that calls `child.wait()`, or `tokio::process::Command` plus `tokio::spawn(child.wait())`.
- **Test:** call `/folder` 100 times and assert that `ps` shows no `<defunct>` children of the daemon.

### M6. `Manager::get(id)` clones the whole list to find one item
- **Where:** `lib.rs:887-889`. It is used by Open/folder (`web.rs:202`), `refresh_address` (`lib.rs:1413`) and the
  CLI `get` loop every 400 ms (`main.rs:454`).
- **Fix:** look up `db.items.get(&id)`, then overlay the live progress and segments for that one id.
- **Test:** a unit test that `get` returns live `downloaded`. A benchmark with 10k items.

### M7. Starting the service: no single-instance lock, and losing the HTTP port is not fatal
- **Where:**
  - `main.rs:378` opens the `Manager` (and its `downloads.json`) before any port is bound, and spawns
    `resume_seeding` (`main.rs:381-385`).
  - A second daemon only exits when `ws::listen` fails (`main.rs:444-447`), so two managers briefly share the file
    **(uncertain whether the second one saves before it exits)**.
  - If port 61002 is held by another program, `web::serve` fails and only logs it (`main.rs:437-441`). The daemon
    keeps running with no API. The GUI and host see the port as "up" (`daemon_up` is a TCP connect), and the GUI
    loops on "waiting for the download service…".
  - Ctrl-C returns from `main` without a graceful shutdown. The token file is only removed when `axum::serve`
    returns (`web.rs:892-893`), so it stays behind. SIGTERM (logout, systemd) is not handled at all.
- **Fix:** take an advisory lock file (`fs2`/`fd-lock`) in the data folder first, then bind both ports, then open the
  `Manager`. Treat a 61002 bind failure as fatal, with a clear message. Handle SIGTERM and Ctrl-C by stopping
  transfers, saving, and removing the token. Have `daemon_up` check `/ext/ping`'s `"app":"reIDM"` instead of a
  bare TCP connect (`api.rs:74-77`, `reidm-host/src/main.rs:67-70`).
- **Test:** start two daemons in a test; assert the second exits non-zero before touching `downloads.json`.
  Occupy 61002 with a dummy listener and assert the daemon exits with an error.

### M8. Destructive actions with no confirmation
- **Where:**
  - "Restart from the beginning" (`app.rs:1691`, `app.rs:2517`, toolbar menu `1050`) discards all progress, and
    for a **finished** download it re-downloads over the existing file. For yt-dlp downloads it **deletes the
    finished video at once** (`lib.rs:1170-1177`).
  - The "Delete Completed" toolbar button (`app.rs:1164-1166`) removes thousands of history rows with no
    confirmation and no undo.
  - The Delete dialog puts "Remove and delete the files" next to "Remove from list only" with equal weight
    (`app.rs:2373-2380`).
- **Fix:** confirm Restart, saying how much progress is lost and, for finished items, "replaces the file on disk".
  Confirm Delete Completed with a count. Style the destructive button red and make "Remove from list" the default
  (Enter); Esc cancels.
- **Test:** GUI unit tests that these commands open a confirmation state instead of calling the API.

### M9. Long secondary lists are not virtualized, and dialogs recompute their content every frame
- **Where:**
  - Grabber results (`app.rs:2296-2317`).
  - The torrent file picker (`torrent.rs:136-144`) and Properties' torrent files (`app.rs:2480-2490`).
  - Playlist rows with two ComboBoxes each (`playlist.rs:270-290`).
  - Rebuilt every frame: the grabber's `chosen` URL vector (`app.rs:2323`); the Add dialog's
    `parse_url_list(&self.add.urls)` (`app.rs:1833`), even when 10k lines are pasted; and the batch dialog's
    `expand_pattern` of up to 10,000 URLs (`app.rs:1918`).
- **Fix:** use `ScrollArea::show_rows` / `TableBuilder::body.rows`. Cache the parsed and expanded results, keyed by
  a hash of the input text and parameters.
- **Test:** headless frame-time tests with 20k grabber results, a 50k-file torrent, and a 5k-video playlist.

### M10. The batch pattern overflows: `0` to `18446744073709551615` hangs the GUI or CLI and runs out of memory
- **Where:** `reidm-core/src/lists.rs:151-156`. `hi - lo + 1` wraps to 0 in release builds, so it passes the
  `> MAX_PATTERN_ITEMS` check, and `(lo..=hi).collect()` never ends. The GUI evaluates this **every frame while the
  user types** (`app.rs:1902-1918`), and the CLI evaluates it at `main.rs:537`. Debug builds panic instead. The CLI
  `--width` is unbounded too (`main.rs:88-89`), so `format!("{i:0width$}")` with a huge width allocates without
  limit.
- **Fix:** `hi.checked_sub(lo).and_then(|d| d.checked_add(1))`, and check against the cap before allocating. Cap
  the width at 20.
- **Test:** `expand_pattern("a*", Numbers{from:0,to:u64::MAX,width:0})` returns `Err` immediately.

### M11. Accessibility (details in section 5)
- **Where:** `crates/reidm-gui/Cargo.toml:21` disables eframe's default `accesskit` feature, so the app has **no
  screen-reader support**. Most of the UI is painted text with no widget info: table rows (`app.rs:1514`), the
  sidebar (`app.rs:1353-1366`), toolbar captions (`app.rs:2955`) and sort headers (`app.rs:1443`). There is no
  keyboard navigation of rows (arrows, Space, Home/End, Shift+arrows). Escape doesn't close dialogs. The fonts are
  hard-coded at 10–11.5 pt. Several colour pairs fail WCAG AA (4.5:1).
- **Fix and test:** see section 5.

### M12. GUI threads: one OS thread per action
- **Where:** `api.rs:302-315` (`action` spawns a thread per id). `stop_all` (`api.rs:514-518`) and "Start all"
  (`app.rs:1037-1042`) loop over every item, so Start all on 5,000 paused items spawns 5,000 threads and sends
  5,000 requests, each triggering a full `save()` (C2).
- **Fix:** the bulk endpoint from H5, plus the single command worker.
- **Test:** count spawned threads for Start all with 5k items; it should be 1.

---

## Low

### L1. API token comparison is not constant-time
`web.rs:92`. The practical risk over loopback is small, but use `subtle::ConstantTimeEq` or compare fixed-length
digests. *Test:* a unit test of `guard` with the correct token, a wrong token, and a wrong token of a different
length.

### L2. The native host routes every caller that isn't reIDM's extension into relay mode, and doesn't check reply sizes
`reidm-host/src/main.rs:150-155`. Only reIDM's exact origin goes to `launcher()`; anything else relays. The browser
manifests restrict callers, so this is defence in depth: compare with the two `CHROME_ORIGINS` and Firefox's
manifest path, and exit otherwise. Host→browser frames over 1 MiB make Chrome kill the host (`write_frame`,
`main.rs:105-110`): check the size and drop the frame.

### L3. `bundled_extension_dir` checks a development path in release builds
`install.rs:311` (`bin/../../extension`). Installed under `C:\Program Files\reIDM\`, that resolves to `C:\extension`,
which any user can create. It is used only if the bundled copy is missing, but it would then be staged as "reIDM's
extension" for Load unpacked. Gate it with `#[cfg(debug_assertions)]`. Also, `uninstall` on Windows leaves the
`*.json` host manifests in the data folder (`install.rs:270-282`).

### L4. The WebSocket and HTTP origin allow-lists differ
`ws.rs:27` allows `extension://` but not `safari-web-extension://`; `web.rs:344-346` is the reverse. Share one
function.

### L5. The GUI blocks for up to 15 s before showing a window, and a startup failure is invisible
`reidm-gui/src/main.rs:46-49` → `api::ensure_daemon` (`api.rs:125-130`). On Windows the `windows_subsystem` attribute
hides the `eprintln!`. Show the window immediately with a "Starting the download service…" state, and a clear error
panel with "Retry" and "Show log".

### L6. Speed limiter "turn on" with no rate set
`app.rs:1075-1078` enables the limiter when `speed_limit == 0`. The status bar then shows "Speed limit: off" in warning
colour (`app.rs:1332-1333`). Open the limiter dialog instead when no rate is set.

### L7. Sorting quirks
"Time Left" sorts by rate, ascending (`app.rs:1473`); "Description" sorts by name (`app.rs:1476`); Status sorts by the
label alphabetically (`app.rs:354`). Give each its own sort key: ETA (remaining/rate), URL, and a status order of
Running > Queued > Paused > Failed > Done.

### L8. CLI correctness
- `reidm list` reads `downloads.json` instead of the API (`main.rs:697-725`). Its values are up to 3 s stale, and the
  ids come out **lexicographically** (1, 10, 11, 2…) because serde_json's `Map` is a `BTreeMap<String>`
  (`preserve_order` is off; verified with `cargo tree`). Use `GET /api/downloads`, fall back to the file, and sort
  ids numerically.
- `reidm pause/resume/rm` go through the WebSocket, ignore replies, and never fail for unknown ids
  (`main.rs:726-728`); see H8.
- `reidm get` uses a fresh `reidm-get-<pid>` data folder in temp (`main.rs:450`). It is never cleaned up, and an
  interrupted `get` can't resume.
- `parse_rate("8Mbps")` gives 8 MiB/s rather than megabits (`main.rs:308-322`), and `"1.5MiB"` fails. Document the
  units or support `ibps` and bits.
- `reidm export` falls back to reading the file on *any* error, a 401 included (`main.rs:574-577`).

### L9. `resume()` doesn't save
`lib.rs:1135-1150`. Fine if `launch()` saves soon after; unconfirmed **(uncertain)**.

### L10. Closing the window leaves the service running, and nothing says so
There is no tray icon, no "Quit reIDM completely", and no first-close notice. See section 5.

---

## F-PERF: proposed design for large lists

1. **Revision counter in the daemon.** Add `rev: AtomicU64` to `Manager`. Bump it on every DB mutation and on every
   progress checkpoint, or have a 250 ms ticker bump it while anything is live. Each item records `rev_changed`.
2. **Conditional GET.** `GET /api/downloads` returns `ETag: "<rev>"`. A request with `If-None-Match` and the same
   rev gets `304` with no body. An idle list then costs nothing.
3. **Deltas.** `GET /api/downloads?since=<rev>` returns `{rev, full:false, upsert:[rows changed after since],
   removed:[ids]}`. Keep a small ring buffer of removed ids, and return `full:true` if `since` is too old.
4. **Light rows.** The list row should omit `segments`, `torrent.files`, `etag`, `last_modified` and
   `request.user_agent`. Add `segments_active: u8` for the "(8)" connection count. Serve the full item from
   `GET /api/downloads/{id}`, fetched only by the Properties window. This shrinks a row from about 1.4 KB to
   about 300 B.
5. **Progress fast path (optional).** `GET /api/progress` returns `[{id, downloaded, state, conns}]` for live items
   only. Or replace polling with Server-Sent Events (`/api/events`) pushing upserts and removals; axum supports SSE
   natively.
6. **Rates on the daemon.** The daemon already meters bytes, so it can report `rate` per item and in total. The GUI
   then needs no per-item history maps.
7. **GUI.** Keep `Arc<Snapshot>`, an `id → index` map, and a `ViewModel` cache (H2). Repaint on change (H3), poll
   adaptively, and keep the table on `body.rows` (it is already virtualized: `app.rs:1479`).
8. **Persistence.** Use a debounced background writer with compact JSON, serialized off the lock, with fsync and a
   `.bak` (C2, H9). For a 10k+ history, consider an embedded store (SQLite or redb) with per-item upserts, so a
   checkpoint writes about 1 KB instead of 19 MB. With four downloads running today, about 4 × 19 MB is rewritten
   every 3 s (`lib.rs:1674-1681`, `1707-1713`).

Expected effect at 10k items while idle: from about 45 MB/s of JSON and 5 fps of full UI work to about 0 B/s
(304s) and roughly 1 fps or none.

---

## 5. UX simplification, onboarding, accessibility

### 5.1 Everything the GUI shows today
- **Menu bar:**
  - **Tasks:** Add URL…, Add torrent…, Add YouTube playlist…, Add batch download…, Import (URL list or IDM .ef2)…,
    Export ▸ (.txt / .ef2), Site Grabber…, Exit.
  - **File:** Open download, Open containing folder, Properties…, Delete, Delete completed.
  - **Downloads:** Start, Stop, Start all, Stop all, Restart, Refresh download address…, Start queue, Stop queue,
    Scheduler…, Speed limiter: turn on/off, Speed limiter settings…, Options….
  - **View:** Dark theme, Show categories, All Downloads, Unfinished, Finished, Scheduled queue,
    Browser integration….
  - **Help:** Browser integration…, Check for updates, About reIDM.
  - Right side: Service running / unavailable.
- **Toolbar (12 buttons):** Add URL, Resume, Stop, Stop All, Delete, Delete Completed, Options, Scheduler,
  Start Queue, Stop Queue, Site Grabber, Speed Limiter, plus Search.
- **Sidebar:** Categories › All Downloads › Video, Music, Images, Documents, Compressed, Programs, Other; Unfinished;
  Finished; QUEUES › Scheduled queue; GRABBER › Site grabber….
- **Table columns:** select box, File Name (+progress, %), Size, Status, Time Left, Transfer Rate, Last Try,
  Description (the URL, or the error).
- **Status bar:** counts, Browser: connected / not connected (opens integration), "Port 1001 busy" (Windows), Total
  rate, Speed limit (opens limiter), update status.
- **Row context menu:** Open file, Open containing folder, Stop/Resume, Seed again/Stop seeding, Restart from the
  beginning, Refresh download address…, Add to / Remove from the scheduled queue, Copy address, Properties…,
  Remove from list, Remove and delete the file.
- **Options:**
  - *General:* Default download folder; Connections per download (1–32); Downloads at the same time (1–16);
    [Speed limiter…] [Scheduler…]; Window: Dark theme, Show a window when a download completes, Fill Add URL with
    links from the clipboard; Browser status + [Set up…]; Updates [Check now].
  - *Folders:* Save each download in the folder for its type; per-category folder for 7 categories (Browse /
    Default); Temporary folder.
  - *Torrents:* When complete (don't seed / 0.5× / 1× / 2× / 5× / forever); Upload limit KB/s; Listening port; DHT.
- **Dialogs:**
  - Add: addresses, Save as, Save to, Import list…, Batch with a pattern…, Start / Download later / Cancel.
  - Batch: pattern, from, to, wildcard size.
  - Scheduler: enable, Start at, Stop at, days, Start now / Stop now.
  - Speed limiter: on, KB/s, presets.
  - Properties: details, segment map, per-download limit, Pause / Resume / Open / Open folder / Refresh address… /
    Restart.
  - Refresh address: new URL or capture from browser.
  - Site Grabber: URL, Follow links depth, Same host only, Ask for sizes, Types, Include, Exclude, Export / Import,
    Download / Download later.
  - Add torrent: link or file, file picker, Download / Download later.
  - Playlist: per-video resolution and codec or audio, apply to selected, own folder.
  - Browser integration, Download complete (with "Don't show again"), About, Delete confirmation.

### 5.2 Recommended default view vs "Advanced"
Add **Options › "Show advanced features"**, off by default and stored in `gui.json` (Prefs), which shows the rest.

| Simple (default) | Behind "Advanced" |
|---|---|
| Toolbar: **Add**, **Pause**, **Resume**, **Remove**, **Settings**, Search | Delete Completed, Start/Stop Queue, Scheduler, Site Grabber, Speed Limiter buttons (keep them in menus) |
| Sidebar: All, Downloading, Finished, plus categories collapsed by default | Scheduled queue node, Grabber node |
| Settings › General: **Download folder**, **Downloads at the same time**, **Limit download speed** (an inline toggle and value instead of a separate dialog), **Theme: System/Light/Dark**, **Notify when done**, **Browser extension status + Set up** | Connections per download; clipboard prefill; per-category folders; temporary folder; all Torrent settings (seeding policy, upload limit, port, DHT); scheduler; check-for-updates button (keep automatic checks) |
| Context menu: Open, Open folder, Pause/Resume, Copy address, Properties, Remove… | Restart from the beginning, Refresh download address, Add to/Remove from queue, Seed again/Stop seeding |
| Add dialog: addresses, Save to, Start / Later | Save as, Import list, Batch pattern |
| Tasks menu: Add URL, Add torrent, Add video playlist | Batch download, Import/Export .ef2, Site Grabber |
| Properties: status, size, speed, Pause/Resume, Open folder | Referer, UA, segment map, per-download limit, info hash |

Default "Sort into category folders" to on (it already is), but hide the per-category grid until Advanced is on.

### 5.3 Confusing wording
- **Stop / Pause / Resume / Start.** The toolbar says Resume and "Stop", the status says "Paused", the Downloads
  menu says "Start" and "Stop", and the context menu says "Stop" and "Resume". Use **Pause** and **Resume**
  everywhere ("Stop" suggests cancelling).
- **"Queued" vs "Scheduled queue" vs "Download later" vs "Start Queue".** The status *Queued* means "waiting for a
  free slot", which has nothing to do with the *scheduled queue*. Rename the status to **Waiting** and the queue to
  **Scheduled (Later)**, and drop the "Start Queue / Stop Queue" jargon in favour of "Start scheduled now" in the
  Scheduler.
- **"Delete" opens "Remove from list".** Call the action **Remove…**; reserve "Delete" for deleting files.
- **"Description"** shows the address; rename it to **Address**. **"Last Try"**: say **Last active**.
- **"Refresh download address"**: say **Update link (expired link)**.
- **"Wildcard size"**: say **Zero-pad to N digits**.
- **"Port 1001 busy"** means nothing to most users. Show it only in Advanced or the integration dialog.
- Torrents: "Seed until it has shared 1× its size", "DHT", "Listening port 0 picks any free port". Group these
  under **Advanced › Torrents** with a one-line explanation, and use "Sharing" instead of "Seeding" in simple mode.
- **"Service running / Service unavailable"**: say **Ready** or **reIDM's background service isn't running
  [Start]**, with an action.
- **Options vs Settings**: "Settings" is the more common term on Linux and macOS; pick one.

### 5.4 First run and onboarding gaps
- No first-run flow: the window opens on an empty table and a yellow "Browser: extension not connected". Add a
  three-step welcome: **(1)** confirm the download folder, **(2)** connect the browser (open the integration
  panel, with "Open the folder", "Copy path", and a live check that turns green when the extension pings),
  **(3)** an optional test download. Show it again from Help.
- The extension needs Developer mode and "Load unpacked". Say clearly that Chrome may warn about developer-mode
  extensions, and link a store listing if one exists. Auto-open the integration window the first time the
  extension has never been seen.
- The GUI blocks for up to 15 s before a window appears (L5), and a startup failure is invisible.
- Closing the window keeps the service running, with no tray icon and no notice (L10). On the first close, show
  "reIDM keeps downloading in the background. [Keep running] [Quit completely]". Add **File › Quit reIDM (stop
  downloads)**, which calls a new `POST /api/shutdown`.
- Errors surface only as 5-second toasts (`app.rs:258`). Keep an error list or log panel, and put failed rows
  under a "Failed" filter.
- No "follow system theme": the default is light unless `--dark` is given.

### 5.5 Accessibility
- **Screen readers:** enable eframe's `accesskit` feature (`Cargo.toml:21`, add `"accesskit"`). Replace painted
  text in rows, tree items and toolbar buttons with widgets, or attach `resp.widget_info(||
  WidgetInfo::labeled(WidgetType::Button, true, label))` to each `allocate_*` response.
- **Keyboard:**
  - Up/Down move the selection, Shift+arrows extend it, Space toggles, Home/End jump, Enter opens
    (already there), Shift+F10 or the Menu key opens the context menu, and F2 or Alt+Enter opens Properties.
  - Esc closes the topmost dialog. Tab order inside dialogs, with a default button on Enter.
  - The toolbar uses `Sense::click`, which is focusable, but has no visible focus ring because the custom paint
    ignores `resp.has_focus()`. Draw one.
- **Contrast:** measured with the WCAG formula.
  - Light `warn` #B06A00 on #FAFAFA: **4.10:1**. It is used for the 12 px "Paused", "Speed limit" and "Port 1001
    busy" text.
  - Dark `accent` #2A84E0 on #2D2D30: **3.58:1**. It is used for the rate text and sort headers.
  - About dialog grey(140) on light: **3.22:1** (`app.rs:2180,2191`). Its blue on light is 3.67:1, and its orange
    on dark is 3.58:1. The hard-coded colours there also ignore the palette.
  - Fix: light warn → #8A5300, dark accent → #5AA2F0 (or lighten panel_alt), and use palette colours in About.
- **Font sizes:** text at 10–11.5 pt (`app.rs:1205,1258,1524,1612,2955`, `api`-driven labels) is too small at
  100% scaling on 1366×768 laptops. Set a minimum of 12 pt, and add **Settings › Text size** (100–200%) mapped
  to `ctx.set_zoom_factor`. egui's Ctrl +/- zoom already works but nothing tells the user about it.
- **DPI:** egui follows the OS `pixels_per_point`. The fixed widths (`add_sized(520…)`, 560 px windows, 46 px
  toolbar) are in points and scale, but at 150–200% on a 1366×768 screen, dialogs of 560 pt plus the 720×420
  minimum window can overflow. Make dialogs resizable and use `ui.available_width()`.
- **Colour alone:** status uses colour plus a text label (good). The segment map and progress bar rely on colour
  only; add a % label (the table has one, Properties doesn't).

---

## 6. Code quality and architecture

### 6.1 Split `app.rs` (2,958 lines, one `impl App` spread over five blocks)
Proposed module tree, mechanical and with no behaviour change:
```
reidm-gui/src/
  app/mod.rs          App struct, eframe::App::ui (frame loop only), poll_* glue
  app/state.rs        AddState, BatchState, SchedEdit, LimiterEdit, PropsState, RefreshState, GrabState, UpdateOffer, Toast
  app/commands.rs     Command enum + worker thread + results (replaces sync api calls; H5)
  app/view_model.rs   cached visible rows/counts/selection set (H2)
  ui/menu.rs  ui/toolbar.rs  ui/sidebar.rs  ui/table.rs  ui/status_bar.rs  ui/context_menu.rs
  ui/widgets.rs       tool(), tree_row(), segment_map(), window(), label()
  dialogs/{add,batch,options/{general,folders,torrents},scheduler,limiter,props,refresh,grabber,integration,complete,about,confirm}.rs
  (existing) api.rs model.rs playlist.rs torrent.rs prefs.rs theme.rs icons.rs
```
Each dialog becomes `struct XDialog { … } impl XDialog { fn show(&mut self, ctx, &ViewModel, &Commands) -> Option<Outcome> }`,
the pattern `playlist.rs` and `torrent.rs` already use.

### 6.2 Duplication
- Spawning the daemon is implemented three times with different flags: `api.rs:96-132`, `launch.rs:69-79` and
  `reidm-host/src/main.rs:37-65` (only the host uses `CREATE_BREAKAWAY_FROM_JOB` and a new process group on
  Windows; only the GUI sets `process_group(0)` on Unix). Move it to one function in a small shared crate
  (`reidm-proto` or a new `reidm-ipc`).
- `UI_PORT` is defined three times (`web.rs:16`, `api.rs:14`, `reidm-host:25`), as are `61001` and the extension ID
  (`install.rs:25`, `reidm-host:30`). Reading the token is written twice (`main.rs:240`, `api.rs:81`), and so is
  opening a URL (`launch.rs:15` vs `api.rs:522`).
- Each handler builds a `Request` from its body in its own way: `web.rs:174-183`, `464-473`, `527-536`, `584-604`,
  and `session.rs:219-232`. Add one `impl From<…> for Request` with the trimming and empty-filtering in one place.
  They disagree today: `add` keeps an empty `referer`/`cookie`, `ext_add` drops them.
- The `guard(&app,&h,true)` boilerplate is repeated in 17 handlers. Use an axum extractor (`struct Authed;
  impl FromRequestParts`) or a `middleware::from_fn` on an `/api` sub-router, plus a separate `Host` and Origin
  layer for `/ext`.
- The GUI `model.rs` hand-mirrors core types (`Item`, `Settings`, `Schedule`), and JSON shapes are built ad hoc with
  `serde_json::json!` (`web.rs:405-416`, `753-763`, `772-784`). Define typed DTOs once, shared by daemon, GUI and
  CLI, to prevent drift (for example, the GUI's `Request` has no `filename`/`dir`).

### 6.3 Error handling
- The daemon's `save()`/`save_settings()` swallow write errors (`lib.rs:828-830,837-840`), and so do the GUI's
  `prefs.save()` (`prefs.rs:43`) and `cancel_refresh` (`api.rs:380`). Log the error and surface it in
  `/api/status`.
- There are many `.lock().unwrap()` calls on `std::sync::Mutex` in async code (`web.rs:399,746`, all over
  `lib.rs`). One panic poisons the lock, and then every request panics. Use `parking_lot::Mutex`, which has no
  poisoning, or handle `PoisonError`.
- Errors come back as plain text on `/api` and as JSON on `/ext`. Use JSON `{error, code}` everywhere so the GUI can
  match on codes rather than strings. The GUI depends on raw text, as in "Finished downloads can't…"
  (`app.rs:451`).
- The GUI `Api` swallows connection errors in `remove` and `action` with toasts only. Failed bulk operations give no
  per-item result.

### 6.4 Tests
There are no tests for `web.rs` handlers, `api.rs` or `app.rs`. Add axum `tower::ServiceExt::oneshot` tests for
`guard` (Host, Origin, token), `/ext` origin rules, body limits, and batch outcomes. Add GUI view-model unit tests,
which become possible once 6.1 separates state from drawing. Add a load test fixture with 10k and 50k items for C1,
C2, H1 and H2.
