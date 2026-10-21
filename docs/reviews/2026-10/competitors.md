# reIDM competitor gap report: IDM, FDM, qBittorrent

Researched 2026-10-20 against reIDM `feature/torrents` (0.7.0 unreleased, librqbit 9.0.1).

## Sources and how far to trust them

| Product | Source | Trust |
|---|---|---|
| reIDM | `README.md`, `docs/USER_GUIDE.md`, `docs/BROWSER_INTEGRATION.md`, `docs/CLI.md`, `docs/API.md`, `CHANGELOG.md`, `crates/reidm-core/src/lib.rs` (`Settings`, engine constants), `crates/reidm-core/src/torrent.rs`, `crates/reidm-gui/src/app.rs` (menus, Options tabs General / Folders / Torrents), `crates/reidm-gui/src/prefs.rs`, and the librqbit 9.0.1 sources in `~/.cargo/registry` | Primary source |
| qBittorrent | Shallow clone of `qbittorrent/qBittorrent` at commit `846f97e` (2026-10-20). Only read, never built or run. Main files: `src/gui/optionsdialog.ui`, `src/gui/advancedsettings.cpp`, `src/base/bittorrent/sessionimpl.cpp` (settings keys and defaults, lines ~486–625), `src/gui/transferlistwidget.cpp`, `src/gui/mainwindow.ui`, `src/app/application.cpp`, `src/base/preferences.cpp` | Primary source; the defaults quoted come from the code |
| IDM | internetdownloadmanager.com: `features2.html`, `support/using_idm/options.html`, `support/using_idm/starting.html`, `support/using_idm/using_idm.html`, `support/idm-scheduler/idm_scheduler.html`, `support/idm-scheduler/idm_queues.html`, `support/idm-grabber/grabber_wizard.html`, `grabber_settings.html`, FAQ `register/new_faq/functions8.html` (connections), `functions9.html` (shutdown), `functions10.html` (keys), `functions11.html` (speed limiter) | Official pages, but closed source. Items marked **[UI, unverified]** come from knowledge of the IDM 6.x interface and are not on the official pages I fetched |
| FDM | freedownloadmanager.org: `features.htm`, home page, `vote.htm`, `blog/`, board thread `viewtopic.php?t=18914`; third-party guide cyberraidentech.blogspot.com (2020, FDM 6 settings) | **There is no official FDM 5/6 source repository.** GitHub search found only forks of old FDM 3.x and add-ons, and Wikipedia says the source stopped at 3.9.7 (SourceForge SVN). So FDM details come from web pages and are marked **[3rd-party]** where they come from a blog |

## What reIDM already has

These are the baseline, not gaps:

- **HTTP(S) engine.** 8 connections per download by default (1–32), dynamic segmentation, `If-Range` and size checks, 5 retries, 60 s idle timeout, resume across restarts, refresh of an expired address (pasted, or captured from the browser), restart.
- **Limits and scheduling.** A global speed limiter, a per-download limit, one scheduled queue, and a scheduler with start/stop times, weekdays and overnight windows. *Downloads at the same time* defaults to 4 (16 in the GUI, 64 in core).
- **Organisation.** Fixed IDM categories with per-category folders and a temporary folder.
- **Adding downloads.** Batch `*` patterns, site grabber (depth up to 3 in the GUI), `.ef2` and txt import/export, *Download complete* window, dark theme, keyboard shortcuts, clipboard pre-fill when Add URL opens.
- **Browser extension** (Chromium only). Takeover before save, click interception, *Download file info* dialog, never-capture sites, minimum size, Alt bypass, media detection, resource grabber, YouTube and playlists through yt-dlp.
- **Torrents.** Magnet and `.torrent` links, file selection, a seeding ratio (none / 0.5 / 1 / 2 / 5 / forever), upload limit, port with UPnP, DHT, stop/seed again, peers, per-file progress.
- **Scripting and remote.** A CLI, a local JSON API, a web view on 127.0.0.1:61002, update check, autostart on Windows.

**Missing in code:** a tray icon, proxy settings, HTTP auth and site logins, FTP, OS notifications from the app, sounds, completion actions, checksums, custom categories, and multiple queues. On the torrent side, librqbit 9.0.1 has **no protocol encryption (MSE/PE)**. It does offer a SOCKS5 `proxy_url`, a `blocklist_url` and allowlist, per-torrent `ratelimits` and `peer_limit`, `trackers`, `update_only_files`, `create_torrent_file`, file `stream()`, `disable_upload` and `disable_local_service_discovery`.

---

## 1. Feature gap table

Columns: ✓ = has it, ~ = partial, ✗ = no, ? = unverified. Value: H/M/L. Effort: S/M/L, judged against reIDM's architecture.

### A. System integration and convenience

