# Development log

Why reIDM is the way it is: each significant change, the reasoning behind it, and what was
considered and rejected. Newest first. Add an entry for every change that involves a decision;
link the PR. Keep entries short — the code and the commit say *what*, this says *why*.

Format: `## YYYY-MM-DD — title (version, PR)`, then **Context**, **Decision**, **Alternatives**,
**Evidence/tests**, and **Follow-ups** where useful.

---

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
