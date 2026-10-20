# Development log

Why reIDM is the way it is: each significant change, the reasoning behind it, and what was
considered and rejected. Newest first. Add an entry for every change that involves a decision;
link the PR. Keep entries short — the code and the commit say *what*, this says *why*.

Format: `## YYYY-MM-DD — title (version, PR)`, then **Context**, **Decision**, **Alternatives**,
**Evidence/tests**, and **Follow-ups** where useful.

---

## 2026-10-20 — Engine: durable progress, a crash-safe list, and the HTTP engine's failure cases (0.8.0)

**Context.** The October engine review (reviews/2026-10/engine.md C1, H1–H7, M1–M4, M8, M9, M11–M13,
L1, L2, L4–L6, L11, L12, L14) and the app review's C2, H7, H9, M4, M6, M10 and L9. The worst of them
(C1): tokio's `File::write_all` returns before the write happens, so `segment.done` ran ahead of the
disk, and the "is the data on disk?" length check could never fire after `set_len`. A full disk or
a crash could leave a resumed download "complete" with zeros inside it. Next worst: `downloads.json`
was rewritten whole (pretty-printed) under the list's lock on a runtime thread, after every add,
with no fsync, and a damaged file either stopped the service or was silently treated as empty.

**Decisions.**
- *Count only what is written; record only what is durable.* Each connection collects 256 KiB (or
  1 s, whichever comes first) and writes it with a positional write (`write_all_at` /
  `seek_write`) on the blocking pool; `done` advances only after the write returns `Ok`, and a
  failed write releases its claim. A checkpoint reads the ranges, `sync_data`s the part file, and
  only then copies them into the list, so every counted byte was written before a flush that
  succeeded. Batching also fixes M8: one blocking-pool hop per 256 KiB instead of per 8–16 KiB
  network read, one plan lock per read instead of two, and one idle timer reset instead of a new
  timeout per read. The length check stays, for a part file deleted or truncated between runs.
  Memory: 256 KiB × connections (2 MiB for a default 8-connection download).
- *The list: copy-on-write items, a guard that notices changes, one saver thread.* `items` holds
  `Shared<Item>` (an `Arc` with `make_mut`), so a save copies pointers under the lock and
  serialises (compact JSON) and writes outside it. The lock returns a `DbGuard` whose `DerefMut`
  marks it touched; on drop it bumps the revision and asks for a save. That keeps
  `self.0.db.lock().unwrap().items.get_mut(&id)` working unchanged in media.rs and torrent.rs, and
  means no change can be forgotten by code that doesn't call `save()`. Saves are debounced to 2 s,
  or immediate (`save_now`) for adds, removals and status changes; the browser hand-off
  (`add_checked`) waits until the new download is on disk before answering, since the browser
  drops its own copy when it hears back.
