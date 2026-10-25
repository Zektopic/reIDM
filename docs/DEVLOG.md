# Development log

Why reIDM is the way it is: each significant change, the reasoning behind it, and what was
considered and rejected. Newest first. Add an entry for every change that involves a decision;
link the PR. Keep entries short — the code and the commit say *what*, this says *why*.

Format: `## YYYY-MM-DD — title (version, PR)`, then **Context**, **Decision**, **Alternatives**,
**Evidence/tests**, and **Follow-ups** where useful.

---

## 2026-10-26 — A disk-space pause says its shortfall, and the cross-drive test stops depending on free space (0.8.0, PR #10)

**Context.** CI on the self-hosted Linux runner failed one test, `a_finished_file_moves_across_drives`:
it timed out waiting for *Done*. 0.8.0's free-space floor (200 MiB) can never be met on the
runner's 64 MB `/dev/shm`, which the test uses as the temporary folder to force a
cross-filesystem move, so the download correctly paused for space. Separately, the pause note
said "needs 1 B more" while the size was unknown: it printed the bytes still to write, not the
shortfall against the floor.

**Decision.** The test turns the floor off (`min_free_space_mib: 0`) — it is about moving a
finished file between drives, not about space policy — and its `wait_until` accepts `Paused`, so
a pause fails at once with the note instead of after the 90 s timeout. The engine keeps the
strict floor: silently capping or disabling it on a filesystem smaller than the floor could let
a small mount (a tmpfs, a small container volume) fill to ENOSPC; pausing with a note and the
Advanced setting is the safe, explained behaviour. The note now states the shortfall —
`need + keep - free` — so it reads "needs 100.0 MiB more free space, 100.0 MiB free (reIDM keeps
200.0 MiB free…)". A completed download's temporary drive counts as needing nothing
(`unreserved` is 0), so the final move off it is never blocked by that drive's own floor.

**Alternatives.** Disabling or capping the floor per filesystem (rejected: may fill the mount).
Leaving the note (rejected: it named no actionable number). Deferring the wording to 0.8.1
(rejected: it is one string, and 0.8.0 ships the note). Naming the volume's total capacity to
flag a floor that can never fit (left as a follow-up: it needs another disk call, and the free
and floor figures already show it).

**Evidence/tests.** With a 1 TiB floor the compat test fails in 0.08 s naming the folder, the
free space and the floor; with floor 0 all four compat tests pass. The new unit test
`a_download_short_of_space_is_told_the_shortfall_not_its_own_size` pins the arithmetic (100 MiB
free, 200 MiB floor, size unknown → 100.0 MiB more). The whole workspace's tests pass, and
clippy with CI's 1.99.1 is clean for Linux and Windows.

## 2026-10-21 — Measured on imitated machines (0.8.0)

**Context.** The request was for reIDM to be right on "potatoes" and on high-end PCs.
`tools/profile-test.mjs` runs a release-build service in a `systemd-run --user --scope` with CPU and
memory limits, downloads from a local server whose content is a function of name and offset, and
checks every file byte for byte; one download is killed with SIGKILL part way and must resume
intact. Each imitated machine uses the preset its owner would pick. Development machine: 12 CPUs,
13 GiB, NVMe; nothing else building.

| | potato (half a core, 256 MB) | laptop (two cores, 1 GB) | desktop (no limits) |
|---|---|---|---|
| settings | preset light | preset balanced | preset maximum |
| big 1024 MiB: MiB/s | 427 | 288 | 332 |
| big: CPU s per GiB | 0.8 | 1.0 | 2.2 |
| big: peak memory (resident) MiB | 16.7 | 26.4 | 59.9 |
| crash 512 MiB: transfer lost to SIGKILL, seconds | 2.4 | 4.2 | 3.7 |
| crash: file intact after resume | true | true | true |
| small 300×2 MiB: files/s | 73.2 | 89.5 | 115 |
| small: peak memory (resident) MiB | 23.0 | 35.9 | 88.9 |
| list 20000: seconds to add | 0.9 | 0.4 | 0.4 |
| list: idle CPU % | 0.0 | 0.0 | 0.1 |
| list: memory (resident) MiB | 69.4 | 86.8 | 135 |
| list: one full rows fetch, ms | 55.0 | 54.0 | 67.0 |

**Reading it.**
- *Every crash left an intact file*, and cost 2–4 s of transfer: progress is recorded every 3 s,
  and what arrived after the last record is fetched again.
- *Light on half a core outran Maximum on loopback.* With no real network, more connections only
  add disk writes and CPU (Maximum: 2.2 CPU-s per GiB, 16 connections × 1 MiB buffers; Light: 0.8).
  The presets pay off against real servers that cap each connection, which a local server does
  not imitate; this measures cost, not the gain. So the default stays Balanced.
- *Memory* follows the preset: 17 MB (Light) to 60 MB (Maximum) for one large download; a
  20,000-item list adds 50–75 MB, and costs 0.0–0.1% CPU while idle.
- *Many small files*: 73 (Light) to 115 (Maximum) files a second — downloads at once matter more
  than connections for small files.
**Follow-ups.** A run against a throttled-per-connection server would show what the presets buy;
a run on a spinning disk would show whether 16 interleaved connections need a larger buffer there.

## 2026-10-21 — Phase 2 merged: what the merge itself found (0.8.0)

**Keep awake outside a desktop session.** logind's polkit policy lets only a process in a local
session block *sleep* (`inhibit-block-sleep`: `allow_any=no`); a service started as a systemd
user unit, or over SSH, gets "Access denied" — which is what the engine stream saw and took for a
sandbox quirk. Any process may block *idle* (`allow_any=yes`), and idle is what lets a machine
suspend itself overnight. The service now asks for `sleep:idle`, then `idle`, and logs which.
Checked on the development machine from an SSH session: an idle lock is listed by
`systemd-inhibit --list` while a download runs and is gone after the service stops.

**Download folder.** With no `user-dirs.dirs` (a minimal Linux install) the system names no
Downloads folder and reIDM fell back to the working directory — `/` for a login item; the new
welcome screen said "Downloads go to .". It now falls back to `~/Downloads`.

**Options, looked at.** Screenshots of the new Options showed the downloads-at-once slider's
rail drawn in nearly the panel's colour, with 2–8 squeezed into the first tenth of a 1–64 rail,
and the General tab's last caption cut with no scroll bar visible (egui's floating bars show only
on hover; a solid bar uses the widgets' fill, also nearly invisible). Fixed: a filled,
logarithmic slider; Options takes the main window's spare height; a solid bar in the text colour.

**Browser suite against the merged service.** The 7 refresh checks that had been skipped now run
and pass. Two checks were racy, not wrong: "the browser kept nothing" looked before the worker
had let go of the browser's copy (it does so when reIDM's answer arrives), and the image-page
check counted the worker's own ping to reIDM (tab −1), which 1.7.0 sends more often. The second
now counts only requests from tabs and records the last ones it saw. 100/100, 48/48 and 20/20,
three runs in a row.

## 2026-10-21 — Phase 2, the window: simple Options, tray, when done, notifications, drag and drop, accessibility (0.8.0)

### Splitting app.rs

**Context.** `crates/reidm-gui/src/app.rs` was 3,275 lines in five `impl App` blocks (review
service-and-window §6.1). Every later change in this stream touches it.

