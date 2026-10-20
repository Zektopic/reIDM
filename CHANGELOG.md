# Changelog

## 0.8.0 (unreleased)

### Fixed

* **A download could be left with a hole in it after a crash, power cut or full disk**: progress
  was counted before it reached the disk. It is now recorded only once it is safely written.
* **The download list survives a crash or power cut**: it is written safely, the previous copy is
  kept as `downloads.json.bak`, and a damaged list is recovered from it instead of stopping reIDM.
* **A finished download never replaces a file that is already there**: it is saved as
  `name (1).ext`.
* Pause works straight away even when the server has stopped answering, and on downloads waiting
  in the queue; Resume pressed while a download is stopping is no longer lost.
* Downloads on a flaky connection keep their connections and finish, instead of dropping to one
  connection and failing.
* Re-adding a link whose finished file was deleted downloads it again; re-adding a stopped link
  uses the browser's fresh cookies.
* Changing the temporary folder no longer restarts unfinished downloads from nothing.
* More changed files are noticed instead of being joined to the old version: a server that drops
  byte ranges, a file that got shorter, or a new modification date.
* A batch pattern with a huge range is refused at once instead of freezing the window.
* The scheduler no longer starts or stops the queue twice when the clocks go back.
* **Torrents of several files were put straight into the download folder** — every file loose in
  *Compressed*, *Video*… — instead of a folder named after the torrent, and a file already there
  with the same name was overwritten. A torrent now gets a folder of its own, and a new torrent never
  writes over a file that is already there: if the name is taken it goes into `Name (1)/`, or a
  single file into a folder beside it. Torrents begun by 0.7.0 carry on where their files are.
* **A video could be reported finished with a hole in it** when yt-dlp couldn't fetch one piece. It
  now stops instead, and carries on by itself when YouTube's addresses expire partway (after about
  six hours) or the connection drops.
* The same torrent can't be added twice, whether its magnet link gives the hash in hex or base32 or
  it comes as a `.torrent` from another site. *Start again* on a torrent waits for its old files to
  be removed first.
* A video whose title isn't UTF-8 could hang; `reidm update-check` crashed.
* A partially unpacked ffmpeg was taken for installed.
* The site grabber's *include* filter keeps links containing any one of the patterns, as documented
  (it required all of them).

### Improved

* Adding thousands of links at once takes a moment instead of minutes (2,000 links: 51 s → 0.1 s),
  and the service stays responsive while the list is saved.
* Files taken from the browser are marked as downloaded from the internet (Windows SmartScreen,
  macOS Gatekeeper), as the browser would have marked them.
* On Linux, a disk too small for a download says so when it starts. On Windows, large downloads no
  longer stall while the disk is filled with zeros.
* The download list and settings are readable only by you on Linux and macOS (they hold cookies).
* Videos and torrents keep to the global speed limit: each gets its share, and what they fetch counts
  against the rest. A new limit reaches a running video within seconds.
* yt-dlp, Deno and ffmpeg are downloaded to disk and checked as they arrive, not held in memory. A
  video no longer waits while another download fetches a tool; *Pause* works while a tool
  downloads; a failed update check is tried again after an hour, not at every video.
* A `.torrent` link that sends more than 10 MiB, or a page for the site grabber over 8 MiB, is
  refused instead of being read into memory.

## 0.7.0

### Torrents

