# Development log

Why reIDM is the way it is: each significant change, the reasoning behind it, and what was
considered and rejected. Newest first. Add an entry for every change that involves a decision;
link the PR. Keep entries short — the code and the commit say *what*, this says *why*.

Format: `## YYYY-MM-DD — title (version, PR)`, then **Context**, **Decision**, **Alternatives**,
**Evidence/tests**, and **Follow-ups** where useful.

---

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