**Decision.** A mechanical split into `app/` with one file per part of the window (`mod.rs` keeps
the `App` struct and the frame loop; `menu`, `toolbar`, `sidebar`, `status_bar`, `table`,
`banners`, one file per dialog, `commands` for what is sent to the service, `state` for the
dialogs' state). Done by a script over line ranges that checked every line landed exactly once;
methods used across files became `pub(super)`, nothing else changed. Its own commit, so later
diffs read as changes, not moves.

**Alternatives.** The review's deeper tree (`ui/`, `dialogs/…`) with a `Dialog` trait per window:
more churn in the same commit, and `pub(super)` across two levels of modules gets awkward; the
flat `app/` keeps every part one `use super::*` away from the `App` it draws.

**Evidence.** `cargo test -p reidm-gui` 13/13 and clippy `-D warnings` unchanged after the split.

### New dependencies

**Context.** The tray, OS notifications, screen-reader support and non-blocking file pickers each
need a crate. The `.deb` and CI builds avoid GTK (rfd uses the portal backend for that reason).

**Decision.** `tray-icon` 0.26 with `default-features = false, features = ["ksni"]`: one API on
all three systems, and on Linux it speaks StatusNotifierItem over D-Bus (pure Rust, zbus) instead
of libappindicator/GTK, so the build needs no GTK. `notify-rust` 4.18 (zbus on Linux,
WinRT toasts on Windows, `mac-notification-sys` on macOS). eframe's `accesskit` feature.
`pollster` to wait on `rfd::AsyncFileDialog` from a thread. `zbus` (Linux) and `windows`
(Windows) were already in the tree through those; they are direct dependencies only for two
questions — is there a tray host, and will Windows show a notification at all — and the
sign-in Run key.

**Alternatives.** `ksni` directly on Linux plus `tray-icon` elsewhere: two code paths for the same
menu. `tray-icon`'s default libappindicator backend: needs GTK 3 dev packages at build time and a
GTK main loop at run time. Hand-rolled D-Bus notifications: notify-rust is small and covers the
other two systems.

**Evidence.** Licences of the 60 crates the lockfile gained: MIT, Apache-2.0, MIT OR Apache-2.0,
and Unlicense (ksni). `cargo check -p reidm-gui --target x86_64-pc-windows-gnu` and
`--target aarch64-apple-darwin` (with `CC/AR=true`) both build the new dependencies.

### Options: simple tabs and Advanced settings

**Context.** Phase 2 adds a dozen service settings (presets, retries, timeouts, proxy, keep-awake,
free-space floor). Shown all at once they would bury the five options most people touch. The
competitor review (§3) recommends qBittorrent's split; the spec fixes the tabs.

**Decision.**
- Tabs General / Folders / Downloads / Network / Torrents, a caption under every option, and an
  *Advanced settings* checkbox at the bottom left (in `gui.json`, kept even on Cancel, since it
  is a choice of view) that adds an Advanced tab: a grid of name | value | unit | Default, with
  "requires restart" in the name. The grid is data (`options::ADVANCED`: key, label, unit, hint,
  range/default, getter/setter, and when the row applies), so the dialog draws it in a loop and
  the tests check every row's range and default. Labels have a fixed column (≤ 52 characters,
  tested); the long explanation is the hover text.
- Performance: the window derives the preset from the knobs (`Performance::detect`), so editing a
  knob in Advanced shows *Custom* at once, and putting it back shows the preset again. Choosing a
  preset writes its knobs into the form, so the Advanced tab shows what it means. The patch
  carries `performance` plus whichever knobs changed (harmless duplicates of what the service
  would write).
- Every phase-2 field is `Option` in `model::Settings`; `None` hides the option and its row, and
  `changes()` never sends a `None`. Unknown enum values from a newer service (`performance:
  "eco"`, proxy `mode: "pac"`) deserialize to `Custom` / `System` instead of failing the whole
  settings load (`#[serde(other)]`).
- Proxy password is write-only: the field starts empty; empty = keep (key omitted); typing sends
  it; *Forget it* sends `"password": ""`. A pasted `socks5://host:1080` fills type, host and port.
  Save is disabled with a reason while a manual proxy has no host or port.
- Speed limit moves into Downloads as a switch and a rate. Turning it on sends both
  `limit_enabled` and `speed_limit`, because the shown rate may be the suggested 1 MB/s rather
  than the service's 0 (a limit of 0 limits nothing).
- *Start reIDM when you sign in* (General) is the window's own job: HKCU `…\Run` value `reIDM`
  on Windows (the value `packaging/windows/reidm.iss` writes, so installer and checkbox agree),
  `~/.config/autostart/reidm.desktop` on Linux, `~/Library/LaunchAgents/com.zektopic.reidm.plist`
  on macOS; each runs `reidm-gui --background`.
- Port, DHT, connections and the temporary folder moved to Advanced; the download folder moved to
  Folders. "Ask before each download" has no service field: Downloads says it is in the
  extension's options instead of showing a control that can't work. The language "placeholder"
  is a line of text, not a dead combo box.
- Cancel now restores the window's preferences (the theme applies live, so Cancel used to leave
  it changed).

**Alternatives.** One scrolling page (FDM): harder to scan with five areas. Hiding the
Advanced grid's unsupported rows vs. greying them: hidden, since a 0.7 service can't take them
at all.

**Needs from the service (spec gaps found).**
- Proxy password: the spec says write-only but not what an absent `password` means. The window
  assumes **absent = keep the saved one, `""` = forget it**.
- The window sends the whole `proxy` object on any change (the service "replaces it whole" was
  assumed, as for `category_dirs`).