* **reIDM downloads and seeds torrents**, with [librqbit](https://github.com/ikatson/rqbit):
  magnet links and `.torrent` files, trackers, the DHT and uTP. A torrent is a download like any
  other in the list — queued, stopped, resumed across restarts, filed by what it holds, removed
  with its files — showing its peers and its upload beside its download.
* **Choose the files** in **Tasks › Add torrent…**, from a magnet link, a `.torrent` address or a
  file on disk.
* **Seeding**: a finished torrent shares until it has uploaded as much as its size, then stops;
  **Options › Torrents** sets that ratio (or none, or forever), an upload limit, the listening
  port (forwarded by UPnP) and the DHT. Right-click › *Stop seeding* / *Seed again*. Seeding
  carries on after a restart.
* **The browser**: a clicked magnet link goes to reIDM (Alt+click leaves it to another program);
  a `.torrent` the browser downloads is fetched by reIDM with the site's cookies — so a private
  tracker's work — and added as the torrent it describes.
* `reidm add magnet:…` and `reidm add file.torrent`; `POST /api/torrent/inspect`, `seed` and
  `stop-seeding` actions, and `torrent` on downloads in the API.

## 0.6.1

### Fixed

* **A video download begun by reIDM 0.5.0 failed when resumed** with "fragment 1 not found,
  unable to continue": 0.5.0 fetched videos over one connection, and since 0.5.1 yt-dlp resumes
  in pieces, which can't continue such a file. reIDM now carries it on the way it began, keeping
  what was already downloaded.

## 0.6.0

### IDM's "Download file info" dialog

* Before a download starts, the extension shows the address, the file name (to change if you
  like), the folder it will go to and its size, with **Start download**, **Download later** and
  **Cancel** — for downloads it takes from the browser, clicked file links, the right-click menu
  and a video's qualities. Cancel means nobody downloads it, neither reIDM nor the browser. *Don't
  ask again*, or *Ask before each download* in the extension's Options, turns it off.
* Notifications alone weren't enough: with Windows' notifications switched off, nothing at all
  said a download had started.

### Folders

* **Downloads are filed by type**, as in IDM: `Compressed`, `Documents`, `Images`, `Music`,
  `Programs` and `Video` under the download folder, anything else in the download folder itself.
  **Options › Folders** — a new tab — moves any of them, or turns sorting off.
* **Unfinished downloads wait in a temporary folder** (`parts` in the data folder, or one set in
  Options › Folders) and move to their folder when complete, so the download folder no longer
  fills with `.reidm` and yt-dlp part files. Downloads already under way carry on where they are.
* The window's categories (and the new *Images*) follow the same rules as the folders.
* `GET`/`POST /api/settings` carry `sort_by_category`, `category_dirs` and `temp_dir`.

### YouTube playlists

* **Tasks › Add YouTube playlist…** lists every video of a playlist or a channel, with its length
  and — looked up in the background — the resolutions and codecs it comes in. Pick a resolution
  and codec per video, or for all at once (*1080p H.264*, *720p H.264*, *4K*, *Best quality*,
  *Audio M4A*, *Audio MP3*); *You get* shows what each will really be. Download now or later, by
  default into a folder named after the playlist.
* New endpoints `POST /api/media/playlist` and `POST /api/media/info`; `media.audio` converts the
  sound to MP3 or Opus.

## 0.5.1

* **A notification when reIDM takes a download.** The extension shows *Downloading in reIDM* with
  the file's name — for a captured download, a clicked link, a video from the list, the
  right-click menu and the grabber — and says why when reIDM can't take one. The browser's own
  download bubble doesn't appear for what reIDM takes, so this is how you know it started. It can
  be turned off in the extension's Options.
* **YouTube downloads are faster and sturdier:** yt-dlp now fetches a video in pieces over as many
  connections as *Connections per download* (up to 16). One connection is throttled by YouTube, and
  a long one is often cut off with a 403. About three times faster in testing.
* **A video download says what it's doing** before its bytes start: *Getting yt-dlp (first time
  only)*, *Looking up the video*, then *Joining video and audio* at the end. A first YouTube
  download used to sit at "0 B" for minutes while reIDM fetched yt-dlp, which looked like a hang.
  `reidm list` shows the same.
* **The size of a video is right from the start,** video and audio together, instead of an
  estimate that swung by gigabytes while it downloaded in pieces.
* The window no longer waits on the download service when you pause, resume or open a download,
  so a slow answer can't freeze it.

## 0.5.0

### YouTube

