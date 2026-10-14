# reIDM

A cross-platform (Windows · macOS · Linux) download manager written in Rust that speaks the **IDM browser-integration protocol**,
so the *official* "IDM Integration Module" extension can hand downloads to it.

* Segmented parallel downloads (default 8 connections), resume after restart/crash, retry with back-off
* Browser-download capture via the IDM extension (WebSocket `:1001`/`:61001` or native messaging host `com.tonec.idm`)
* A native desktop window (menu bar, icon toolbar, category tree, sortable download table, status bar) — no browser needed
* **Batch downloads**: paste a list, import a text file, or scan a page and pick
* **Site grabber**: find every file a page links to, filter by type/host/pattern, follow links to a chosen depth
* **Re-adding a link resumes it** rather than starting a second copy
* A queue, so adding a hundred links does not open a hundred connections
* **Our own browser extension** (`extension/`) — takes over downloads, finds media, needs nothing from Tonec
* Update checking against GitHub Releases (`reidm update-check`, or Help → Check for updates)
* CLI: `reidm get|add|import|export|grab|list|pause|resume|rm|daemon|ui|update-check|install-host|patch-extension`
* No telemetry, no licensing logic — see `NOTICE.md`

## The window

**reIDM** (Start Menu / desktop shortcut on Windows, `reIDM.app` on macOS, `reidm-gui` on Linux) is a desktop
front-end for the background service. It starts the service if it is not already running, so downloads continue
after the window is closed.

The same list is also reachable as a local web page via `reidm ui` (`http://127.0.0.1:61002/`), which is handy
over SSH or on a headless box. That page only answers to its own `127.0.0.1` host and needs a per-run token, so
no other website can drive it.

> Status: **0.1, early.** The download engine, the extension handshake/capture and the window are tested
> (see `docs/PROTOCOL.md` → Verification). There is no scheduler, no video/media grabber, no FTP, and no
> authentication prompts.

## Install

Grab a package from the Releases page:

| OS | File | Notes |
|---|---|---|
| Windows | `reIDM-setup-*.exe` | per-user install, registers native host for Chrome/Edge/Brave/Firefox, daemon at login |
| Debian/Ubuntu | `reidm_*_amd64.deb` | `systemctl --user enable --now reidm` |
| macOS | `reIDM-*-universal.dmg` | open it, run **Install reIDM.command**. Unsigned: right-click → Open the first time |

Or: `cargo build --release` and run `target/release/reidm daemon`.

## Using the official extension

Short version: **Windows – works as shipped. Linux/macOS – the vendor extension blocks itself, so you need `reidm patch-extension`.**
Details and the exact reason are in [`docs/EXTENSION.md`](docs/EXTENSION.md).

## CLI

```sh
reidm get https://example.com/big.iso -c 16 -d ~/Downloads
reidm daemon &                       # for the extension
reidm add https://example.com/a.zip
reidm list
```

## Layout

| Path | Purpose |
|---|---|
| `crates/reidm-proto` | `MSG#…;` wire codec |
| `crates/reidm-core` | download engine |
| `crates/reidm` | daemon + CLI |
| `crates/reidm-host` | native-messaging host (`com.tonec.idm`) |
| `decomp/` | Ghidra output of the analysed vendor binaries (**private repo only**, see `NOTICE.md`) |
| `docs/` | protocol, extension, packaging and reverse-engineering notes |
| `extension/` | reIDM's own MV3 browser extension — see its [README](extension/README.md) |
| `tools/` | installer unpacker, bulk-decompile script, icon generator, extension hand-off test |

reIDM is not affiliated with Tonec. Use it only for content you are entitled to download.