**Evidence.** 11 new unit tests in `options.rs` (unchanged → empty patch for 0.7 and 0.8 settings;
an older service is sent nothing it doesn't know and shows only 3 Advanced rows; presets ↔
Custom; unknown values load; limiter sends rate; password write-only; manual proxy needs an
address; pasted proxy address; Advanced rows' defaults/ranges/reset; labels fit). 3 tests in
`autostart.rs` (desktop-entry quoting per the XDG spec, plist escaping, idempotent removal).
End to end against a stand-in 0.8 service: choosing *Light* and Save posted exactly
`{"connections":4,"max_concurrent":2,"performance":"light","write_buffer_kib":64}`.

### Tray, close to tray, notifications, When done

**Context.** Competitor review §2 items 1, 2 and 10. Closing the window used to leave a service
running with nothing on screen to show for it. The window must keep reacting (tray menu,
completions, a shutdown countdown) while hidden.

**Decision.**
- *Where the work runs.* eframe 0.36 runs `App::logic` for hidden windows whenever something
  requests a repaint (it paints invisible windows directly every ≥100 ms on request, because
  Windows sends no `RedrawRequested` to them — `eframe/src/native/run.rs`, egui #5229). So
  everything that must go on while hidden — draining the poller, notifications, tray commands,
  When done, close handling, the poll pace — moved from `ui()` to `logic()` (`app/lifecycle.rs`);
  `ui()` only draws. `ViewModel::update` moved too (the tooltip needs counts while hidden); it is a
  key comparison when nothing changed.
- *Tray.* `tray-icon` with the ksni backend. On Linux the window first asks D-Bus whether
  `org.kde.StatusNotifierWatcher` reports `IsStatusNotifierHostRegistered` (on a thread; tray-icon
  itself registers with `assume_sni_available(true)`, so it never fails and a hidden window would
  be unfindable on GNOME without the AppIndicator extension). No host → no tray, normal close,
  and the *Download complete* window stays the default. The icon is created on the window's
  thread (macOS requires it). Menu and click events reach the window through a channel plus
  `request_repaint`; hover/move events are dropped without waking it. Tooltip and menu state are
  pushed only when they change (each change is a D-Bus message on Linux).
- *Closing.* With a tray and *close to tray* on: the window hides (`Visible(false)`); the first
  time, a one-time dialog says reIDM is still in the notification area (`tray_notified`), instead
  of an OS notification that might not show. *Tasks › Exit* closes the window even with a tray
  (the service keeps running, as before); *Quit reIDM* (tray, File) stops the service and asks
  first when downloads are running or waiting. Minimise-to-tray is an Advanced window option.
- *When done.* Downloads › When done and the tray submenu (radio), disabled when `/api/status`
  has no `when_done`. While `fires_at` is set, a countdown dialog with a focused Cancel (Esc
  too) shows, repainting once a second, and the window comes to the front for it even from the
  tray. `exiting: true` closes the window for real.
- *Notifications.* `notify::Watch` (pure) turns snapshots into Finished/Failed events (first look
  primes; a download already done when first seen is not news; each new failure once).
  `notify::messages` (pure) groups more than three at once into one notification. Sending runs on
  a thread (D-Bus/WinRT round trip); an error falls back to the *Download complete* window
  (failures: a message in the window). On Windows, `ToastNotifier::Setting()` is checked first
  for the same AUMID notify-rust uses, because toasts switched off globally "succeed" silently
  (AGENTS.md trap). `on_finish` is `None` until chosen: notification with a tray, dialog without.
  A 0.7 `gui.json` with `notify_complete: false` reads as *Nothing*.

**Alternatives.** Keeping logic in `ui()` and un-hiding via platform calls: eframe already solves
it. An OS notification for the "still in the tray" note: may not show (Windows, no server).
Actions on notifications (*Open*, *Show in folder*): on Linux they need a thread waiting per
notification until it is closed, and some servers never close them; left out.

**Evidence.** Unit tests: `when_done` (countdown text, wire values, unknown action loads),
`notify` (events, grouping), `tray` (menu ids round-trip, tooltip), `prefs` (0.7 migration).
End to end on a private D-Bus session (`dbus-run-session`, a Python stand-in watcher + host +
notification server, `scratchpad/p2gui/sni_host.py`, `drive.py --dbus`): the icon registered as
`org.kde.StatusNotifierItem-<pid>-1`; finishing and failing a download produced
`Download complete | holiday-video-1080p.mp4` and `Download failed | report-2026.pdf / HTTP 404`;
the tray's *When done › Shut down the computer* posted `{"action":"shutdown"}`, the countdown
showed "The computer shuts down in 55 seconds." and Cancel posted `/api/when-done/cancel`; tray
*Quit* with a queued download brought up the confirmation. With `--no-host --no-notifications`:
no icon registered and the *Download complete* window showed.

### Drag and drop, and watching the clipboard

**Context.** Competitor review §2 items 8 and 9.

**Decision.**
- *Drops.* winit delivers dropped **files** only (X11 XDND `text/uri-list` entries that aren't
  `file://` are discarded; Windows accepts only `CF_HDROP`), so "URLs/text → Add URL" is served
  by: a dropped item whose path is itself a link (some platforms), and shortcut files holding a
  link — `.url` (Windows; what dragging a link to the desktop makes), `.webloc` (macOS),
  `.desktop` `Type=Link` (Linux), read only if under 64 KiB. `.torrent` → Add torrent (the first;
  more than one is refused with a message, as the dialog holds one); `.txt`/`.list`/`.ef2` →
  the existing import. `dropped::classify` is pure and tested.
- *Clipboard.* A thread owns one `arboard::Clipboard`, reads text once a second while enabled
  and blocks on a channel while disabled (no polling at all when off, the default), and sends
  the text to the window only when it changed. The window decides (`dropped::ClipboardSeen`,
  pure): the first text after switching on is primed, not offered; a link is offered once per
  session; text the window copied itself (`App::copy_text`, used by Copy address / Copy path) is
  never offered. Only one-token links whose path ends in an extension from
  `reidm_core::grabber::COMMON_TYPES`, or magnet links with `xt=urn:`, qualify. Download/Later
  post to `/api/downloads`; a magnet opens Add torrent. A hidden window is shown without
  `Focus`, so typing elsewhere isn't interrupted.

**Alternatives.** Polling `arboard` in `ui()`: an X11 round trip every frame and a repaint loop
to keep polling. A change-count API (`GetClipboardSequenceNumber`, `NSPasteboard.changeCount`):
cheaper per tick, but per-OS code for a once-a-second read.

**Evidence.** 4 unit tests (classification incl. shortcuts of all three systems; which copied
text qualifies; once-per-session and own copies). End to end under Xvfb with a GTK program
owning the clipboard: copying `https://example.com/pub/new-release.iso` showed the prompt within
2 s, *Download* posted `{"url":…,"later":false}`; two copies in a row showed the first with "and
1 more copied".

### Queue order, checksums, free space

**Context.** Competitor review §2 items 12, 15 and 16; spec "Downloads".

**Decision.**
- *Which downloads take part in moves.* Only unfinished ones (`queue::queue`: not `Done`, by
  `order`). Moving past a finished download changes nothing anyone can see, so "Move up" over a
  finished neighbour would look broken. The window sends the selected unfinished ids in queue
  order to `POST /api/downloads/bulk {"action":"move","to",…}`, always the bulk form.
- *Semantics.* Top/bottom move the selection as a block in its own order; up/down move each
  selected download one place past its unselected neighbour (qBittorrent's queue up/down), which
  for a contiguous block is "the block moves one place" and for a selection already at the end
  is a no-op. `queue::can_move` greys out moves that change nothing.
  **For the service:** the spec says "moving a block keeps its internal order" but not what a
  non-contiguous selection does, nor whether finished downloads count as neighbours; the window
  assumes the above. The mock service used for testing does the same.
- *Feature detection.* `Item.order: Option<u64>` and `Item.checksum_ok: Option<Option<bool>>`
  (a custom deserializer tells *absent* — an older service — from `null` — no checksum given).
  The view model records whether any row has them; the move entries, the # column, *More
  options › Expected checksum* and the *Integrity* row appear only then. Limitation: with an
  empty list nothing can be detected, so the checksum field is hidden until the list has a row.
  **Suggestion for the service:** a `features` list in `/api/status` would remove that guess.
- *Checksum input.* `checksum::parse` accepts `algo:hex`, `ALGO hex`, `algo=hex` or bare hex
  (length → algorithm) and explains mistakes while typing. *Verify* runs on its own thread with a
  one-hour timeout (a large file on a slow disk), not the command thread, so it doesn't hold up
  Pause/Resume queued behind it.
- *Free space.* The download folder's entry from `free_space` (matched by path to
  `default_dir`, else the first), all entries in the tooltip, orange under 1 GB.

**Evidence.** Tests: `queue` (3), `checksum` (3), `model::free_space_line`. Against the mock:
Ctrl+↑ on the 3rd queued download posted the move and the # column showed it 2nd; the status bar
showed "123.5 GB free"; Properties of a finished download showed "Matches the checksum it was
added with", and Verify posted `{"algo":"sha256"}` and showed the value.

### Accessibility (review M11, §5.5)

**Context.** Painted widgets were invisible to screen readers and had no focus indication; three
colours failed WCAG AA; text went down to 10 pt; a Linux portal picker froze the window.

**Decision.**
- eframe `accesskit`. Painted widgets get `Response::widget_info` with a role and a name, and a
  2 px `accent_fg` outline when focused (`app/widgets.rs::accessible`). Rows speak
  "name, state, 36%".
- Colours: a WCAG contrast test over every (text colour × background) pair in both themes, which
  found two more failures than the review (light *ok* on the window background at 4.4:1, and
  white on the dark accent fill — the primary button — at 3.8:1). Fixed by darkening light warn
  and ok, and splitting the dark accent into a fill (#2272CC) and a text colour (#5AA2F0). The
  dark fill is 2.8:1 against the panel, under WCAG's 3:1 for components; acceptable because the
  primary button is identified by its label and position, not its fill.
- 12 pt minimum everywhere (the review's figure). The toolbar became 10 px taller (two caption
  lines of 12 pt) and its padding and search box narrower so it still fits 1080 px.
- Keyboard: list navigation with per-event modifiers (`egui::Event::Key { modifiers }`), because
  `InputState::modifiers` is the state when the frame runs — a Ctrl+↑ pressed and released
  between two frames arrives as a plain ↑ (found with XTest, but also possible with fast
  typists on a busy frame). Clicking a row calls `surrender_focus`, else egui keeps focus on the
  cell and treats the arrows as moves between widgets. Esc closes the topmost dialog in a fixed
  order (the confirmation and countdown handle their own). Shift+F10 / Menu for the context menu
  is left out: the menu opens at the pointer and has no row rectangle to anchor to from the
  keyboard handler.
- Pickers: `picker::Picker<T>` opens `rfd::AsyncFileDialog` on a thread and waits with
  `pollster`; the answer returns with the purpose (`PickFor`) and a repaint. One at a time.

**Evidence.** `theme::tests::text_is_readable_on_every_background`; `view::step` tests. Under Xvfb:
↓ moved the selection, Shift+↓ extended it, Ctrl+↑ posted
`{"action":"move","ids":[2,6],"to":"up"}`, Ctrl+N opened Add URL and Esc closed it.
(The harness needed `XSetInputFocus` on the window: without a window manager nothing gets the
keyboard; right-button XTest clicks don't register either, before or after these changes.)

### First-run welcome

**Decision.** One small window, not a three-step wizard (the brief: "keep it light"): where
downloads go with *Change…* (saved to the service at once), whether the extension is connected
with *Set up the extension…* (turns green live), and *Get started*. `welcomed` defaults to
`true` when a `gui.json` exists without it (a 0.7 user has used reIDM) and `false` only when
there is no `gui.json`. Help › *Welcome…* shows it again. The review's optional test download is
left out.

### Following the merged service (b2bb4a4)

**Context.** The engine/service stream landed with corrections to the spec.

**Decision.**
- *Presets.* The window drops its copy of the preset table and reads `profiles` from
  `GET /api/settings`; presets show only when the service sends both `performance` and all three
  profiles. Because "a knob given in the same patch as a preset wins", a chosen preset is now sent
  **alone**; the owned knobs (`connections`, `max_concurrent`, `write_buffer_kib`) are sent only
  when the result is `custom`. The form keeps `performance` as loaded and as chosen, and
  `knob_changed()` (called by the slider and the Advanced rows that own knobs) works it out
  again — so a service that reports `custom` for numbers that happen to match a preset is shown
  as it reports, until the user changes something.
- *Proxy host* validated like the service (letters, digits, `.-_:[]`), with the reason, before
  Save — the service refuses the whole change otherwise.
- `free_space[].free` may be `null`: `Option<u64>`, skipped (a hard `u64` made the whole list fail
  to parse and the status bar lose its line).
- Rows: `waiting_for_space` → "Waiting for disk space" with the note on hover; a `Failed` whose
  error starts with `checksum mismatch` → "Checksum mismatch". *Verify* is offered for
  `checksum_ok == false` too (the service computes it for those).
- `exiting`, the when-done endpoints, moves and verify matched what the window already did.
  "Countdown only after something has run" needs nothing from the window but words in the guide.

### Measurements

Release builds (`lto`, `opt-level = "s"`), throw-away service on 61101/61102 with a fake HOME
(`scratchpad/bench/measure.sh`, and `p2gui/measure2.sh` which also writes a `gui.json`), 10,000
finished downloads, 20 s to load then 30 s sampled, Xvfb. Session D-Bus hidden from the window
(`env -u DBUS_SESSION_BUS_ADDRESS`) so nothing touches the real desktop, except the tray run,
which used `dbus-run-session` with the stand-in host.

| | window CPU | window RSS |
|---|---|---|
| before (feature/parity-and-polish 42197c4 + split), idle | 0.1 % | 138 MB |
| before, one download at 1 MiB/s | 0.9 % | 96 MB |
| after (p2/gui), idle, no tray host | 0.1 % | 132 MB |
| after, one download at 1 MiB/s | 1.0 % | 90 MB |
| after, idle, watching the clipboard | 0.1 % | 135 MB |
| after, idle, tray icon shown (stand-in SNI host) | 0.2 % | 82 MB |

(Service CPU: 0.0–0.1 % idle, 16–17 % with the 1 MiB/s download, both before and after; the
service binary was the same debug build for both runs.)

### Against the real merged service

**Context.** End-to-end runs of the window against the service built from `p2/gui` after merging
b2bb4a4/27b9b3e: throw-away data folder and HOME, ports 61101/61102, a local web server with
sparse files (`scratchpad/p2gui/drive.py --real=…`).

**Evidence.**
- Limit 1 MiB/s, one at a time, four downloads: rows carried `order` from 2³²; Ctrl+↑ on the
  third waiting download made the service's order `big, third, other, small` (it was `big, other,
  third, small`).
- Options › Downloads › *Light* while the service said `custom` (after `max_concurrent: 1` by
  API): the service ended at `performance: "light"`, connections 4, at once 2, buffer 64.
- A wrong SHA-256: the row read *Checksum mismatch* (red); Properties showed the Integrity row
  with what Resume and Restart do, and Verify. `min_free_space_mib: 1000000`: the new download
  read *Waiting for disk space* with the service's note ("not enough space on …/parts: needs 1 B
  more, 33.7 GiB free (reIDM keeps 976.6 GiB free; …)") on hover.
- Manual proxy: pasting `socks5://127.0.0.1:1080`, user `me`, password `s3cret`, Save → the
  service stored kind `socks5`, host `127.0.0.1`, port 1080, `password_set: true`; reopening
  showed "saved; type to replace it" with *Forget it*; a second Save changing only the host kept
  `password_set: true`.
- *When done › Exit* with a 1 GB download: `fires_at` appeared when it finished, the countdown
  showed, and the window process had exited 80 s later (`exiting`).
- Status bar: `36.2 GB free` from `free_space`.

**Found and fixed.** Typing a proxy address, Save's "why not" text sat in the row of buttons; a
long one (the address check) stretched Options to the full screen width, where it stayed — and the
Save button moved out from under the pointer. The reason now has its own line, always reserved (so
the dialog doesn't jump 10 px when one appears), the dialog has a maximum width, and the address is
judged as Save will send it (after taking apart `scheme://host:port`).

**Found, not mine (for the engine stream).** `reidm_core::default_download_dir()` falls back to
`"."` when `directories` finds no Downloads folder (no `user-dirs.dirs`, as on minimal Linux
installs and in this test's fake HOME), so the service saves into whatever directory it was started
from — for a service the window starts, the window's working directory. Suggest falling back to
`$HOME/Downloads` (creating it) before `"."`.

## 2026-10-21 — Phase 2, engine and service: presets, proxy, queue order, checksums, space, power (0.8.0)

Branch `p2/engine` (commits e9dc348, a3eaa12, d775a3a, 674b5b5).
Spec: docs/design/0.8-phase2.md, corrected in the same branch where building it showed a gap.

### 1. Settings: presets, the old constants, the proxy

**Context.** Phase 2's settings table (presets, write buffer, retries, retry wait, idle and connect
time limits, proxy, keep-awake, space to keep free). The time limits and retries were constants;
the proxy did not exist; `reqwest::Client` was built once.

**Decision.**
- `Settings`/`SettingsPatch` move to `settings.rs`, with `Settings::apply(patch)` doing the whole
  change on a copy (validated first; the client rebuilt if the proxy or a time limit changed) and
  `normalize()` clamping and naming the preset. A preset writes its three knobs; a knob in the
  same patch wins (and so makes it `custom`); a knob changed later makes it `custom`; `custom`
  chosen explicitly stays. Old files read as the preset their numbers match, else `custom`.
  `custom`'s torrent peers are balanced's (100). `GET /api/settings` returns `profiles`.
- The client lives in an `RwLock<reqwest::Client>`; every request takes a clone (an `Arc`), so a
  running download uses the new proxy from its next connection. A proxy reqwest can't use is
  refused with nothing changed; one that fails at load falls back to the system's with a warning
  instead of refusing to start.
- Each run snapshots the settings (`RunCtx.opts`): retries (0 = for ever), the doubling wait
  (first wait, ×2, capped at 60 s or the first wait if longer), the two time limits, the buffer.
- Password: stored in `settings.json` (0600, like the list), never returned (`password_set`);
  a `proxy` without `password` keeps it, `""` removes it. yt-dlp gets `--proxy` through a
  `--config-locations` file written 0600 and deleted after the run — not on the command line,
  which other local users can read. Torrents: SOCKS5 only (librqbit's `ConnectionOptions`),
  next session. Page scans (`/api/grab`) use it too.

**Alternatives.** Passing the proxy to yt-dlp in its environment (`HTTPS_PROXY`): yt-dlp's
handlers don't all honour it for SOCKS. `--proxy` on the command line: leaks the password to `ps`.
Rejecting a bad proxy at load: a typo would stop the service.

**Evidence.** `settings::tests::*` (presets, ranges, retry waits, password kept/removed/never
shown, refused proxies, every proxy mode builds a client, a phase-1 file), engine
`downloads_go_through_the_proxy_when_one_is_set` (the test server answers proxy-form requests: a
file only reachable through it downloads; with `none` it can't), `retries_follow_the_settings`,
`media::tests::the_proxy_reaches_yt_dlp_in_a_private_file…` (content, 0600, removed),
`web::tests::the_proxy_password_is_kept_but_never_shown`.

### 2. Changed ids, and what a long list costs

**Context.** Phase 1's follow-up: with 50,000 downloads and one running, the row cache cloned and
re-serialised the whole list at every revision. Measured again here: 25–27% of a core.

**Decision.**
- `store::Items` wraps the map; `get_mut`, `values_mut`, `insert` and `remove` record ids. The
  guard's drop bumps the revision and appends `(rev, ids)` to a change log *under one mutex*, so a
  reader never sees a revision whose ids aren't recorded yet. The log keeps 65,536 ids; older →
  `None` (look at everything). Progress of running downloads moves the revision without a record,
  so `Manager::changed_since` adds the live ids. `start_queue`/`stop_queue` now find ids first and
  take only those (a `values_mut` over 50k would have marked all of them).
- `RowCache::update(rev, get_many(changed))`; the byte comparison stays, so a named-but-unchanged
  row is not sent.
- What remained was the saver: every 3 s checkpoint scheduled a save of the whole 50k list (~7% of a
  core). Debounced saves now wait up to 25× the last save's duration (2 s minimum, 30 s cap) —
  about 4% of a core at most; urgent saves (adds, removals, status, flush) never wait. Progress is
  still checkpointed durably; a crash can cost up to that much recorded progress (bytes on disk are
  re-fetched, never trusted).

**Alternatives.** A per-item revision in `Item` (makes every row carry engine bookkeeping, and the
cache still scans 50k to compare); a progress file per download (a new format; a bigger change
than this phase); keeping the 2 s debounce (the service then spends its time rewriting the list).

**Evidence.** Release builds (LTO off, 16 codegen units), a throw-away service on 61201/61202, one
download held to 1 MiB/s, a client polling `/api/downloads/rows?since=` every 500 ms
(`scratchpad/p2e-bench/measure.sh`), service CPU over 30 s:

| | before (42197c4) | after |
|---|---|---|
| 50k, one running, polled 2/s | 25.3%, 26.6% | 1.7%, 2.8% |
| 50k, one running, nobody polling | 7.4% | 2.8% |
| 5k, one running, polled 2/s | 2.8% | 1.0% |

With the row change alone (before the save change): 7.7% polled and 7.7% unpolled — i.e. the rows
went from ~18% to ~0, and the rest was saving. Tests: `store::tests::the_record_of_changes…`,
engine `the_list_says_which_downloads_changed`, `rows::tests::only_the_changed_downloads…`, and
the existing 50k web test.

### 3. Queue order

**Decision.** `Item.order` (serde default 0 → assigned on load in id order, from 2³²); new items
get `next_order`. `pump` sorts the waiting ones by `(order, id)`. `queue.rs`: top/bottom place the
selection (in its order) before/after every download; up/down swap positions within the
unfinished downloads plus the chosen ones (finished downloads never wait, so stepping over them is
what the user sees), a block moving together. Positions keep their numbers; only downloads whose
order changes are written (so a move is a few changed rows). Out of room below (after ~4 billion
*top*s) it renumbers.

**Evidence.** `queue::tests::a_block_moves_together…`, engine
`the_queue_starts_the_first_in_its_order_and_downloads_can_be_moved` (max_concurrent 1: the moved
one starts first), web `downloads_move_in_the_queue_one_or_many`, compat
`files_from_0_8_phase_1_load_with_the_new_defaults` (ids 7, 2, 5 → order 2, 5, 7; kept across a
restart).

### 4. Checksums

**Decision.** `Request.checksum {algo, value}` (md5/sha1/sha256/sha512 via `md-5`, `sha1`, `sha2`;
the only new crate is `md-5`, `sha1` was already in the lock file through librqbit). Validated and
lower-cased on add; refused for videos and torrents. Hashed on the blocking pool after
`finalize`; a mismatch sets `checksum_ok = false` and fails the download, **keeping the file**.
Resume of such a download re-hashes instead of downloading (the expected value may have been the
mistake); Restart replaces its own file (`replace` now also covers a checksum failure). `POST
/api/downloads/{id}/verify`.

**Evidence.** engine `a_checksum_is_checked_and_a_mismatch_keeps_the_file` (no new range requests
on resume), `checksum::tests`, web `a_finished_downloads_checksum_is_worked_out_on_demand`.

### 5. Not enough space

**Decision.** `space.rs`: per drive, what is still to be written — the part file's size minus
what is *allocated* (Unix `st_blocks`, so Linux's `fallocate` reservation counts as written;
Windows: minus what is written), plus the whole file on the download folder's drive when it is
another drive (the final copy). Unknown size = 1 byte. Short = still needs > 0 and free < need +
keep. Checked at start, after the probe, and in the checkpoint ticker (HTTP, 3 s); videos and
torrents every 10 s. A shortage stops the run like a Pause (`Live::stop_for_space`), so everything
written is kept and recorded; `launch` turns it into `Paused` + `waiting_for_space` + note. A
30 s task restarts those that fit, in queue order, subtracting each one's need from its drive.
Pause clears the flag (wait for the user); Resume clears it (check now).

**Alternatives.** Pausing whenever free < keep even with nothing left to write: pointless for a
reserved download and confusing ("why did it stop?"). Failing instead of pausing: violates "never
lose a download" in spirit (it would need the user).

**Evidence.** engine `a_download_waits_for_space_and_carries_on_when_there_is_some` and
`running_out_of_space_part_way_keeps_everything_downloaded` (temp on /dev/shm: every counted byte
is in the part file, the resumed file is identical); `disk::tests::free_space_…`.

### 6. Keep-awake and when-done

**Decision.** `power.rs`, behind `Inhibitor` and `PowerActions` traits; the daemon passes the
system ones. One task, once a second (O(1) unless an answer needs the list): hold while anything
transfers (and seeds, if asked), release 30 s after. Linux: `systemd-inhibit --what=sleep
--mode=block … cat` — `cat` reads reIDM's pipe, so if reIDM dies the pipe closes and the lock is
gone; release closes the pipe, waits ≤ 1 s, then kills and reaps (no zombie). macOS: `caffeinate
-i -w <pid>`. Windows: a thread holding `SetThreadExecutionState` (it belongs to the thread). A
refusal is logged once per busy period. Hold/release run on the blocking pool.
When-done: `Countdown` state machine; starts only after something ran since the action was chosen
(spec changed: choosing shut-down with an empty queue must not shut down a minute later), 60 s,
flush, then act once. Exit: the service answers `exiting: true` for 6 s, then shuts down.
Windows actions enable `SeShutdownPrivilege` first; macOS hibernate = `pmset sleepnow`.

**Evidence.** `power::tests` — the state machines; a paused-time test of the whole watcher with a
fake inhibitor and fake actions (held during a transfer, still held 2 s after, released after 30,
sleep fires once after 60, keep_awake off → no hold, exit calls the callback); the child
lifecycle with `cat` standing in for `systemd-inhibit` (alive while held, gone and reaped after,
an immediately failing helper is an error). Nothing ever calls the real power manager in tests.
web `when_done_is_set_shown_and_cancelled`, `exit_when_done_tells_the_window_then_stops`.

### 7. Extension contract (1.7.0)

- `/ext/ping` `refresh_armed: {id, name} | null`.
- `refresh_address` probes and checks before stopping; after stopping it checks again (first bytes
  may have arrived) and resumes the download if that fails.
- `/ext/add`: `refresh_id` refreshes exactly that download or refuses (never adds; the refresh
  stays armed); `no_refresh` skips the armed download (stays armed); `request_id` (≤ 64): a
  `tokio::sync::OnceCell` per id, so a repeat during the first check waits for its answer — the
  answer, refusals included, is kept 10 minutes, at most 4,096 (oldest evicted).
- Without these fields, behaviour is unchanged.

**Evidence.** web `a_refresh_aimed_at_one_download_refreshes_it_or_is_refused`,
`a_new_download_can_say_it_is_not_a_refresh`,
`a_repeated_request_id_gets_the_first_answer_even_while_it_is_still_being_checked` (two
concurrent requests, one probe at the server, same answer; refusals repeated; bound), engine
`a_new_address_is_checked_before_the_download_is_stopped` (still running, still moving).

### 8. Mark-of-the-Web for videos and torrents

yt-dlp's final file (its `after_move` path) and each selected file of a finished torrent are marked
like HTTP downloads (`disk::mark_from_internet`, on the blocking pool). Compile-checked for Windows
and macOS only (no-op on Linux).

### Not done / follow-ups
- The update check's blocking client ignores the proxy.
- A video keeps the space check every 10 s only while yt-dlp runs; its size before the first
  progress line is unknown (counted as one byte).
- Hibernate on macOS is sleep (the system decides).

## 2026-10-21 — Extension 1.7.0: "Use this link to refresh …?", idempotent hand-offs (0.8.0)

Source: docs/design/0.8-phase2.md ("Refresh address (extension M14)", "Idempotent /ext/add
(extension M19)"), and what the October extension review left open (L13, L18).

### Refreshing an address asks first (M14)

**Context.** After *Refresh download address* → "click the link again in the browser", reIDM
tries every link it is handed in the next 180 s against the waiting download. The extension
didn't know, so the user saw an ordinary *Download file info* dialog for what was really a
refresh; pressing Cancel ("it's a duplicate") lost the refresh, and any unrelated download in
that window went through the refresh attempt first.
**Decision.**
- Before each hand-off of an http(s) link without `media` (browser downloads, clicked links, the
  right-click menu and popup, a direct-address video format), the worker pings reIDM. If
  `/ext/ping` reports `refresh_armed: {id, name}` (a bare id is accepted too), the dialog first
  shows **Use this link to refresh *name*?** with **Refresh**, **New download** and **Cancel**,
  titled *Refresh download address*. Address, origin and size stay visible (the size helps decide
  "same file?"); name and folder are hidden, as they are the waiting download's.
- The question is shown **even with the dialog off**: the user armed it moments ago, and which
  link they meant is theirs to say. *New download* then starts at once; with the dialog on it
  turns into the usual form.
- The answer goes to `/ext/add`: Refresh → `refresh_id: <id>`; New download (Start or Later) →
  `no_refresh: true`. "Start all" and the no-window fallback send neither, so reIDM decides as
  before.
- Outcomes: `refreshed` → notification *Address refreshed — <name> continues from this link*, and
  *Just now* lists it under the waiting download's name. Refresh asked but reIDM added a new
  download instead (a service that ignores `refresh_id`) → *Added as a new download — this link is
  not the same file as <name>…*. A refusal of a held browser download → the browser keeps it, and
  the dialog says so, as for any refusal.
- Without the field (reIDM before 0.8) nothing changes: no ping-driven question, no new fields.
**Alternatives.** Using the once-a-minute ping's value (misses most of the 3-minute window).
Skipping the dialog and refreshing silently (the unrelated-download problem stays). Calling
`/api/downloads/{id}/refresh` directly (needs the API token, which the extension can't read).
Asking only when the dialog is on (then the user who switched the dialog off gets the old
guessing behaviour exactly when they armed something).
**Cost.** One localhost GET per hand-off. Negligible next to the hand-off itself, and it keeps the
content scripts' "is reIDM up" status fresh.
**Evidence.** Browser suite check 14 (five checks with the dialog on, two with it off). Against the
current service they print `skipped: service lacks refresh_armed`. Run once against a stand-in
proxy that adds `refresh_armed` to the current service's ping: all 19 checks in that run passed
(the dialog's question and title, Refresh moves the waiting download to the mirror with no new
item, *Just now* says `refreshed`, New download → usual form → new item with the waiting download
untouched and nothing left in the browser, dialog off → question still shown, New download starts
at once).

### Idempotent hand-offs (M19)

**Decision.** `send()` gives every `/ext/add` a `request_id` (`crypto.randomUUID()`, 36 chars) and
reuses it for the launch-then-retry and for the one retry after the 25 s timeout. The retry
against the original address after a redirect is a different request and gets its own id.
`/ext/add-batch` gets none: it has no retry.
**Evidence.** Suite check 15: a stand-in reIDM that never answers the first request; the extension
asks again after 25 s with the same id and takes the second answer. Costs ~25 s per default run.

### L13: the picked format keeps its container; stable-volume audio

**Context.** reIDM names a video download `<stem>.%(ext)s` (media.rs), so the extension's
extension was only ever what the dialog showed; but a WebM video joined with M4A audio becomes an
MKV under `--merge-output-format mp4/webm/mkv`.
**Decision.** Video-only formats get audio of their own container first (`<itag>+bestaudio[ext=webm]`
for WebM, `[ext=m4a]` for MP4), then any audio. "Stable volume" (DRC) audio is asked for as
yt-dlp's `<itag>-drc`, then the plain itag. The content script now passes `drc` to the worker.

### L18: host access stays required

Without host access the extension can read no cookies, so a fresh install would hand reIDM
signed-in downloads it then can't fetch (falling back to the browser), and click interception
couldn't run. Per-site grants make the first download from each site behave differently;
asking for all sites at first run is the same grant with an extra prompt. Documented in
BROWSER_INTEGRATION.md and extension/README.md instead.

### Not done

- **Expected checksum in the dialog.** The dialog has no *More* section to put it in, a checksum
  is rarely known at the moment of the click, and reIDM's own Add dialog, `reidm add --checksum`
  and Properties' *Integrity* row cover it. Revisit if users ask.
- L15 (PDF stays on the click list, as in IDM), as before.

### What the service must do (passed to the engine stream)

`/ext/ping` `refresh_armed`; `/ext/add` `refresh_id` (refuse, never add, when the link isn't the
armed file — a held browser download must stay with the browser) and `no_refresh` (an armed
download that was never probed accepts any link, so "New download" needs an explicit opt-out); a
repeated `request_id` waits for the first request's answer, refusals included.

## 2026-10-20 — Extension 1.6.0: never lose a held download, one dialog, cheap when idle (0.8.0)

Source: the October extension review (H1–H6, M1–M22, L1–L18).

### Held downloads: one storage key each, tagged with the worker that holds them

**Context.** A download held in `onDeterminingFilename` lived only in the worker's memory and in
one `storage.session` object (read-modify-write). The audit asked whether Chrome releases a dead
worker's filename hold (H3.5).
**Evidence.** Measured in Chrome 156 over CDP: after `ServiceWorker.stopAllWorkers`, or
`Target.closeTarget` on the worker, while a download was held and paused, Chrome **releases the
name hold** (the browser names the file), but the download **stays paused** (`in_progress`,
`paused: true`) until something resumes it. An extension reload also released it. So recovery
means "resume", not "restart the download".
**Decision.** Each held download is written to `storage.local` as `reidm-pending:<id>`:
`{url, paused, stage: 'asking'|'taken', instance, at}`. `instance` is a random id for each worker
start. At start-up, every record from another instance is an orphan, however young: if the
download is still `in_progress` with the same URL, it is resumed (`asking`) or cancelled and
erased (`taken`, meaning reIDM had already accepted). `taken` is written before the browser's copy
is cancelled. A dialog whose port disconnects sends a `wake` message, so the next worker starts at
once; the one-minute alarm is the backstop.
**Alternatives.** `storage.session` (cleared by an extension reload, and it shares a 10 MB quota a
page could fill: M4). The audit's 60-second "still in flight" heuristic (it misses orphans if the
next worker stays up). Restarting orphans with `downloads.download` (not needed, per the evidence).
Cost: three `storage.local` writes per download, and each wakes the content scripts'
`storage.onChanged`. Acceptable per download, unlike the per-minute status write (M6).

### One dialog window with a queue; four ways an answer can end

**Context.** Each request opened its own focused popup window at the same place (M3). A window
closed before its page connected left the download held for ever (H3.1). Enter on a focused
Cancel started the download (M1). After a refusal the dialog showed dead buttons (M2).
**Decision.** One window, port `confirm`, with requests queued in the worker ("N more waiting",
**Start all**). Every request ends in exactly one of four answers:
- `start` or `later`: the outcome is shown with `report()`;
- `cancel`: Cancel, Escape, or closing the window while it shows that request;
- `release`: the window closed before it ever showed (nobody saw it, so nobody declined it), or
  nobody answered for 10 minutes. The caller gives the download to the browser.

Closing the window while more requests wait reopens it for them. A refusal shows "reIDM couldn't
take it: … The browser downloads it instead." (or "Nothing was downloaded." for menu, popup and
picker requests) with only Close/Next. Enter acts only from the name field. *Don't ask again*
applies only on Start or Later.
**Alternatives.** Offsetting several windows and capping them (still steals focus). Treating "closed
before shown" as Cancel (it would lose a download the user never saw).
**Evidence.** Browser checks: Enter on Cancel; two at once give one window and 2 records; a refusal
leaves only Close; a window closed at the moment it appears; the worker stopped while two slow
downloads are held (both finish in the browser, intact).

### The live-manifest loop (H4) was real, and much worse than the audit thought

**Evidence.** A test page polled an HLS master and media playlist once a second. In 10 s the
extension fetched them **~9,800 times each** (`Sec-Fetch-Site: none`), against 11 fetches by the
page. The worker's own re-fetch is seen by its own `onBeforeRequest` (tab −1), and `noteManifest`
fetched again, without end.
**Decision.** `noteManifest` ignores tab < 0, and re-fetches a manifest at most once per tab and
address (origin + path, because tokens in the query change on every poll). It skips the fetch when
the hook already supplied the formats. Measured after the fix: 0–2 extension requests for the
whole run.

### Idle cost on weak PCs (M5–M8)

- `onBeforeRequest` for media is filtered in the browser to `*://*/*.<ext>*` for the 15 media
  extensions, and to the types main_frame, sub_frame, media, xmlhttprequest, object and other.
  `onHeadersReceived` is limited to media and xmlhttprequest. Both listeners are removed while
  *Look for video and audio* is off. Checked: a page with 60 images makes 0 listener calls. XHR
  and fetch responses still reach `onHeadersReceived`, because a media type is only known from
  the response.
- `reidm-status` is written only when up, version, media, dir or folders change. Checked: three
  pings produce no write.
- Content script: no 1 s `getBoundingClientRect` timer (the button is positioned on scroll,
  resize, ResizeObserver, `playing` and `mouseenter`); mutations scan only added or changed nodes,
  debounced to 2 s and run in idle time; URLs already sent are not sent again; the format queries
  run only when the picker opens; no button in frames under 200×150. SPA navigation comes from the
  Navigation API's `navigatesuccess` instead of polling. A browser check confirms it reaches the
  isolated world when the page itself calls pushState.
- `inject.js` became a `world: "MAIN"` `document_start` content script registered with
  `chrome.scripting.registerContentScripts`, only while media detection is on (M8 and M9). It now
  runs before page scripts, a page's CSP can't block it, and `web_accessible_resources` is gone,
  so pages can't fetch `chrome-extension://…/inject.js` to detect reIDM. Checked: off means
  `window.fetch` is native; on means hooked; the fetch probe is blocked.
- The MSE hook that reported the page itself as "media" is gone (M10).

### Cookies from the tab's real store (H5)

`chrome.cookies.getAllCookieStores()` maps tab ids to store ids. A download has no tab, so an
incognito download uses the store of an incognito tab. If none can be found, the result is `null`,
meaning **no cookies**, never the profile's. The menus, popup, grabber and picker all honour
*Also in incognito windows*, with an error that names the setting. Checked: a normal tab resolves
to `"0"`; incognito without an incognito tab resolves to `null`. Not exercised: a real incognito
window, because enabling an unpacked extension in incognito needs the profile's protected
preferences.

### YouTube SPA navigation (H6): confirmed and fixed

Confirmed with a fixture: the global `ytInitialPlayerResponse` stays on video A after pushState to
B. Player responses are now kept by `videoDetails.videoId` (from the global,
`/youtubei/v1/player`, and `movie_player.getPlayerResponse()`), and only the one matching the
`v=`, `/shorts/`, `/live/` or `/embed/` id in the address is offered. The page sent is the
canonical `watch?v=<id>`. With no match, the picker shows only **Best**, which is yt-dlp's best.
Checked: A, then B after pushState, then C with no response (empty list, C's title).

### Smaller decisions

- **M19 (timeouts).** reIDM's probe is limited to 12 s, under the 25 s client timeout, so a timeout
  is rare. After a timeout the extension asks again (60 s): reIDM answers a known address with
  `existing` without checking it again, which makes the retry an idempotency check rather than a
  guess. A real idempotency key needs the service (below).
- **M16 (size probe).** The dialog's size probe is a `HEAD` request and is never sent for held
  downloads, whose size the browser already knows. A ranged GET can use up a one-time link.
- **M15 (protection).** Programs (`.exe`, `.msi`, `.dmg`, `.apk`, scripts…) that a page starts
  always get the dialog, with a warning, even with *Ask* off. Downloads Chrome flags
  (`danger` not safe or accepted) stay with the browser. Marking the files reIDM saves
  (Mark-of-the-Web, quarantine) is the service's job (below).
- **M13.** "Download all links" opens the grabber with links to files ticked, instead of queueing
  up to 2,000 addresses, logout links included.
- **L14.** An HLS quality outside YouTube goes to reIDM as `media: {format: 'best'}`, so yt-dlp
  joins the segments instead of reIDM saving a playlist text file.
- **L4.** The click hand-back still replays `a.click()`. Navigating from the worker would drop the
  page as referrer, which download sites check. The dead `handedBack` WeakSet is removed.
- **Not done:** L13 (yt-dlp's output extension: depends on the service's merge flags), L15 (PDF
  stays on the click list, as in IDM), L18 (optional host permissions: a larger change).

### The browser test no longer hangs

A dialog button pressed through CDP `Runtime.evaluate` closes the window, and the evaluation's
answer then never comes. The suite waited for ever. Buttons and keys are now pressed
asynchronously. Every CDP call times out after 30 s and names its expression and session.
`inWorker` retries while a newly started worker finishes evaluating (`reidm is not defined`).
The test profile allows automatic downloads for the test site, which the several-at-once checks
need.

### What the service must change

1. **M14, refresh-address.** Add the armed download to `GET /ext/ping`, for example
   `"refresh_armed": {"id": 12, "name": "file.iso"}` or `null`. The extension can then say "Use
   this link to refresh *file.iso*?" instead of a new-download dialog. In `refresh_address`, probe
   and check the new address **before** `stop_and_wait`, and leave a running download untouched
   when the check fails. Today any unrelated browser download within 180 s stops the armed one.
   The extension already announces `outcome: "refreshed"` ("Address refreshed").
2. **M19.** An idempotency key in `POST /ext/add` (for example `"request_id"`), answered from a
   short-lived cache, so a retry after a client timeout can't add twice.
3. **M15.** Write `Zone.Identifier` (ZoneId=3, ReferrerUrl, HostUrl) on Windows and
   `com.apple.quarantine` on macOS for files reIDM saves from the extension.
4. Optional, M16: a `dry_run` on `/ext/add` returning size and name, so the dialog needs no
   request of its own.

## 2026-10-20 — Service and window: long lists, a thread that never blocks, safer control paths (0.8.0)

### 1. The list as rows, with deltas and 304s (C1, H1, F-PERF)

**Context.** The window fetched `GET /api/downloads` — every field of every download — every 300 ms,
through ureq 3, whose `read_to_string` stops at 10 MB. Reproduced: with 10,000 finished downloads
(8 segments each) the full list is 13.8 MB and the old window shows "Service unavailable" with an
empty list, permanently. At 5,000 it loads, and then costs ~20% of a core while idle.

**Decision.**
- `GET /api/downloads` keeps its shape (web view, CLI, scripts) and gains `ETag: "<boot>-r<revision>"`
  and `304` on `If-None-Match`. Browsers revalidate this automatically, so the web view benefits too.
- New `GET /api/downloads/rows[?since=<cursor>]`: a *row* per download (no segments, no torrent file
  list, no validators/user agent; `connections`/`parts` counts, torrent `files`/`skipped` counts,
  `folder`), and with a cursor only the rows that changed plus removed ids, or `304`.
- The service keeps each row's serialised JSON (`crates/reidm/src/rows.rs`) and decides "changed" by
  comparing bytes. So deltas need nothing from the engine but `Manager::revision()`, which only says
  when looking again is worthwhile. Removals are kept in a ring of 4,096; a caller further behind,
  with a cursor from another run (`boot` is random per run), or none, gets `full: true`.
- `GET /api/downloads/{id}`: the full item, fetched by Properties about once a second while open.
- The window's poller applies deltas to a `Vec<Arc<Item>>` (unchanged rows are shared, not copied),
  and lifts the body limit to 256 MB as a backstop.

**Alternatives.** Raising the ureq limit alone (keeps ~45 MB/s of JSON at 10k while idle); SSE push
(more moving parts, and the 304 path already makes idle free); per-item revisions in the engine
(better — see "Follow-up" — but not available, and the byte comparison works without it).

**Evidence.** `rows::tests::*`, `web::tests::fifty_thousand_downloads_are_served_and_an_unchanged_list_costs_nothing`
(50k items: full list 60 MB, rows 20.5 MB, a one-row delta < 1 KB, unchanged → 304 with no body),
screenshots of old vs new window at 10k/50k (old: "Service unavailable", 0 total; new: 50000 total).

### 2. A window that does nothing while nothing happens (H1, H2, H3)

**Decision.**
- View model (`crates/reidm-gui/src/view.rs`): visible order and all counts rebuilt only when
  `(snapshot rev, view, search, sort, desc)` changes; lower-cased name/URL and category computed once
  per row on arrival. Selection is a `HashSet`. No O(n²) `retain`s; rates tracked only for running
  downloads and seeding torrents.
- No unconditional `request_repaint_after(200ms)`: the poller and the command thread call
  `request_repaint()` when something changed; otherwise the frame asks for one in 30 s (relative
  "Last Try" times), sooner for a toast or a pending channel.
- Adaptive polling (`Pace`): 500 ms while something runs and the window is focused, 1 s idle,
  3 s unfocused, 5 s minimised; a command pokes the poller so its effect shows at once.

**Evidence.** `view::tests` (an unchanged frame does not rebuild; 50k build ≈ 90 ms debug, 1,000
unchanged frames 0.13 ms), `api::tests` (deltas in place, rates, pace), measurements below.

### 3. Nothing blocks the window's thread (H4, H5, M12, L5)

**Decision.**
- One command thread (`worker.rs`) with a queue; jobs run in order and hand back a closure the frame
  runs (toasts, dialog state). Bulk actions are one request (`POST /api/downloads/bulk`, ≤ 10,000 ids)
  instead of a thread and a request per download. Long calls (playlist, torrent inspect, page scan,
  refresh address) keep a thread each, so a 10-minute yt-dlp lookup doesn't hold up *Pause*.
- One token cell shared by poller and commands; a `401` re-reads the file once and retries once; a
  missing token is an immediate error (the 15 s sleep loop is gone from the UI path).
- `ensure_daemon` moved from before the window to the poller; the window opens at once with
  "Starting the download service…". `daemon_up` checks `/ext/ping` says `"app":"reIDM"` instead of a
  bare TCP connect, and stops waiting if the spawned service exits (port taken / already running).
- Dialogs open from the last polled settings instead of a blocking `fetch_settings`.

**Not done.** `rfd::FileDialog` remains synchronous (fine on Windows/macOS; on Linux the portal call
blocks the frame while the picker is open — judged acceptable, `AsyncFileDialog` is a later change).

**Evidence.** `worker::tests` (5,000 commands, one thread, in order), `api::tests::a_restarted_service_s_new_token…`,
`api::tests::no_token_means_an_answer_at_once…` (< 50 ms).

### 4. Options can't overwrite settings it never loaded (H6)

**Decision.** `options.rs`: the form remembers the settings it was opened with; Save sends only the
changed fields (`category_dirs` as a whole when any folder changed, since the API replaces them all),
and is disabled with "Loading the service's settings…" until they have loaded. Slider ranges are the
service's own (connections 1–32, at once 1–64). `POST /api/settings` now answers `500` (not `400`)
when the engine applied the settings but could not save them — detected by comparing settings before
and after, not by matching the message.

**Evidence.** `options::tests::*` (unchanged → empty patch; 32 at-once survives; one folder → all folders).

### 5. Security of the control paths (H8, M1, M2, L1, L2, L4)

**H8 — decision: the WebSocket no longer honours types 1001–1003.** Checked what the real IDM
extension sends: types 0, 1, 2, 13, 14 (and 15+ for media), never ≥ 1000 — those were reIDM's own CLI
additions (`docs/PROTOCOL.md`). So dropping them breaks nothing for Tonec's extension, while the
socket (no token; any local process, any user) could pause, remove and *delete files*. The CLI's
`pause`/`resume`/`rm` use the token-protected HTTP API and now fail for unknown ids. `reidm-host` also
refuses to relay types ≥ 1000 and relays only for Tonec's two Chromium IDs or its Firefox ID.
*Alternative rejected:* requiring the IDM extension's `Origin` — native host and patched copies send
none, and a local process can forge it anyway.

**M1.** `/ext/*` (and its preflight) check `Host` like `/api/*` (`127.0.0.1`, `localhost`, `[::1]`
with the port). A ping marked `Sec-Fetch-Site: cross-site|same-site` is answered but not recorded as
the extension checking in (an `<img>` on a page can't fake "Browser: connected").
*Verified at merge by the browser suite.* The concern was: Chrome sends `Sec-Fetch-Site: none` (or no header) for the
extension service worker's own GET; if it sends `cross-site`, "Browser: connected" would stop showing
(capture is unaffected). I couldn't run `tools/capture-test.mjs` (ports in use by another agent).

**M2.** Per-route `DefaultBodyLimit`: torrent routes `MAX_TORRENT_FILE/3*4 + 1 MiB` (14.3 MiB for the
engine's 10 MiB cap), lists 8 MiB, 64 KiB elsewhere; a readable 413 (JSON on `/ext`). Batches capped at
10,000 with a message. **L1** constant-time compare. **L2** host caller check and 1 MiB reply cap.
**L4** one origin list (`chrome-`, `moz-`, `safari-web-`, `extension://`).

**Evidence.** `web::tests::{the_token_is_checked_whatever_its_length, ext_endpoints_refuse_other_host_names,
a_web_page_cannot_fake_the_extension_checking_in, oversized_requests_are_refused_with_an_explanation,
unknown_downloads_are_reported_not_silently_accepted}`, `session::tests::control_messages_cannot_remove_downloads`,
`reidm-host` `tests::{callers, control_messages_are_not_relayed}`.

### 6. Start-up, handlers, CLI (M5, M7, L3, L8)

- **M7:** a `daemon.lock` (std `File::try_lock`) in the data folder first, then both ports, then the
  list; losing either port is fatal with a message naming it. SIGTERM, Ctrl-C and `POST /api/shutdown`
  stop serving, remove the token file and `Manager::flush()` (logged if it fails).
- **M5:** every engine call that touches the disk or clones the list runs in `spawn_blocking`;
  helpers started by Open/Open folder are waited for on a thread (no zombies).
- **L3:** the `bin/../../extension` fallback only in debug builds; Windows uninstall removes the
  manifests it wrote. **L8:** `list` from the API (numeric order) with file fallback only when
  unreachable, `export` likewise; `get` resumes in a stable per-URL folder and removes it when done;
  `parse_rate` takes `MiB` and treats `8Mbps` as bits. `REIDM_UI_PORT`/`REIDM_LOCAL_PORT` for a
  throw-away second service (tests, measurements); the extension ignores them.

**Evidence.** `tests::a_second_service_for_the_same_folder_is_refused`, `tests::get_resumes_in_the_same_folder`,
`tests::rates`, `web::tests::shutdown_stops_the_service_and_removes_the_token`,
`web::tests::storage_problems_reach_the_status`; manual run: shutdown → token gone, list + `.bak` flushed.

### 7. Window behaviour (M8, M9, L6, L7, L10, 5.3)

Confirmations for Restart (says how much is thrown away, and that a finished file is replaced),
Remove completed (with the count) and deleting files; Enter takes the harmless choice, Esc cancels,
destructive buttons are red. First close explains downloads continue, with *Quit reIDM completely*
(also in File). Limiter "turn on" with no rate opens its dialog. Time Left sorts by remaining time,
Status by Running > Queued > Paused > Failed > Done, the last column is Address and sorts by URL.
Grabber results, Properties' torrent files, the torrent picker and playlist rows use `show_rows`; the
Add dialog's parse and the batch preview are cached; a batch range is checked (≤ 10,000) before
expansion, which also avoids the core M10 overflow from the GUI. Storage warnings
(`Manager::warnings()`, in `/api/status`) show as a banner. Wording: Pause/Resume everywhere, Remove
for list removal, "Ready" / "Background service not running".

### Measurements

Release builds (LTO off, 16 codegen units, same for both), Xvfb 1280×800, a throw-away service on
ports 61101/61102 with a fake `HOME` (no browser registrations touched), lists generated with 8
segments per item. CPU is the share of one core over 30 s after the list had loaded.

| Scenario | Window CPU | Service CPU | Window RSS | Service RSS |
|---|---|---|---|---|
| before, 5k idle | 19.6% | 9.0%¹ | 129 MB | 29 MB |
| before, 10k idle | 7.1% (list fails to load) | 17.6%¹ | 84 MB | 59 MB |
| before, 50k idle | 6.3% (list fails to load) | 53.7%¹ | 83 MB | 158 MB |
| before, 5k, one download running | 19.9% | 9.4%¹ | 174 MB | 88 MB |
| after, 5k idle | 0.1% | 0.0% | 133 MB | 31 MB |
| after, 10k idle | 0.1% | 0.0% | 138 MB | 50 MB |
| after, 50k idle | 0.0% | 0.0% | 193 MB | 156 MB |
| after, 5k, one download running | 1.0% | 1.1% | 133 MB | 38 MB |
| after, 50k, one download running | 1.3% | 11.7% | 163 MB | 218 MB |

¹ The "before" window had to be run against this branch's service (the old one can't move its
port off 61002), at a commit where the revision was still the temporary hash shim — so those service
numbers include an extra list hash per request. The window numbers are the old window's own.

Handler timings (debug build, from the tests): 50k full list 60 MB, rows 20.5 MB, a one-row delta
< 1 KB. View-model rebuild for 50k rows ≈ 90 ms in debug (≈ 10× less in release), unchanged frames free.

### Follow-up

- **Service CPU with a long history and something running** (11.7% at 50k, one download): each
  revision change makes the row cache clone and re-serialise the whole list to find the changed row.
  The engine could expose which ids changed since a revision (or a per-item revision); `RowCache`
  would then touch only those. Everything else in the design stays.
- `rfd::AsyncFileDialog` for the Linux portal; accessibility (M11) and the UX/settings redesign are
  phase 2. `reidm-host`'s `port_open` is still a bare TCP connect.
- ~~Verify the `Sec-Fetch-Site` gate with the real-browser suite~~ — verified at merge: the suite
  now checks that the real extension shows as connected (it does, version 1.6.0).

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