| Feature | IDM | FDM | qBt | reIDM | Value | Effort | How competitors do it |
|---|---|---|---|---|---|---|---|
| Tray / notification-area icon; minimize or close to tray; tray menu (speed limiter, exit) | ✓ (speed limiter from tray, `functions11.html`) | ✓? | ✓ | ✗ (GUI has none; the service runs headless) | H | M | qBt `optionsdialog.ui` › Desktop: "Show qBittorrent in notification area", "Minimize…/Close qBittorrent to notification area", tray style Normal/Monochrome. Defaults `SystrayEnabled=true`, `MinimizeToTray=false`, `CloseToTray=true` (`src/base/preferences.cpp:246–293`) |
| Actions when all downloads finish: exit, sleep, hibernate, shut down, reboot | ✓ via the Scheduler: "Turn off computer when done", "Hang up modem", "Force processes to terminate", "Open the following file when done" (`idm_scheduler.html`) | ✓ ("launch applications, connect/disconnect" in scheduler, `features.htm`) | ✓ Tools › "When downloads complete": Disabled/Exit/Suspend/Hibernate/Shutdown (`mainwindow.ui:75–80`, `ShutdownDialogAction` in `src/base/types.h`); countdown confirmation (`shutdownconfirmdialog.ui`) | ✗ | H | S–M | qBt shows a cancellable countdown dialog and has "Confirmation on auto-exit when downloads finish" |
| Prevent system sleep while downloading / seeding | ? | ? | ✓ "Inhibit system sleep when torrents are downloading / seeding" (`optionsdialog.ui` Power Management; `src/gui/powermanagement/`) | ✗ | H | S | Platform calls: `SetThreadExecutionState` on Windows, IOPMAssertion on macOS, logind/DBus inhibit on Linux |
| Start with the OS | ✓ | ✓ | ✓ (Windows) | ~ (Windows only, by the installer; no toggle) | M | S | Toggle in Options › General |
| Clipboard monitoring (prompt when a link is copied) | ✓ "URL clipboard monitoring" (Options › General) | ? | ✗ | ~ (reads the clipboard only when Add URL opens) | M | S | IDM pops the Download File Info dialog for copied URLs whose extension is on the file-type list |
| Drag and drop links onto the window or a floating drop target | ✓ drop target, "Tasks › Show drop target" (`starting.html`) | ✓? | ✓ (.torrent and magnet drops) | ✗ | M | S (window) / M (floating target) | |
| Desktop notifications from the app (complete, failed) | ~ (dialog) | ✓ "Notify me of completed downloads via Notification Center", on by default [3rd-party] | ✓ "Display notifications", "…for added torrents", timeout (`advancedsettings.cpp`) | ~ (completion window; the extension notifies) | M | S | |
| Sounds per event | ✓ Options › Sounds | ✗? | ✗ | ✗ | L | S | |
| Antivirus scan after download | ✓ Options › Downloads › "Virus checking settings", `[File]` placeholder, presets for known AVs | ✓ antivirus dropdown, e.g. Windows Defender [3rd-party] | ✗ (it has "Enable Mark-of-the-Web (MOTW)" and "quarantine for downloaded files", `advancedsettings.cpp`) | ✗ | M | S | |
| Mark-of-the-Web / macOS quarantine on finished files | ✓? (Windows attachment API) | ? | ✓ (advanced; on by default upstream — default not re-checked) | ✗ | M (security) | S | Write a `Zone.Identifier` ADS with HostUrl/ReferrerUrl on Windows; set `com.apple.quarantine` on macOS |
| Run an external program on add / finish, with placeholders | ✓ "Open the following file when done" (queue level) | ✓? | ✓ "Run on torrent added / finished" with `%N %L %G %F %R %D %C %Z %T %I %J %K` (`optionsdialog.cpp:732–745`) | ✗ | M | S | |
| Email on completion | ✗ | ✗ | ✓ SMTP: from/to/server/encryption/auth/test (`optionsdialog.ui`) | ✗ | L | M | |
| UI language / translations | ✓ many | ✓ 24 languages | ✓ ~70 | ✗ (English only) | H (global audience) | L | |
| Customizable columns and toolbar | ✓ (`using_idm.html`: columns, toolbar customize) | ~ | ✓ column visibility/resize (`transferlistwidget.cpp`) | ~ (sortable, fixed set) | L–M | S | |
| Statistics (all-time downloaded/uploaded, share ratio) | ✗ | ✗ | ✓ `statsdialog.ui` | ✗ | L | S | |
| Speed graph | ✗ | ✓? (FDM 6 has a speed graph) | ✓ "Enable speed graphs" | ~ (segment map only) | L | M | |
| Execution / event log viewer | ✗ | ~ (6.34 adds "disable logging") | ✓ `executionlogwidget`, log file rotation (Behavior › Log Files) | ✗ (only `RUST_LOG` to stderr) | M (support) | S | |
| Show free disk space and external IP in the status bar | ✗ | ✗ | ✓ "Show free disk space", "Show external IP" | ✗ | M | S | |
| Confirm exit with active downloads | ✓? | ? | ✓ `ExitConfirm=true` | ✗ | L | S | |
| UI lock password | ✗ | ✗ | ✓ (`mainwindow.cpp` "UI lock password") | ✗ | L | S | |
| Export / import settings | ✗ | ✓ Export/Import settings [3rd-party] | ~ (ini file) | ~ (`settings.json` can be copied) | L | S | |
| Portable mode | ✗ | ✓ (3.9) | ✓ (`application.cpp` "Running in portable mode") | ~ (`REIDM_DATA_DIR`) | L | S | |
| Delete files to the Recycle Bin / Trash instead of permanently | ? | ✓ "Delete button action" setting [3rd-party] | ✓ "Torrent content removing mode: Delete permanently / Move to trash" (`advancedsettings.cpp`) | ✗ | M | S | `trash` crate |

### B. Network and HTTP engine

