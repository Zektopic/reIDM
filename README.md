# reIDM

A cross-platform (Windows · macOS · Linux) download manager written in Rust that speaks the **IDM browser-integration protocol**,
so the *official* "IDM Integration Module" extension can hand downloads to it.

* Segmented parallel downloads (default 8 connections), resume after restart/crash, retry with back-off
* Browser-download capture via the IDM extension (WebSocket `:1001`/`:61001` or native messaging host `com.tonec.idm`)
* A native desktop window (toolbar, category tree, sortable download table, status bar) — no browser needed
* CLI: `reidm get|add|list|pause|resume|rm|daemon|ui|install-host|patch-extension`
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
| `docs/` | protocol, extension and reverse-engineering notes |
| `tools/` | unpacker, bulk-decompile script, manifest generator |

reIDM is not affiliated with Tonec. Use it only for content you are entitled to download.
