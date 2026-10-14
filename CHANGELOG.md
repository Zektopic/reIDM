# Changelog

## 0.3.1

### Added

* **A video quality picker in the extension.** A *Download with reIDM* button appears over a playing
  video and opens a list of what the player actually offers — resolution, codec, fps, bitrate and
  size, best first, with audio-only tracks tagged and last. It understands YouTube's
  `streamingData`, HLS `#EXT-X-STREAM-INF` and DASH `<Representation>`. The button is anchored to the
  video's own box and follows it on scroll and resize; it is hidden when the player is too small.
* The picker is rendered inside a closed shadow root, because a page's own CSS on a site like
  YouTube is aggressive enough to break a plain element.

### Fixed

Five bugs in the extension, all found by loading it in a real Chromium and watching what it did:

* **The service worker never started.** A top-level `await` before the listeners were registered is
  the pattern the docs show, and Chrome accepts it silently: the extension loaded, logged nothing,
  and did nothing at all. Listeners are now registered synchronously and the settings are corrected
  asynchronously.
* **A refused download was destroyed.** The browser's copy was cancelled before reIDM was asked, so
  if reIDM was not running the file was simply lost. It is now paused, offered, and cancelled only
  on acceptance — otherwise resumed. See `docs/PROTOCOL.md`; the official extension does the same.
* **A manifest fetched inline was never seen.** A page that loads its manifest from an inline
  `<script>` does so before the injected hook exists. The worker now re-fetches and parses manifests
  it observes through `webRequest`, which does not depend on timing.
* **A richer format list was being downgraded.** The page hook's answer overwrote the worker's list,
  silently collapsing a four-variant stream to one progressive file. The two sources are now kept
  apart and shown as a union.
* **Everything was lost when the service worker was torn down.** Chrome stops the worker freely, and
  the per-tab media index lived only in module scope. It is persisted to session storage now.

## 0.3.0

### Added

* **Batch downloads.** `Add URL` now takes a list, one per line; `reidm import <file>` reads one from
  a text file or stdin, and `POST /api/downloads/batch` answers per URL so the caller can report
  "8 queued, 2 resumed, 1 failed".
* **A queue.** `max_concurrent` (default 4, settable in Options) caps how many downloads transfer at
  once; the rest wait and start as slots free. Adding a hundred links no longer opens a hundred
  connections.
* **Resume by re-adding the link.** Queueing a URL that is already known resumes it instead of
  creating a second copy: a paused or failed download carries on from where it stopped, and a
  finished one is reported as already downloaded. URL identity ignores the fragment and sorts the
  query, so `?b=2&a=1` and `?a=1&b=2` are the same download.
* **Site grabber** (`reidm grab`, the Site Grabber window, `POST /api/grab`). Scans a page for files,
  optionally follows links to a chosen depth, filters by extension, host and include/exclude
  patterns, and can ask each server for the size. It reports when a page's links are built by
  JavaScript rather than pretending it found none.
* **Import and export.** URL lists in and out as plain text, from the File menu, the grabber window,
  or `reidm import` / `reidm export`.
* **reIDM's own browser extension** (`extension/`): MV3, takes over browser downloads by pausing
  them and cancelling only once reIDM has accepted the URL, right-click entries for links, pages,
  selections and media, and media sniffing through `webRequest` plus a page-side `fetch`/`XHR` hook.
  It has its own `/ext/ping` and `/ext/add` endpoints, authorised by origin rather than by token.

### Fixed

* The scheduler hands a freed slot to a waiting download when one finishes, is removed, or the limit
  is raised.
* The grabber reads `Content-Length` from the header rather than `Response::content_length()`, which
  reports the empty body of a HEAD response as zero.

## 0.2.2

* The window checks for a new release **once shortly after it starts**, so a newer version is
  noticed without the user going looking for it. It is a single GET to api.github.com; nothing is
  reported back, and nothing is downloaded or run automatically.
* Fixed a clippy failure on the macOS job (`chunks_exact` with a constant size).

## 0.2.1

### Added

* **Update checking.** The window, the Options dialog and `reidm update-check` now compare the
  running version against the repository's latest GitHub release. It reports and links; it never
  downloads or runs anything by itself. The repository is private, so a check needs a token —
  set `REIDM_GH_TOKEN` (or `GH_TOKEN`/`GITHUB_TOKEN`), or make the repository public and it works
  unauthenticated.
* A **redesigned window** closer to IDM's own: a menu bar (Tasks/File/Downloads/View/Help), an
  icon-over-label toolbar with Resume, Stop, Stop All, Delete, Delete Completed, Options and
  Scheduler, a category tree with expanders and per-type icons, and columns for Time Left,
  Transfer Rate and Last Try alongside the existing ones, plus a row-selection checkbox column.
  Clicking a column header sorts.
* The **queue, grabber and quota nodes** are shown greyed with a tooltip saying they are not
  implemented, rather than omitted, so the window does not look like it quietly lost features.