| Feature | IDM | FDM | qBt | reIDM | Value | Effort | How competitors do it |
|---|---|---|---|---|---|---|---|
| Proxy (system / manual HTTP, HTTPS, SOCKS5, auth, exceptions, PAC) | ✓ Options › Proxy/Socks: "Get System", PAC script, per-protocol proxy, "Use proxy/socks from a browser's request", exceptions with `*`, login | ✓? | ✓ Connection › Proxy Server: Type/Host/Port/auth, "Perform hostname lookup via proxy", use for BitTorrent / peers / RSS / general | ✗ | H | M | reqwest supports `Proxy::all/http/https`, socks5h (feature `socks`) and system proxy. librqbit `ConnectionOptions.proxy_url` (socks5) |
| HTTP authentication (Basic/Digest/NTLM/Kerberos), credential prompt | ✓ Basic, Negotiate, NTLM, Kerberos (`features2.html`) | ✓? | n/a | ✗ (README: "no HTTP authentication prompts") | H | M | IDM Add URL › "Use authorization" |
| Saved site logins per host/path | ✓ Options › Site Logins (per directory) | ? | n/a | ✗ | M | M | Store in the OS keychain (`keyring` crate) |
| FTP / FTPS (PASV) | ✓ "Use FTP in PASV" | ✓ | ✗ | ✗ | L–M | M | `suppaftp` crate |
| Per-server connection exceptions | ✓ Options › Connection › "Exceptions:" table (host → threads) | ✓ "Max number of connections per server" per traffic mode [3rd-party] | n/a | ✗ (auto-backs off on 429/503) | M | S | |
| Download quotas (MB per N hours) | ✓ "Download Limits", e.g. 40 MB/hour, 150 MB per 4 h | ✗ | ✗ | ✗ | L | S | |
| Speed modes / alternative limits and a bandwidth schedule | ✓ (limiter on/off) | ✓ Low/Medium/High traffic modes, each with DL/UL limit, max connections, connections per server and simultaneous downloads; High cannot be limited (board t=18914) | ✓ Alternative rate limits (default 10 KiB/s), scheduled From/To and Every day/Weekdays/Weekends (`sessionimpl.cpp:585–588`) | ~ (a single limit) | H | M | |
| Configurable retries, timeouts, retry delay | ✓ queue "Number of retries for each file" (default: requeue, retry every 30 s indefinitely) | ? | n/a | ✗ (constants: `MAX_RETRIES=5`, `IDLE_LIMIT=60s`, connect 20 s) | M | S | |
| "Ignore file modification time changes when resuming" | ✓ Options › Downloads | ? | n/a | ✗ (strict `If-Range`) | L–M | S | Let `If-Range` use the ETag only; tolerate a changed `Last-Modified` when size and ETag match |
| User agent for manually added downloads / custom headers | ✓ "User agent for manually added downloads" | ? | n/a | ~ (browser UA passed through; fixed for manual adds) | L | S | |
| Mirrors / multi-source download | ✗ | ✓ (3.9 "Multiple mirrors") | n/a | ✗ | L | L | |
| Checksum verification (MD5/SHA-1/SHA-256/SHA-512) | ✗ | ✓ "SHA-512 file integrity check" (released, `vote.htm`) | ✓ (piece hashes; "Recheck torrents on completion") | ✗ for HTTP | M | S | |
| Pause slow downloads so the next queued one starts | ✗ | ✓ "Pausing slow downloads" (released, `vote.htm`) | ✓ "Do not count slow torrents in these limits", 2 KiB/s thresholds, 60 s timer (`sessionimpl.cpp:530–533`) | ✗ | M | S | |
| Bind to a network interface / IP | ✗ | ✗ | ✓ advanced "Network interface", "Optional IP address to bind to" | ✗ | L (VPN users: M) | S | reqwest `local_address`; librqbit `bind_device_name` |
| Pre-allocate disk space | ✗? | ✓? | ✓ "Pre-allocate disk space for all files" (default off) | ✗ | L | S | |
| Free-space check before start | ✓? | ✓? | ✗ | ✗ | M | S | |
| Live media preview while downloading | ✓ zip preview in Download File Info | ✓ "Preview audio and video files before download finishes" | ✓ "Preview file…" | ✗ | M | M | |
| Video conversion after download | ✗ | ✓ "convert file formats after downloading" | ✗ | ~ (MP3/Opus for YouTube audio) | L | M | |
| HLS/DASH merge for non-YouTube sites | ✓ (video panel) | ✓ (add-ons; "M3U stream" fix in 6.31) | ✗ | ✗ (lists them only; README limit) | H | M | Hand the manifest to yt-dlp's generic extractor or ffmpeg with cookies and referer |

### C. Organisation and queues

| Feature | IDM | FDM | qBt | reIDM | Value | Effort | How competitors do it |
|---|---|---|---|---|---|---|---|
| User-defined categories (name, extensions, folder, site restriction) | ✓ right-click in the tree to add/edit/delete; restrict to sites (`using_idm.html`) | ✓? | ✓ categories with save paths and subcategories, plus tags (`torrentcategorydialog`, `torrenttagsdialog`) | ~ (7 fixed categories, folder override only) | H | M | |
| "Change folder for category on last selected" (remember last folder) | ✓ Options › Save To | ✓ "Fixed default download folder" vs last [3rd-party] | ✓ save path history (advanced "Save path history length") | ✗ | M | S | |
| Multiple named queues, each with its own concurrency, schedule and completion action | ✓ main + sync + any number (`idm_queues.html`) | ~ (per-download schedule) | ~ (one torrent queue: 3 down / 3 up / 5 total, off by default) | ~ (one scheduled queue; global concurrency) | M–H | M | |
| Queue order / priority (move up/down, top/bottom) | ✓ arrows and drag in the Scheduler | ✓? | ✓ (Queue › top/up/down/bottom) | ✗ | M | S | |
| Periodic synchronization queue (re-download changed files) | ✓ | ✗ | ✗ | ✗ | L | M | |
| Per-download scheduling | ✗ | ✓ Scheduler checkbox in the add dialog, days plus start/end [3rd-party] | ✗ | ✗ | L | S | |
| Move a finished download / "Set location…" | ✓ Properties change location | ✓? | ✓ "Set location…" | ✗ | M | S | |
| Rename a download | ✓ | ✓ | ✓ "Rename…" | ~ (name only at add time) | M | S | |
| Description / comment column | ✓ (link text becomes the description, grabber) | ✗ | ~ (comment) | ✗ | L | S | |
| Watched folder (auto-add `.torrent`, `.txt` of links) | ✗ | ✗ | ✓ "Automatically add torrents from:" with per-folder options (`watchedfolderoptionsdialog.ui`) | ✗ | M | S | |
| Excluded file names (never download matching files) | ✗ | ✗ | ✓ "Excluded file names" | ✗ | L | S | |

### D. Torrents