* **Every quality in YouTube's list can now be downloaded.** They were all greyed out: YouTube
  gives the browser scrambled addresses (`signatureCipher`), or none, and only its player's code
  unscrambles them. reIDM now hands the page and the chosen format to
  [yt-dlp](https://github.com/yt-dlp/yt-dlp), which keeps up with YouTube's changes.
* **Picture and sound in one file.** Qualities above 360p come as separate video and audio;
  ffmpeg joins them. The quality clicked is the one fetched when yt-dlp offers it, else the best
  at that height, H.264 with AAC first.
* **reIDM fetches what it needs by itself** the first time: yt-dlp (checked for a newer release
  once a day), Deno for yt-dlp to read YouTube's player when no Deno, Node or Bun is installed,
  and on Windows ffmpeg — each from its project's GitHub releases, checked against the SHA-256
  the release publishes. On macOS and Linux, ffmpeg comes from the system (the `.deb` recommends
  it).
* Video downloads live in the list like any other: progress, stop and resume, the speed limit,
  *Downloads at the same time*. They are named after the video and quality; Properties shows the
  format; removing one also removes yt-dlp's partial files.
* The popup offers the video on a YouTube page as one row, in the best quality, instead of the
  player's internal addresses (which would have saved a web page).
* Audio formats are labelled by bitrate ("130k") instead of YouTube's "tiny", and YouTube's
  loudness-normalised copies are marked *stable volume*.
* `reidm add --video [FORMAT] URL…` does the same from a terminal, for YouTube and the other
  sites yt-dlp supports. The API's `POST /api/downloads` and `/ext/add` take `media`, and
  `/ext/ping` says `media: true`; the extension won't send a video to an older reIDM, which would
  save the page instead.

### Fixed

* `reidm list` showed a download that had just started as *Queued* until its first progress was
  saved.

## 0.4.2

### Fixed

* **Windows: Setup still failed with "DeleteFile failed; code 5" when reIDM had been started as
  administrator.** Setup runs with your normal rights, so it can neither stop such a reIDM nor
  see where it runs from. It now notices one, and stops with a message saying how to close it,
  instead of failing on the first file. The uninstaller does the same.

## 0.4.1

### Fixed

* **Windows: installing over a running reIDM failed with "DeleteFile failed; code 5".** reIDM's
  service runs all the time, so an upgrade found `reidm.exe` in use and Setup couldn't replace it.
  Setup now stops the reIDM programs running from the install folder first (the service, the
  window, a native host the browser started), and starts the service again when it's done. The
  uninstaller stops them too. Unfinished downloads resume where they were.

## 0.4.0

The browser now hands its downloads to reIDM *before* saving anything, the engine gains IDM's
remaining core features, and the desktop window catches up with IDM's.

### Browser integration

* **Downloads are taken before the browser saves them.** The extension holds Chrome's
  file-name decision (`onDeterminingFilename`) for every download, pauses it, and asks reIDM to
  take it with the browser's cookies, referrer and user agent. Only after reIDM has fetched the
  first byte is the browser's copy cancelled and erased — so there is no download bubble, no
  *Save as* dialog and no partial file, even for a 2 KB file or with *Ask where to save each
  file* on. If reIDM can't take it, the download is resumed exactly as it was.
* **File links are taken on click**, before the browser requests anything. Links that turn out to
  be web pages (login walls, "your download will start" pages) are handed back to the browser.
* **reIDM is started on demand.** A new native-messaging host, `com.zektopic.reidm`, lets the
  extension start reIDM when it isn't running. The extension has a fixed ID, so the host admits
  only it. The service re-registers the host with every Chromium-based browser at each start.
* **Form submissions, `blob:`/`data:` files, other extensions' downloads, Alt+click, excluded
  sites and incognito windows stay with the browser**, and the popup says which rule kept each one.
* **Resource grabber.** Lists everything a page offers, from every frame — links, images (the
  largest `srcset` candidate, lazy-load attributes, CSS backgrounds), video, audio, subtitles,
  embeds, and every resource the page actually loaded — with filters, sizes on request, and one
  batch hand-off, or *Download later*, or an `.ef2` export.
* **New popup and options.** Status with *Start reIDM*, recent hand-offs, media on the page,
  *Never on this site*; capture rules for file types, minimum size, excluded sites and incognito.
* Media is also recognised by the type the server reports, with sizes from the response headers.
* **Every installer ships the extension**, and reIDM keeps a copy in the data folder up to date,
  so updating reIDM updates the extension. **Help › Browser integration** shows the folder to load
  and whether the extension is connected; `reidm extension` prints it. Releases add
  `reidm-extension-<version>.zip`.

### Download engine

* **Dynamic segmentation.** A connection that finishes its part takes over half of the largest
  remaining one, so a download doesn't wait on its slowest connection.
* **Servers that limit connections** (`429`, `503`) no longer fail a download: the refused
  connection hands its range to the others.
* **Never splices a changed file.** Resumed requests carry `If-Range` with the server's `ETag`,
  and every range's total size is checked; a file that changed on the server stops the download
  with a clear message, and *Restart* fetches the new one.
* **Speed limiter**, global and per download, applied immediately to running transfers.
* **Scheduled queue and scheduler.** *Download Later*, *Start/Stop Queue*, and start and stop
  times on chosen weekdays, in local time, including overnight windows. A finished download
  leaves the queue.
* **Refresh download address.** Give a stopped download a new link to the same file — typed, or
  captured from the browser's next download — and it carries on from where it was.
* **Restart** a download from the first byte.
* **IDM `.ef2` import and export**, with each download's referrer, cookies and user agent; form
  (POST) entries are reported rather than fetched with a GET. **Batch downloads** from `*`
  patterns, numbers or letters, up to 10,000 addresses.
* Responses are no longer transparently decompressed, so the bytes saved are the bytes the
  server stores.

### Desktop window

* Toolbar: Scheduler, Start Queue, Stop Queue, Site Grabber, Speed Limiter. Tree: Unfinished,
  Finished and the scheduled queue next to the file types.
* **Properties** with a live segment map — each range, its progress, and where each connection is
  writing — and the download's own speed limit.
* Windows for refreshing an address, batch downloads, the scheduler, the speed limiter, browser
  integration, and IDM's *Download complete*.
* *Add URL* takes a folder, offers *Download later*, and fills itself from the clipboard.
* Multi-select delete; keyboard shortcuts (Delete, Ctrl+A, Enter, Ctrl+N/Insert, Ctrl+V); the
  rate is averaged over three seconds instead of flickering; preferences persist in `gui.json`.

### Command line and API

* New commands: `batch`, `limit`, `queue`, `refresh`, `restart`, `extension`; `add --later`,
  `import` of `.ef2` files, `export --format ef2`. `list` shows scheduled downloads and why a
  download failed.
* New endpoints for each of the above, and `/api/status` reports the connected extension.
* Documentation: a user guide, browser integration, command-line and HTTP API references,
  architecture and security model, troubleshooting and building guides.
* `REIDM_DATA_DIR` moves the data folder on every system, for a separate instance or a test.

### Packages

* **The macOS disk image is a proper drag-to-Applications installer**: its window shows reIDM,
  an arrow and the Applications folder on a drawn background, and nothing else. Everything
  reIDM needs is inside the app now — the service, the browser's native host, the extension — so
  there is no install script to run; open reIDM once and it registers itself with your browsers.
* The real-browser test (`tools/capture-test.mjs`) runs on Windows too.

### New look

* A new icon: a download button in front of a globe, with a small "re". Generated by
  `tools/make_icon.py`, with the small sizes drawn rather than scaled.

### Fixed

* **Two transfers could run the same download** when two parts of reIDM started queued downloads
  at the same moment; they wrote one file until one renamed it away from the other. The slot is
  now reserved under the lock that chooses it.
* **IDM's Alt-click and Insert-click didn't work with Tonec's extension.** reIDM sent 0 for the
  key masks in its settings message, which disabled both; it now sends IDM's defaults (found by
  reading the extension's code side by side with the decompiled IDM).
* On Windows, a reIDM service started through the browser's native host is no longer killed
  when the browser closes the host: it now leaves the browser's job object.
* **On Windows, the browser couldn't start reIDM when reIDM had registered its native host
  itself** (rather than the installer): the registration named the host by an extended-length
  path (`\\?\C:\…`), which browsers can't launch — Chrome said only "Error when communicating
  with the native messaging host". Paths are now written plainly, and existing registrations are
  rewritten at the next start. Found by running the browser test on Windows.
* On Windows, registering the native host no longer writes "The operation completed
  successfully." into the service's log, or flashes a console window.
* The release workflow's notes had an unindented line that made the whole file invalid YAML,
  so the 0.4.0 release would not have built.

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