* A new **application icon** with a gradient, an optical-size-aware glyph, and a multi-size
  `.ico` (16/24/32/48/64/128/256) instead of one large bitmap, so Explorer and the taskbar each
  get a crisp one. `tools/make_icon.py` still has no third-party dependencies.
* `docs/PACKAGING.md`, covering how each artifact is built and what signing needs.

### Fixed

* **The macOS build could not run on Apple silicon.** `lipo` does not carry code signatures into
  the universal binary it produces, and macOS refuses to execute an unsigned arm64 binary — the
  symptom is a bare `Killed: 9` with no dialog. Each binary is now re-signed after `lipo`, and
  the bundle is signed too.
* Every `.strong()` label in the window rendered **white on white**, because egui resolves strong
  text to `widgets.active.fg_stroke` and the theme used that slot for the primary button's white
  caption. The column headers were invisible as a result.
* Tree expanders pointed the wrong way: expanded nodes showed a collapsed chevron.
* The sort indicator was drawn with `▲`/`▼`, which the bundled font has no glyph for and rendered
  as a missing-character box; it is painted now.

## 0.2.0

The first release with a real desktop application, and a correctness pass over the download engine.

### Added

* **Native desktop window** (`reidm-gui`, `reIDM.app`): toolbar, category tree, sortable download table
  with progress/size/status/time-left/rate columns, status bar, Add-URL and Options dialogs, light and
  dark themes. It is a front-end for the background service, so downloads continue when the window closes.
* An application icon for every platform (`.ico` for Windows, `.icns` in the macOS bundle, PNGs plus a
  `.desktop` entry for Linux), generated by `tools/make_icon.py` with no image-library dependency.
* `GET /api/status` on the daemon, and a status-bar warning when another program (typically an installed
  IDM) holds port 1001 on Windows, so the browser-integration problem is visible instead of buried in a log.
* `Options` can change the download folder and the connections per download at runtime; they persist in
  `settings.json` next to the download list.

### Fixed

A code audit of the 0.1 engine found several ways a download could end up wrong. Each fix has a
regression test in `crates/reidm-core/tests/resume.rs`.

* **Silent corruption when resuming from a server that ignores `Range`.** The engine asked for the whole
  body but wrote only the outstanding byte count at offset 0, leaving a stale tail from the previous
  attempt — then reported the download as complete. It now restarts the transfer properly.
* **Silent corruption when the part file is shorter than the database believes** (deleted, truncated, or a
  power loss between the two writes). `set_len` filled the gap with zeros. The engine now verifies the
  bytes are on disk and starts over when they are not.
* **`rm --purge` / "Remove and delete the file" never deleted a finished download**, because a completed
  item has no part file left for the old code to match on.
* **A lock-order inversion between `list()` and `resume()`** that could wedge the whole daemon (web UI,
  CLI and extension) permanently. All locks are now taken `db` before `live`.
* **Two downloads could be given the same path and part file**, corrupting each other, because the temp
  file only appears after the probe completes. Paths are now reserved at queue time.
* **An orphaned `.reidm` part file** was left behind whenever an unfinished download was removed, which
  made every later download of that name become `name (1).ext`.
* **Zero-byte files could not be downloaded at all** (the probe treated `416` as an error).
* **A hostile or buggy peer could panic the connection task** with a negative string length in a protocol
  frame; lengths are now validated.
* **A stalled server hung the download forever** and Pause had no effect, because cancellation was only
  checked after a chunk arrived. There is now an idle timeout and a 1-second cancellation tick.
* **Removing a download while its probe was in flight could resurrect it** from a stale copy.
* **`reidm patch-extension SRC SRC` wiped the extension**, because `fs::copy` truncates its destination.
* **A second daemon overwrote the API token file before discovering the port was taken**, breaking the
  running daemon's clients until it was restarted.
* Windows: a filename of `NUL`/`CON.txt`/`COM1` resolved to a device and discarded the data; filenames are
  now escaped.
* Windows: "Show in folder" failed for any path containing a space, because `explorer /select,` does not
  use the normal argument-quoting rules.
* `Content-Disposition` with a quoted semicolon (`filename="a;b.zip"`) lost the extension, and RFC 5987
  language tags (`filename*=UTF-8'en'…`) were not handled.
* The download reply sent to the extension claimed the download was taken even when the queue had
  rejected it, which made the browser drop it silently.
* The API token is generated with a real fallback instead of a constant if the OS random source is
  unavailable, and the token file is created private rather than chmod-ed afterwards.

### Changed

* The no-console launcher that used to be called `reidm-gui` is gone; `reidm-gui` is now the desktop app.
  `reidm-gui --background` keeps its old job of starting the service at login.
* `docs/PROTOCOL.md` now separates what the vendor binaries do from what reIDM implements, and no longer
  presents the original `IDMMsgHost` named-pipe design as if reIDM spoke it.

## 0.1.1

* Web UI, no-console Windows launcher, hidden autostart.

## 0.1.0

* Initial release: segmented resumable download engine, daemon, CLI, native-messaging host, packages.