| Feature | IDM | FDM | qBt | reIDM | Value | Effort | How competitors do it |
|---|---|---|---|---|---|---|---|
| Seeding limits by time and inactive time; action Stop/Remove/Remove+files; Any/All | ✗ | ~ (ratio % of size [3rd-party]) | ✓ Seeding Limits (`optionsdialog.ui`; `GlobalMaxRatio`, `GlobalMaxSeedingMinutes`, `GlobalMaxInactiveSeedingMinutes`, `ShareLimitAction`, `ShareLimitsMode`) | ~ (ratio only, then stop) | M | S | |
| Per-torrent options (DL/UL limit, ratio, save path) | ✗ | ✓ priority control | ✓ `torrentoptionsdialog.ui`, `torrentsharelimitswidget.ui` | ✗ | M | S | librqbit per-torrent `ratelimits` |
| Change file selection / priority after adding | ✗ | ✓ "priority control" | ✓ content tab priorities | ✗ (only at add time) | M | S | `Session::update_only_files` |
| Sequential download / first and last pieces first | ✗ | ✓? | ✓ context menu "Download in sequential order", "Download first and last pieces first" | ✗ | M | M–L | librqbit has no flag; `stream()` gives reader-driven prioritisation |
| Separate torrent queueing limits and slow-torrent exclusion | ✗ | ✓ (traffic modes) | ✓ `MaxActiveDownloads=3`, `MaxActiveUploads=3`, `MaxActiveTorrents=5`, queueing off by default | ~ (shares the global slot count) | M | S | |
| Edit trackers; auto-add trackers (list or URL) | ✗ | ? | ✓ "Edit trackers…", `AddTrackersEnabled`, `AdditionalTrackersURL` | ✗ | M | S | librqbit `AddTorrentOptions.trackers` / session `trackers` |
| Force reannounce / force recheck | ✗ | ? | ✓ context menu | ✗ | M | S | |
| Copy magnet link / info hash; export `.torrent` | ✗ | ? | ✓ Copy › Magnet link / Info hash v1/v2; "Export .torrent…" | ~ (hash shown in Properties) | M | S | |
| Encryption mode (allow / require / disable) | ✗ | ✓? (libtorrent 2.0.11) | ✓ `Encryption=0` (allow) | ✗ (librqbit lacks MSE/PE) | M (ISP throttling) | L (upstream work) | |
| Anonymous mode | ✗ | ? | ✓ `AnonymousModeEnabled=false` | ✗ | L | M | |
| PeX / LSD toggles | ✗ | ? | ✓ DHT/PeX/LSD all on by default | ~ (DHT toggle only) | L | S | librqbit `disable_local_service_discovery` |
| IP filter (`.dat`/`.p2p`), manual bans | ✗ | ? | ✓ "IP Filtering", "Manually banned IP addresses…" | ✗ | L–M | S | librqbit `blocklist_url` / `allowlist_url` |
| Connection limits (global / per torrent / upload slots) | ✗ | ✓ | ✓ 500 / 100 / 20 / 4 (`sessionimpl.cpp:545–548`) | ✗ | M | S | librqbit `peer_limit` |
| Peer protocol TCP / uTP / both | ✗ | ? | ✓ "Peer connection protocol" | ✗ (both, fixed) | L | S | |
| Super seeding | ✗ | ? | ✓ "Super seeding mode" | ✗ | L | L | |
| Torrent creator | ✗ | ? | ✓ `torrentcreatordialog.ui` | ✗ | L | S | `librqbit::create_torrent_file` |
| RSS reader and auto-download rules (smart episode filter) | ✗ | ✗ | ✓ RSS Reader (refresh interval, max articles), Auto Downloader rules, "Smart Episode Filter" (`src/base/rss/*`) | ✗ | M | L | |
| Search engine plugins | ✗ | ✗ | ✓ Python plugins (`src/gui/search`) | ✗ | L (legal and maintenance risk) | L | |
| Add dialog extras: content layout, add stopped, stop condition, top of queue, merge trackers on duplicate | ✗ | ? | ✓ Downloads › "When adding a torrent" | ~ (file pick, now/later) | L–M | S | |
| Default handler for `.torrent` and `magnet:` | n/a | ✓ "Make FDM default torrent client" [3rd-party] | ✓ "File association" | ~ (via the browser only) | M | S–M | |

### E. Remote, browser, site grabber

| Feature | IDM | FDM | qBt | reIDM | Value | Effort | How competitors do it |
|---|---|---|---|---|---|---|---|
| Remote web UI with authentication, bind address, HTTPS, whitelist, ban after failures | ✗ | ✓ remote control from Android | ✓ WebUI: IP/port, UPnP, HTTPS cert/key, user/pass or **API key**, localhost bypass, subnet whitelist, ban after N failures, session timeout, CSRF/clickjacking/Host-header protections, reverse proxy, DynDNS (`optionsdialog.ui` WebUI) | ~ (web view on 127.0.0.1 only; token API) | M | M | |
| Firefox extension | ✓ | ✓ | n/a | ✗ (Chromium only) | H | M | |
| Store-listed extension | ✓ | ✓ | n/a | ✗ (Load unpacked) | H | S–M (process) | |
| Force-capture key (e.g. Ins) as well as prevent key (Alt) | ✓ configurable "Keys" (`functions10.html`) | ✓ Alt bypass [3rd-party] | n/a | ~ (Alt only) | L | S | |
| Site grabber: project templates, login, recreate folder structure, convert links for offline, size filters, scheduled projects | ✓ 4-step wizard (`grabber_wizard.html`) | ✓ (3.9 HTML spider) | n/a | ~ (scan, depth ≤3, types, include/exclude) | L–M | M | |

---

## 2. Implement next: top 25, ranked

Ranked by user value ÷ effort, with "normal user notices it in week one" weighted heavily. **[S]** means the simple Options view; **[A]** means behind an *Advanced settings* toggle (see §3).

