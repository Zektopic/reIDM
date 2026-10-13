# reIDM

A cross-platform (Windows · macOS · Linux) download manager written in Rust that speaks the **IDM browser-integration protocol**,
so the *official* "IDM Integration Module" extension can hand downloads to it.

* Segmented parallel downloads (default 8 connections), resume after restart/crash, retry with back-off
* Browser-download capture via the IDM extension (WebSocket `:1001`/`:61001` or native messaging host `com.tonec.idm`)
* CLI: `reidm get|add|list|pause|resume|rm|daemon|install-host|patch-extension`
* No telemetry, no licensing logic — see `NOTICE.md`

## UI

`reidm ui` (Windows: the **reIDM** Start Menu/desktop shortcut, `reidm-gui.exe`) starts the daemon if needed and opens
`http://127.0.0.1:61002/` — add URLs, see progress/speed/ETA, pause, resume, retry, open, show in folder, remove.
The page only answers to its own `127.0.0.1` host with a per-run token, so other websites cannot drive it.

> Status: **0.1, early.** Download engine and extension handshake/capture are tested (see `docs/PROTOCOL.md` → Verification).
> There is no native GUI (the UI is a local web page), no scheduler, no video/media grabber, no FTP, no authentication prompts.

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