- *Crash-safe files.* Write a temporary file with `create_new` (mode 0600 on Unix: it holds
  cookies), `sync_all`, hard-link the current file to `.bak` (copy where links don't work), rename
  over, fsync the folder (Unix). On load: a parse failure (including an empty file) is moved aside
  as `.corrupt-<time>` and the `.bak` used, else start empty with a warning; a missing main file
  with a `.bak` restores the backup; a file that exists but can't be read uses the `.bak` or
  refuses to start with an explanation, never an empty list saved over it. Settings are the same.
  Problems are kept for `Manager::warnings()`.
- *An address index.* `Db` keeps `url_key → ids` (not serialised, rebuilt on load); addresses change
  only through `Db::set_url`. `add_many_with` prepares every request first, creates each folder
  once, then places them all under one lock with one claimed-path set, and saves once.
- *`revision()`.* An `AtomicU64` bumped by the guard, by every progress change (`Progress::store`
  only when the value changes), and when a connection takes or lets go of a range (the segment map
  and connection count show that). Idle, it does not move.
- *Cancellation that wakes.* `Cancel` = flag + `tokio::sync::Notify`. `send()`, the body read, the
  limiter wait and retry naps all `select!` on it, so Pause works on a hung connection. Requests time
  out after 30 s without an answer; the client has `read_timeout(60s)`, TCP keepalive and a 30 s
  pool idle timeout. `restart`/`refresh_address` refuse ("still stopping; try again") instead of
  carrying on while the old run is live.
- *Retries.* A failure after progress resets the connection's retry count and carries on at once
  (resumable downloads only — a non-resumable retry starts from 0, so its retries must stay
  bounded). Every 30 s a run adds a connection back while it has fewer than configured and work
  is left.
- *Never replace at finalize.* `renameat2(RENAME_NOREPLACE)` / `renamex_np(RENAME_EXCL)` /
  `MoveFileExW` without `REPLACE_EXISTING`, with a check-then-rename fallback where the file system
  can't refuse. On `AlreadyExists` the next free name is chosen under the list lock and recorded.
  Across drives: copy beside the destination under `.<id>.reidm-move`, fsync, then the same
  rename. `Item.replace` (set by Redownload of a finished item) is the only way to replace. The
  server's name is chosen and recorded under a single lock.
- *Preallocation.* Linux: `fallocate` (reserves the space; a disk too small fails at once with a
  clear message), falling back to `set_len` where unsupported. Windows: `FSCTL_SET_SPARSE` then
  `set_len`, so NTFS doesn't zero-fill up to the first write of the last range. macOS: `set_len`
  (APFS is sparse). Raw `extern "system"` declarations for the two Win32 calls rather than a new
  dependency.
- *Mark-of-the-Web.* After the move: `:Zone.Identifier` with `ZoneId=3`, `ReferrerUrl`, `HostUrl`
  (credentials stripped) on Windows; `com.apple.quarantine` = `0081;<hex time>;reIDM;` via
  `setxattr` on macOS. Failures (FAT, network shares) are logged at debug and ignored.
- *Smaller ones.* Pause on a queued download pauses it (M1); Resume during a stop sets `requeue`
  (M2); re-adding a finished link whose file is gone downloads it again (M3) and re-adding takes
  fresh cookie/referer/user agent when not running (M4); Remove of a running download defers file
  deletion to the run's exit, including a file renamed into place meanwhile (app M4, L5); the part
  file's path is stored in the item (M9); a 206 needs `Content-Range` with the right start, 416 on
  resume and a different `Content-Length` without ranges are `ChangedOnServer`, and without an ETag
  a different `Last-Modified` is too (M11); `get` looks up one item (app M6); names are cut to 200
  bytes keeping the extension (L1) and bidi controls replaced (L12); query values are re-encoded and
  sorted by name only in `url_key` (L2); refresh before any bytes re-plans ranges for the new
  server (L4); a limiter wait cut short refunds its time (L6); the batch pattern count is checked
  and width capped at 20 (app M10); when the clocks go back, the scheduler keeps its mark for a step
  back under 2 h (L14).
- *Global limit (M5, the core half).* `Limiter::charge(n)` counts bytes paced elsewhere against the
  bucket without waiting, debt capped at 1 s of the rate. Media and torrents use it (the merge
  replaced their `take(n, &AtomicBool::new(true))`, which the refund would have turned into a
  no-op).

**Alternatives.**
- A writer task per download fed by a channel: one more queue and its back-pressure, for no gain
  over positional writes from each connection.
- `O_DSYNC`/fsync per write: correct but throttles to the disk's flush rate; one flush per 3 s
  checkpoint costs the same durability at a fraction.
- SQLite/redb, or a progress file per download (the review's long-term option): a bigger change
  and a new format; the copy-on-write snapshot removes the lock contention, and with compact JSON a
  10,000-item list serialises in tens of milliseconds on the saver thread.
- Serialising under the lock (only compact, off-runtime): still stalls the API for the
  serialisation. Rejected for the snapshot.
- `posix_fallocate`: glibc emulates it by writing zeros where unsupported — exactly what H7 is about.
- Clearing cookies and segments of finished items (M13): the GUI's segment map reads the
  segments, and Redownload needs the cookie. Not done.
- `CancellationToken` from tokio-util: not a dependency; `Notify` does the same in ten lines.
- Keeping `remove` synchronous: the daemon calls it from many places; the tombstone in `Live` gets
  the same ordering (no deletion while writers run) without an API change.

**Evidence/tests.** 77 unit tests and 31 + 15 + 3 + 1 integration tests in reidm-core; five
consecutive runs of lib/engine/resume/compat/scale clean; clippy `-D warnings` clean for Linux,
`x86_64-pc-windows-gnu` and `aarch64-apple-darwin`.
- C1: `a_failed_write_never_leaves_counted_bytes_missing` (a test-only fault injector fails writes
  like a full disk, with a short write at the boundary; every counted byte must match the source,
  the saved list must agree, and the resumed file must be identical). Mutation-checked: counting a
  failed write makes it fail ("range 1000000..1262144 is counted but not on disk").
- H1/H9: store unit tests (empty file → backup, truncated without backup → empty + warning,
  unreadable → error or backup, missing → backup, 0600) and `a_damaged_list_is_recovered_…`.
- 0.7.0: `a_list_from_0_7_0_loads_and_its_downloads_carry_on` writes a 0.7.0-shaped
  `downloads.json` (pretty, every 0.7.0 field) and part file, resumes, checks the bytes and that it
  continued from the saved offsets.
- C2/H6, measured (debug build, `tests/scale.rs`, `measure_batch_add`):

  | | before | after |
  |---|---|---|
  | `add_many` 2,000 links into an empty list | 51.1 s | 0.115 s |
  | 2,000 more into 2,000 | 153.7 s | 0.090 s |
  | 10,000 into empty / 10,000 more into 10,000 | (not run: minutes) | 0.38 s / 0.43 s |
  | `list()` of 4,000 items | 2.6 ms | 2.9 ms |

  `adding_thousands_of_links_is_quick` keeps 5,000 under 20 s (debug, generous for CI).
- The others each have an integration test in `tests/engine.rs` or `tests/compat.rs` (pause on a
  server that never answers, connections that keep breaking, a file appearing at the destination,
  two same-named downloads, pause queued, resume while stopping, deleted file re-added, fresh
  cookies, remove while running, temporary folder moved, no `Content-Range`, 416, length without
  ranges, `Last-Modified`, refresh to a no-range server, revision, 0600, move across drives via
  `/dev/shm` where it is another file system), plus unit tests for names, `url_key`, the limiter,
  the no-replace rename, preallocation, the Zone.Identifier/quarantine values and DST. Only the C1
  test was mutation-checked against the old behaviour; the H3/H4 tests are written so the 0.7.0
  code could not pass them (a 15-minute hang; a dozen breaks against five retries) but were not run
  against it.
- Not testable here: the Windows sparse file and Zone.Identifier stream, and the macOS xattr, are
  compile-checked by clippy for those targets only.

**Follow-ups (outside reidm-core).**
- The daemon should call `Manager::flush()` on Ctrl-C/SIGTERM before exiting (L8); without it the
  last ≤2 s of debounced changes (progress, notes) can be lost — never adds, removals or status
  changes, which are written at once, and never correctness, since progress is checkpointed safely.
- The HTTP API can use `Manager::revision()` for ETag/304, and `Manager::warnings()` for a banner
  in `/api/status` (app H9).
- `update_settings` now returns an error if the settings could not be saved (they still apply in
  memory); the API should show it.
- Media and torrent outputs could call `disk::mark_from_internet` (pub(crate)) per file (app H7).
- Any mutable access through the list's lock counts as a change (revision bump, save within 2 s).
  Torrent `record()` and `set_note` took `get_mut` twice a second and compared afterwards, so an
  idle seed rewrote the list every 2 s; fixed at merge (look first, write only on a change; test
  `repeating_a_note_is_not_a_change`). **Rule for new code: read with `get`, take `get_mut` only
  to change something.**

## 2026-10-20 — Torrents, videos and tools after the review (0.8.0)

**Context.** The engine review's torrent/video/tool findings (reviews/2026-10/engine.md M5–M7,
M10, L3, L9, L10, L13), plus one it missed. Checking librqbit 9.0.1 showed that with an explicit
`output_folder` it does **not** add the torrent's name as a subfolder (`session.rs:1286`), so 0.7.0
wrote a multi-file torrent's files loose into the category folder, over any same-named file, while
the list pointed at a folder that didn't exist.
**Decisions.**
- *Placement before creation.* Describe the torrent first (kept `.torrent`, a capped fetch, or peers
  for a magnet), choose a free place under the list lock, record it (`TorrentStatus.folder`) and save
  — only then let librqbit create files, with `overwrite: false` unless reIDM is carrying on its own
  files. Rejected: renaming single files (needs a custom librqbit storage); refusing outright
  ("explain, don't just fail" when a safe place exists). 0.7.0 torrents keep their layout.
- *Videos with holes.* yt-dlp's default skips a piece it can't fetch and reports success; reIDM now
  passes `--abort-on-unavailable-fragments` and re-runs yt-dlp (re-reading the page, so fresh
  addresses) up to 20 times while each run makes progress.
- *Speed limit.* yt-dlp gets `--limit-rate` = its share of the global limit, restarted (≤ every 5 s)
  when the limit changes; the torrent session's limit is its share, updated every 500 ms; both are
  charged to the shared allowance. Known gap: a running video keeps the share it started with when
  other downloads start or stop — restarting on every queue change would re-extract constantly.
- *Tools.* Streamed to disk with SHA-256 computed on the way; one lock per tool, taken after a
  lock-free "installed" check; 30 s answer/stall timeouts; failures remembered in memory (update
  check: 1 h, missing tool: 2 min) — a restart is a fair time to try again.
- *Bodies.* `.torrent` capped at 10 MiB, grabber pages 8 MiB, enforced while reading.
**Evidence.** 10 new integration tests (tests/torrent.rs, tests/media.rs) and 11 unit tests; the
lib, torrent and media tests passed 8 runs in a row. One honest gap: the restart-vs-delete test also
passes without the fix on this machine (the race window is small), so it guards behaviour, not the
race.
**Follow-ups.** Scheduler across a daylight-saving change (L14) belongs in `spawn_scheduler`: hold
`prev` when the clock goes back by less than 2 h.

## 2026-10-20 — Four reviews, and the plan they set (0.8.0, in progress)

**Context.** Before adding more features, the code was reviewed in four independent passes:
the download engine, the service and window, the extension, and what IDM, FDM and qBittorrent
offer that reIDM doesn't. The reports are kept in [reviews/2026-10](reviews/2026-10/) so later
work can see what was found, not only what was changed.
**What they found that matters most.**
- *Data integrity:* the engine counted bytes as downloaded when tokio accepted them, not when
  they were written, and the file was pre-sized, so the "is the data on disk?" check on resume
  could never fail — a crash or a full disk could resume past a hole of zeros. `downloads.json`
  was written without fsync, and a damaged one either blocked start-up or loaded as empty.
- *Scale:* the window fetched and parsed the whole list every 300 ms and broke beyond about
  7,000 downloads (a 10 MB response limit); a batch add rewrote the whole list once per link.
- *Security:* the IDM-protocol WebSocket accepted remove-and-delete-files from any local process;
  `/ext/*` had no `Host` check (DNS rebinding).
- *Extension:* the popup's "best quality" YouTube row always failed; a held browser download
  could be lost if the dialog or the worker went away at the wrong moment.
- *Gaps:* tray and close-to-tray, "when done" actions, keeping the PC awake, proxy and site
  logins, speed modes, user categories and queues, clipboard and drag-and-drop.
**Decision.** Fix the reviews' critical and high findings first (correctness before features),
in four parallel streams that own separate files: engine core, engine modules
(torrent/yt-dlp/tools), service and window, extension. Then add the most-used missing features
and simplify Options: everyday settings in plain tabs, everything else behind an *Advanced
settings* switch, and one *Performance* choice (Light / Balanced / Maximum) instead of a dozen
numbers for people who don't want to tune.
**Alternatives.** Features first (rejected: the integrity bug can corrupt files users already
have); one agent at a time (slower, no benefit, the files don't overlap).

## 2026-10-20 — Self-hosted CI, and a browser test that finds its own browser

**Context.** Linux CI and the `.deb` release job moved to the self-hosted runner `manupa-hnx`
(the runner initially lacked `clippy`; fixed by installing the component in the workflow).
Playwright replaced its Chromium build and deleted the one the browser test named; the test
crashed on spawn and left a test service holding reIDM's ports.
**Decision.** The test picks the newest Playwright Chromium and treats a spawn failure as an
ordinary error so cleanup runs. The release workflow is exercised on a branch
(`workflow_dispatch`, publishes nothing) before a real release depends on the runner.

## 2026-10-16 — Torrents through librqbit (0.7.0, PR #9)

**Context.** Users asked for torrent downloads and seeding.
**Decision.** Use librqbit (Rust, Apache-2.0, DHT/uTP/trackers/fast resume) rather than writing
BitTorrent or bundling a separate client (aria2/qBittorrent). One session for all torrents,
matched to reIDM's list by info hash; reIDM's list stays the source of truth and the session is
reconciled with it at start-up. A torrent is an ordinary list item (slots, pause, categories).
Seeding stops at a share ratio (default 1.0) so users don't silently upload forever.
`.torrent` files on the web are fetched by reIDM with the browser's cookies (private trackers),
not by librqbit. Magnet links are keyed by info hash.
**Alternatives.** aria2 (C++, separate process, weaker integration); qBittorrent-nox (huge,
separate UI). Writing BitTorrent ourselves (years).
**Evidence.** Offline e2e test with a local librqbit seeder; Debian swarm 40–69 peers,
pause/restart/resume repeatedly. A race was found and fixed: librqbit's pause during the
start-up check left a torrent paused after our unpause — the run loop now restarts any torrent
found paused while reIDM runs it.
**Costs.** ~600 crates in the dependency tree; Windows Firewall prompt on first listen.

## 2026-10-16 — Continue old single-connection videos the way they began (0.6.1, PR #8)

**Context.** 0.5.1 switched yt-dlp to parallel fragments; a 4.8 GB download begun by 0.5.0
failed on resume with "fragment 1 not found".
**Decision.** On that exact error, run yt-dlp once more over one connection. Narrow by design:
other failures fail as before.

## 2026-10-15 — Download file info dialog, category folders, temporary folder, playlists (0.6.0, PR #7)

**Context.** The 0.5.1 notification never showed on a user's PC (Windows notifications off
globally). Users wanted IDM's Start-Download dialog, IDM's category folders, part files out of
Downloads, and YouTube playlists.
**Decisions.**
- The dialog is an extension window (`confirm.html`), not the desktop app's: it works when the
  window is closed, and the held browser download stays paused (no *Save as*) while the user
  decides. A port with a heartbeat keeps the MV3 service worker alive. Cancel = nobody downloads.
- Categories live in the engine (`category.rs`) so service, window and extension agree.
  Folders default to `<Downloads>/<Category>` and follow the download folder unless set.
- Unfinished data waits in `<data>/parts` (yt-dlp pieces via `-P temp:`) and is moved into place
  when complete (copy across drives). Legacy part files beside the target are honoured, so
  in-flight downloads are not restarted.
- The playlist window lists with `--flat-playlist` (fast) and looks each video up in the
  background, three at a time; selectors always end in fallbacks so one video lacking a codec
  doesn't fail the batch, and "You get" predicts yt-dlp's choice exactly.

## 2026-10-15 — Faster, sturdier YouTube downloads; stage notes; notifications (0.5.1, PR #6)

**Context.** A 2K download "looked frozen": minutes at 0 B while yt-dlp was fetched, then <1 MB/s
over one connection, then a 403.
**Decision.** `--extractor-args youtube:formats=dashy` + `--concurrent-fragments N` (3× faster in
testing, no 403); an `Item::note` stage line; size announced up front; GUI actions off the UI
thread.

## 2026-10-15 — YouTube through yt-dlp (0.5.0, PR #5)

**Context.** YouTube formats are ciphered or missing; the extension can't resolve them.
**Decision.** Hand the page and a format selector to yt-dlp, fetched on first use from GitHub
releases and verified against published SHA-256 (plus Deno, and ffmpeg on Windows). Rejected:
reimplementing signature deciphering (moving target), bundling yt-dlp (goes stale within weeks).
Processes are killed as a group (`killpg`, `taskkill /T`) because PyInstaller builds are two
processes.

## 2026-10-15 — Windows installer fixes (0.4.1, 0.4.2, PRs #2, #4)

Setup stops reIDM before replacing files ("DeleteFile failed; code 5"); a reIDM started
elevated can't be stopped from normal rights, so Setup now says so instead of failing.

## 2026-10-14 — IDM parity, capture before the browser saves (0.4.0, PR #1)

The extension holds `onDeterminingFilename`, pauses the browser's download, asks reIDM to fetch
the first byte, and only then cancels the browser's copy — no bubble, no *Save as*, no partial
file; anything reIDM can't take is resumed in the browser. Dynamic segmentation, refresh of
expired addresses (same size, ranges if partial), queue/scheduler, limiter, site grabber, `.ef2`.
Found by testing on Windows: native-host manifests had `\\?\` paths browsers can't launch.