**1. Tray icon, close to tray.** Add a notification-area icon to `reidm-gui` (`tray-icon` crate, which works with egui/winit on all three OSes). The menu holds: Show reIDM, Add URL…, Pause all / Resume all, Speed limiter on/off with the current value, a *When done* submenu (#2), and Exit. Exit closes the window **and** stops the service after confirming if anything is active; plain close only hides the window. The icon tooltip shows the total rate and the active count. Settings in Options › General [S]: *Show icon in the notification area* (default on) and *Closing the window keeps reIDM running in the tray* (default on, matching qBt `CloseToTray=true`); *Minimize to tray* (default off) goes under [A]. On the first close, show a one-time toast saying reIDM is still in the tray (qBt keeps `MinimizeToTrayNotified` for this). On Linux without an SNI host, fall back silently to no tray.

**2. "When all downloads are done" actions.** A per-session one-shot choice under Downloads menu › *When done* and in the tray: Do nothing / Exit reIDM / Sleep / Hibernate / Shut down. It is not persisted, as in qBt, so nobody's PC shuts down unexpectedly next week. When the active and queued set (not paused or failed) becomes empty, show a 30-second countdown dialog with *Cancel* (qBt `shutdownconfirmdialog`). Add a separate option per scheduled-queue run in the Scheduler dialog: *When the queue finishes: nothing / exit / sleep / shut down / run program* [S]. *Force applications to close* (IDM) goes under [A]. Seeding torrents do not count as "not done". Implementation: Windows `ExitWindowsEx`/`SetSuspendState`, macOS `pmset`/`osascript`, Linux `systemctl poweroff|suspend|hibernate` through logind. API: `POST /api/when-done {action}`; CLI: `reidm when-done shutdown`.

**3. Keep the computer awake while downloading.** The core service holds a sleep inhibitor while any HTTP download is transferring, released 30 s after the last one stops. [S]: *Keep the computer awake while downloading* (default **on**; IDM-style users expect overnight downloads to finish). [A]: *…also while seeding* (default off, as in qBt). Use `keepawake` or direct calls (`SetThreadExecutionState(ES_SYSTEM_REQUIRED|ES_CONTINUOUS)`, `IOPMAssertionCreateWithName`, `org.freedesktop.login1.Manager.Inhibit "sleep"`). Do it in the service, not the GUI, so it works when the window is closed.

**4. Proxy settings.** Add an Options › **Network** tab. [S]: *Proxy: Use system settings (default) / No proxy / Manual*; Manual shows type (HTTP, HTTPS, SOCKS5), host, port, an optional username and password (stored in the OS keychain, not `settings.json`), and *Bypass for:* with `*.host` wildcards, as IDM does. [A]: *Resolve host names through the proxy* (socks5h), *Use the proxy for torrent peers* (librqbit `ConnectionOptions.proxy_url`, SOCKS5 only; the UI says HTTP proxies don't apply to torrents), *Use the proxy for yt-dlp* (`--proxy`), and *Use the proxy the browser used for this download* (IDM; the extension can read `chrome.proxy.settings`). Rebuild the reqwest client when this changes. Extend `Settings`/`SettingsPatch` and the API.

**5. HTTP authentication and site logins.** On a `401` with `WWW-Authenticate: Basic|Digest`, the download moves to a new status *Needs login*. The GUI shows a credential prompt: user, password, *Remember for this site*. A saved login is keyed by `scheme://host[:port]/path-prefix`, as IDM does per directory, and stored in the keychain. Add *Use authorization* fields to Add URL (IDM `using_idm.html`) and a **Site logins** list in Options › Network [S] (add / edit / remove). Support Basic and Digest (`diqwest` or hand-rolled); NTLM/Negotiate are out of scope at first and should be stated as such. Credentials in `user:pass@host` URLs are moved into the store and stripped from the displayed address. Extension hand-offs that hit `401` go back to the browser as today.

**6. User-defined categories.** Turn the fixed `Category` enum into data: `{name, icon, extensions[], folder, sites[]}`, seeded with today's seven. Right-clicking the category tree gives *New category… / Edit… / Delete* (IDM). The edit dialog has name, file types, folder, and *Only for downloads from these sites* (IDM's site restriction, e.g. `*.googlevideo.com`). Matching order: site rule, then extension, then *General*. Options › Folders keeps showing the per-category folder list [S]. Migrate `category_dirs` on load. The extension's *Download file info* dialog and the in-app Add URL gain a category picker that changes the suggested folder. [S]: *Remember the last folder chosen for each category* (IDM "Change folder for … category on last selected", default off).

**7. Speed modes with a schedule.** Replace the single limiter with three named **speed modes**, *Full* / *Balanced* / *Background* (FDM's High/Medium/Low, qBt's normal/alternative). Each has a download limit, a torrent upload limit and, under [A], max downloads at once and connections per download. Full is unlimited by default; Balanced has none set; Background defaults to 25% of the measured peak speed or 200 KB/s. A status-bar and tray switch flips modes in one click, like qBt's turtle. Under [A]: *Use Background mode on a schedule: From hh:mm To hh:mm, Every day / Weekdays / Weekends* (qBt `BandwidthSchedulerEnabled` UI). The existing per-download limit stays. Keep `speed_limit`/`limit_enabled` as Balanced for compatibility.

**8. Clipboard monitoring.** [S]: *Watch the clipboard for download links* (default **off**, to avoid surprise pop-ups; reIDM's current Add-URL prefill stays on). The GUI polls the clipboard every second (arboard, or a change-count API where available). A newly copied URL whose path ends in an extension on the capture list, or a `magnet:` link, raises a small *Download this link?* toast with **Download** / **Later** / **Dismiss**. The same URL is never re-prompted within a session. Ignore copies from reIDM's own window.

**9. Drag and drop links and files onto the window.** Accept URL, text and file drops (egui `dropped_files` plus raw text via winit). A URL or text containing URLs opens Add URL prefilled; `.torrent` files open Add torrent; `.txt` or `.ef2` files open Import. [A] later: an IDM-style floating drop target (always-on-top small window) toggled from the tray.

**10. OS notifications and the completion window.** The service emits *completed*, *failed (reason)* and *needs login* events. The GUI or tray shows native notifications (`notify-rust`, or `winrt-notification` on Windows) with actions *Open* / *Show in folder*. [S]: *When a download finishes: show the Download complete window / show a notification / nothing* (radio; default notification once the tray exists, since the modal window is IDM-faithful but intrusive). [A]: *Notify when a download fails* (default on), *Play a sound* (default off; IDM Sounds tab reduced to one checkbox per event with the system sound).

**11. Multiple queues.** Generalise the single scheduled queue into named queues `{name, max_at_once, schedule, on_finish_action, retries}`. Two are built in: **Main** (today's global behaviour) and **Scheduled** (today's queue). Users can add more, e.g. "Night", or "Slow site" at 1 at once (the IDM recipe in `idm_scheduler.html`). The tree gains a *Queues* node; downloads can be dragged onto a queue; the right-click menu gets *Move to queue ›*; there is a Q column. The Scheduler dialog gets a queue list on the left (IDM layout) with *Files in the queue* (reorder ↑↓, top, bottom) and *Schedule* tabs. The global *Downloads at the same time* remains a hard cap above per-queue limits. Simple view: the Scheduler shows only the Scheduled queue unless the user has created others.

**12. Queue order and "Move to top".** Even before #11, add Move up / down / top / bottom to the context menu and Ctrl+↑/↓ shortcuts, plus an `order` field persisted in `downloads.json`, so the next queued item to start is user-controlled. API `POST /api/downloads/{id}/move {to: top|up|down|bottom}`. [S]; no setting needed.

**13. Configurable retries and timeouts.** [A] Options › Connection: *Retries per connection* (default 5, 0–99, 0 = forever), *Wait between retries* (default 3 s, exponential up to 60 s), *Give up on a silent server after* (default 60 s), *Connect timeout* (default 20 s). Queue level, as IDM does: *If a download fails, retry it N times at the end of the queue* (default 3, then mark failed so the queue can finish and fire its when-done action). These replace `MAX_RETRIES`, `IDLE_LIMIT` and the 20 s connect timeout in `reidm-core/src/lib.rs`.

**14. Per-site connection exceptions.** [A] Options › Connection › *Site exceptions*: a table of host pattern → max connections per download (1–32) and an optional max simultaneous downloads (IDM's "Exceptions:" plus FDM's "per server"). Also remember automatically: when a host answered 429/503 to extra connections, record its working maximum for 24 h so the next download starts at that number rather than relearning it. Show *Limited by server (n)* in the status column.

**15. Free-space check and status-bar disk space.** Before starting or resuming, check the free space on both the temp folder and destination volumes (`fs4::available_space`). If the remaining bytes exceed the free space, pause with *Not enough disk space on X: (needs 4.2 GB, 1.1 GB free)* instead of failing mid-way, and auto-resume when space frees up (poll every 30 s). The status bar shows the download folder's free space (qBt "Show free disk space"). The 0.6 temp-folder design makes this matter: the cross-drive copy on completion needs the destination space too.

**16. Checksum verification.** Properties gets an **Integrity** row: choose MD5 / SHA-1 / SHA-256 / SHA-512, paste the expected value (or auto-detect it from the clipboard), and press *Verify*; it shows ✓/✗ and the computed hash (FDM's SHA-512 feature). Add URL gets an optional *Expected checksum* field ([S], collapsed under "More options"), verified automatically on completion; a mismatch marks the download *Checksum mismatch* with Retry. Auto-pick up `<file>.sha256` or `SHA256SUMS` next to the URL only under [A]. CLI `reidm add URL --sha256 …`; API field `checksum: {algo, value}`.

**17. Antivirus scan and Mark-of-the-Web.** [S] Options › Downloads: *After a download finishes, scan it with:* None (default) / Windows Defender (`MpCmdRun.exe -Scan -ScanType 3 -File "[File]"`) / Custom program with a `[File]` placeholder (IDM). Run it before the completion notification; a non-zero exit shows a warning badge, never a deletion. Separately and on by default, [A] *Mark downloaded files as coming from the internet*: Windows `Zone.Identifier` ADS (ZoneId=3, HostUrl, ReferrerUrl), and macOS `com.apple.quarantine`. qBt exposes both MOTW and quarantine. This also makes SmartScreen and Gatekeeper protect users the way the browser download would have.

**18. Torrent seeding and queue limits, qBt-style.** Options › Torrents [S]: *Stop seeding when ratio reaches* (existing) **or** *seeding time reaches* (minutes), with *then: stop / remove from list / remove with files*. [A]: *inactive seeding time*, Any/All of the above (qBt `ShareLimitsMode`); *Max active torrent downloads* (default 3) and *Max seeding torrents* (default 3), counted separately from HTTP downloads at the same time; *Don't count slow torrents* (below 2 KB/s for 60 s, qBt defaults); *Max peers per torrent* (default 100) and *in total* (default 500) via librqbit `peer_limit`; and DHT / LSD toggles.

**19. Torrent context-menu essentials.** These are cheap on librqbit: *Choose files…* after adding (`update_only_files`), *Limit this torrent's download/upload…* (per-torrent `ratelimits`), *Copy magnet link* / *Copy info hash*, *Export .torrent…* (persisted `torrent_bytes`), *Edit trackers…* (add/remove; re-add the torrent with `trackers` if live editing isn't available — check upstream), *Force recheck*, *Open in another app*. Under [A]: *Add these trackers to every new torrent* (list, plus an optional URL to a list, qBt `AdditionalTrackersURL`).

**20. Run a program when a download finishes or is added.** [A] Options › Downloads: *Run on finish:* a command template with tokens `%N` name, `%F` full path, `%D` folder, `%U` URL, `%C` category, `%Z` size, `%I` info hash (modelled on qBt `optionsdialog.cpp:732–745`), executed without a shell (argument splitting with quotes, so names aren't injected), and *Run on add*. Per-queue *Run when the queue finishes* is part of #2/#11. The log (#22) records exit codes.

**21. Watched folder.** [A] Options › Downloads: *Automatically add from folder:* a path list. A `.torrent` dropped there is added (then moved to `added/` or deleted, per a checkbox); a `.txt` or `.ef2` is imported. Each folder optionally gets a category/queue and *add as Later*. Uses `notify` crate events plus a 10 s poll fallback (network shares). qBt `watchedfolderoptionsdialog.ui` is the model.

**22. Log viewer and diagnostics.** Help › *Show log*: a ring buffer of the service's `tracing` events at info level and above (start, finish, fail with reason, retries, server limits, torrent errors), filterable by level, with *Copy* and *Save log file…*. [A]: *Write a log file*, keep N days (qBt "Backup the log file after / Delete backups older than"). It shortens the TROUBLESHOOTING loop for non-technical users.

**23. Remote web UI, done safely.** Turn the existing web view (`reidm ui`, 127.0.0.1:61002) into an opt-in remote UI. [A] Options › Remote: *Allow other devices to use reIDM's web page* (default off), listen address (default localhost), port, username + password (argon2-hashed) or a generated API key (qBt has both), *Ban an address after 5 failed logins for 1 hour*, session timeout 1 h, HTTPS with a user-supplied cert/key, and a *Trusted subnets* bypass. Keep the CSRF, Host-header and clickjacking protections qBt enables by default. Show the URL plus a QR code for phones; that is FDM's Android remote use case without an app.

**24. HLS/DASH merging for non-YouTube media.** When the user picks an HLS or DASH stream from the extension's media list, add it as a yt-dlp job (the generic extractor handles `.m3u8` and `.mpd` with `--add-header Referer` and the cookie file, already plumbed for YouTube) and merge with ffmpeg into `.mp4`, instead of downloading the manifest file. The quality picker already lists variants; pass the variant URL. Fall back to the current behaviour with a clear message for DRM. This removes the most visible "Status and limits" item in README.

**25. Translations scaffolding.** Move GUI and extension strings into Fluent (`fluent-bundle`) or `rust-i18n` resource files plus Chrome `_locales/`, add a language picker [S] (default: OS language), and ship English plus whatever the community contributes. It is a large one-off refactor, but IDM, FDM and qBt all treat i18n as table stakes. Do it before the string count grows further.

*Just below the cut:* Firefox extension; per-download *Move to… / Rename*; *Delete to Recycle Bin* (`trash` crate, [S] option default on); RSS auto-downloader; torrent creator; IP filter (`blocklist_url`); "ignore modification time on resume"; download quotas; FTP; protocol encryption (needs upstream librqbit work); statistics window.

---

## 3. UX patterns worth copying

**IDM: few decisions at the moment of download, and defaults that just work.**

- **One dialog at the moment that matters.** The *Download File Info* dialog shows a category guessed from the file type, the folder that implies, *Start download* / *Download later*, and nothing else (`starting.html`). reIDM already copies this in the extension. Copy it into the app's Add URL too: category dropdown, folder follows the category, *Remember* checkbox.
- **Silent mode is a composition of three switches.** "Show start download dialog", "Show download complete dialog", and the progress dialog's start view "Don't show" (`options.html`). Power users get silence without a separate mode. reIDM should expose the same three as one [S] radio group: *Ask me each time / Start silently*.
- **"Start download immediately while showing Download File Info dialog".** IDM starts fetching while the user is still reading the dialog, so the click feels instant. reIDM's probe already fetches byte 1; consider continuing the transfer during the dialog and discarding it on Cancel.
- **Recipes in help, not knobs in the UI.** For a site that limits connections, IDM documents the combination (separate queue at 1 at a time, plus a connection exception) instead of adding a dedicated setting (`idm_scheduler.html`). Learning server limits automatically (#14) makes even that unnecessary.
- **Completion actions live with the schedule,** so they only fire for unattended runs and "do not run if you stopped the queue manually" (`idm_scheduler.html`). That is a good safety rule to copy.
- **Wizards for complex tools.** The site grabber is a 4-step wizard with project **templates** (all pictures, all video, whole site) that pre-fill the later steps, and every step hides extra filters behind "Advanced >>" (`grabber_wizard.html`). reIDM's grabber could gain the same templates as a first dropdown.

**FDM: three speed modes instead of a dozen numbers.**

- **Speed modes** (Low / Medium / High), switchable from the bottom panel; each mode bundles speed limits, connections, per-server connections and simultaneous downloads. The full mode is deliberately unlimitable "by design" (board t=18914), so "fast" never surprises anyone. Users pick an intent, not values (#7).
- **Settings are one scrolling page with sections** (download folder, browser integration, traffic limits, antivirus, BitTorrent, advanced) rather than tabs [3rd-party]. Notifications are on by default; *Start downloading without confirmation* is a single checkbox.
- **FDM 6.29 added a "switch back to the classic interface" option** (blog). Redesigns kept reversible earn trust.
- **A feature-voting page** (`vote.htm`) shows what users ask for: SHA-512 integrity, pausing slow downloads, native Apple Silicon builds.

**qBittorrent: progressive disclosure done in code.**

- **A hard split between Options and Advanced.** The Options dialog holds roughly the 80 settings users understand. The **Advanced** page is a flat two-column table of 80+ libtorrent and qBt knobs, each with units and "(auto)" / "(system default)" / "0 (disabled)" annotations, plus an *Open documentation* link per section (`advancedsettings.cpp`). Copy this exactly: a grid of name | value | unit, a *Reset to default* per row, and a header warning.
- **Sentinel values are labelled in the control.** "0: disabled", "-1 (unlimited)", "0 (permanent lease)" are spelled out, so nobody guesses what 0 means. reIDM's `seed_ratio` (0 = off, <0 = forever) should be shown the same way, and it already uses a dropdown.
- **Safe defaults that need no explanation.** Queueing is off; DHT, PeX and LSD are on; encryption is *Allow*; close-to-tray is on; exit confirmation is on; preallocation is off; recheck on completion is off (`sessionimpl.cpp`, `preferences.cpp`).
- **One-shot actions are not persisted.** "When downloads complete" lives in the Tools menu and resets each session, and a countdown confirmation stands between the user and an unwanted shutdown.
- **Per-item overrides only where needed.** The torrent options dialog and share-limit widget offer *Use global* / *Unlimited* / *Custom* per value (`torrentsharelimitswidget.ui`). That is a good pattern for reIDM's per-download limit too.
- **"Requires restart" is said in the label** ("Resume data storage type (requires restart)"), rather than in a pop-up after saving. reIDM's `torrent_port` and `torrent_dht` (SettingsPatch: "takes effect when the torrent session next starts") should say so in the UI.
- **The status bar is a dashboard:** DHT nodes, free disk space, external IP, and the alt-speed toggle, each optional (Options › Behavior › Status Bar).

### Proposed simple / advanced split for reIDM

- **Simple tabs:** General, Folders, Downloads, Network, Torrents.
- **Advanced:** an *Advanced settings* checkbox at the bottom-left of Options reveals a sixth tab with a qBt-style grid holding every [A] item above plus the knobs in §4. The checkbox is remembered in `gui.json`.
- **A one-line "what this does" caption under each simple option,** as the USER_GUIDE already writes them.

---

## 4. Performance and tuning knobs (low-end vs high-end)

### What competitors expose, with defaults

| Knob | IDM | FDM | qBittorrent (default, source) | reIDM today |
|---|---|---|---|---|
| Connections per download | "Default max. conn. number"; FAQ: 1–2 on dial-up, "16 or 32" on broadband (`functions8.html`); the options page advises about 4–8 (conflicting official advice) | Per mode. FDM 5: 12 / 24 / 48 for Low / Medium / High [3rd-party 2016]; FDM 6 values not verified | n/a (peers: `MaxConnectionsPerTorrent=100`) | 8 (1–32) |
| Connections per server | Exceptions table | "Max connections per server" per mode [3rd-party] | n/a | none (auto back-off) |
| Simultaneous downloads | Per queue | Per mode. FDM 5: 3 / 6 / 12 [3rd-party]; FDM 6 not verified | Queueing off; `MaxActiveDownloads=3`, `MaxActiveUploads=3`, `MaxActiveTorrents=5` | 4 (GUI 1–16, core 1–64) |
| Global peer connections / upload slots | – | ? | `MaxConnections=500`, `MaxUploads=20`, per torrent 100 / 4 | librqbit defaults, not exposed |
| Outgoing connection rate (half-open equivalent) | – | – | "Outgoing connections per second" `ConnectionSpeed=30` | not exposed |
| Disk cache | – | – | `DiskCacheSize=-1` (auto), TTL 60 s (libtorrent 1.x); disk queue 100 MiB on lt ≥2.1, else 1 MiB | OS cache only; writes go through `tokio::fs` |
| Disk I/O mode | – | – | `DiskIOType=Default` (mmap / POSIX / pread), read and write mode "Enable OS cache", coalesce reads & writes on Windows only | n/a |
| I/O and hashing threads | – | – | `AsyncIOThreadsCount=10`, `HashingThreadsCount=1`, `FilePoolSize=100`, checking memory 32 MiB, `MaxActiveCheckingTorrents=1` | librqbit `runtime_worker_threads`, `concurrent_init_limit` not exposed |
| Memory cap | – | – | "Physical memory (RAM) usage limit", 512 MiB (`application.cpp:454`, Windows); process memory priority | none |
| Send buffers | – | – | watermark 500 KiB, low 10 KiB, factor 50%; socket send/receive buffers 0 (system); socket backlog | none |
| Request queue / outstanding requests | – | – | `RequestQueueSize=500`, `MaxOutstandingBlockRequests=2000` | – |
| UI refresh interval | – | – | `RefreshInterval=1500` ms | GUI polls the service (rate averaged over 3 s) |
| Resume-data save interval | continuous | ? | `SaveResumeDataInterval=60` min, statistics every 15 min | progress saved "every few seconds"; torrent state every 5 s |
| Pre-allocation | – | ? | off | off (sparse) |
| Speed-limit overhead / LAN / uTP | – | – | apply to uTP: on; overhead: off; LAN peers: off (`IgnoreLimitsOnLAN=false`) | – |
| Slow item handling | – | "Pause slow downloads" | slow torrents: 2 KiB/s down and up, 60 s | – |
| Smallest piece to split | dynamic | ? | n/a | `MIN_SPLIT=1 MiB` (no split below), `MIN_PIECE=256 KiB` |
| Retries / timeouts | queue retries; 30 s retry loop | ? | stop-tracker timeout 2 s; DNS cache 1200 s | 5 retries, 60 s idle, 20 s connect, 12 s browser probe |
| Peer churn | – | – | peer turnover 4% / 90% cutoff / 300 s | – |

### Recommendation: one performance profile control, with knobs behind Advanced

The simple view gets one control: **Options › Downloads › *Performance*: Light / Balanced (default) / Maximum**. It maps to the values below; selecting a profile writes them, and editing any knob in Advanced switches the label to *Custom*.

| Setting | Light (old laptop, Raspberry Pi, metered or Wi-Fi) | Balanced | Maximum (desktop, gigabit) |
|---|---|---|---|
| Connections per download | 4 | 8 | 16 (32 allowed) |
| Downloads at the same time | 2 | 4 | 8 |
| Write buffer per connection before flush | 64 KiB | 256 KiB | 1 MiB |
| Progress save interval | 10 s | 5 s | 5 s |
| Min piece for dynamic split | 1 MiB | 256 KiB | 256 KiB |
| Torrent peers per torrent / total | 40 / 150 | 100 / 500 (qBt) | 200 / 1000 |
| Active torrents (down / seed) | 1 / 1 | 3 / 3 | 5 / 8 |
| Outgoing peer connects per second | 10 | 30 (qBt) | 60 |
| GUI refresh | 2 s | 1 s | 0.5 s |
| librqbit runtime worker threads | 2 | auto | auto |

**Advanced grid entries to add** (all exist today as constants or librqbit options):

- connections per download, downloads at the same time
- `MIN_SPLIT`, `MIN_PIECE`
- retries, idle timeout, connect timeout
- write buffer size, progress save interval
- torrent `peer_limit` global and per torrent, `concurrent_init_limit`, `runtime_worker_threads`
- DHT bootstrap nodes, disable LSD, disable upload
- bind interface (`bind_device_name` / reqwest `local_address`)
- UPnP on/off
- GUI refresh interval

**Measure before exposing more.** reIDM writes through `tokio::fs` with explicit `flush()` (`lib.rs:1850–1908`). On spinning disks and SMB shares the dominant cost is likely small random writes from 32 interleaved connections, so a coalescing write buffer per segment is the first engine-side tuning to try (qBt's "Coalesce reads & writes" is the analogue). I have not benchmarked this; it is a hypothesis.

---

## Things I could not verify

- **FDM 6 defaults.** No official settings documentation or source exists. The connection and simultaneous-download numbers are from a 2016 FDM 5 blog, and the setting names from a 2020 FDM 6 blog.
- **IDM's download-progress dialog.** The "Options on completion" and "Speed Limiter" tabs, and Exit / Sleep / Hibernate as completion choices, are from knowledge of the IDM 6.x UI. They are not on the official pages fetched; the official FAQ names only "Turn off computer when done" and "Hang up modem when done" in the Scheduler.
- **IDM's speed limiter scope.** It applies **per download** ("total speed can reach the number of downloads multiplied by the set speed", `functions11.html`), unlike reIDM's global limiter.
- **IDM's connection advice conflicts** between the options page (about 4–8) and the FAQ (16–32).
- **Cells marked "?"** in the tables could not be confirmed from primary sources.
- **librqbit gaps.** No protocol encryption, and no sequential-download flag, in librqbit 9.0.1 as read from source. Live tracker editing on a running torrent was not confirmed.
