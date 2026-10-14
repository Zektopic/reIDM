# Reverse-engineering notes

Target: `https://download.internetdownloadmanager.com/idman643build15.exe` — 12,559,328 bytes,
SHA-256 `a902e77169a3e4d8a88ee2ccaab11d3d43ab209483625a3602438ed4a22bbfd9`, PE32 GUI stub.
Tooling: REA (Ghidra 12.1.4 provider) for interactive analysis, headless Ghidra (`tools/decomp.sh`) for bulk decompilation.

## Findings

| # | Finding | Type | Evidence |
|---|---|---|---|
| 1 | The EXE is a ~93 KB unpacker stub (strings: "Unsuitable structure of Setup File", zlib error text) followed by a 12.4 MB overlay of **192 independently zlib-deflated files** (`78 DA` streams, each preceded by a 24-byte header holding FILETIMEs). | observed | `tools/unpack_idm.py`; list in `installer-manifest.tsv` |
| 2 | Recovered set includes IDMan.exe (6.2 MB), IDMNetMon (x86/x64/ARM64), IDMMsgHost.exe, idmnmcl.dll, shell/IE/Mozilla integration DLLs, 4 WFP/TDI kernel drivers, OpenSSL 1.x DLLs, language files, a 430 KB zip holding the legacy Firefox add-on, and `000` = the installer's file list. | observed | manifest |
| 3 | Native-messaging manifests (`127`,`128`): host name `com.tonec.idm`, Chrome origins `ngpampappnmepgilojfohadhhmbhlaek`, `llbjbkhnmlidjebalopleeepgdfgcpec`, Firefox id `mozilla_cc3@internetdownloadmanager.com`, path `IDMMsgHost.exe`. | observed | `docs/installer-manifest.tsv` rows 127–128 |
| 4 | `IDMMsgHost.exe` is a thin relay: stdio ⇄ overlapped I/O on `\\.\pipe\IDMNetworkMonitor.`; prefixes `MSG#0#17#1#0:9:<pid>;`; JSON-unescapes inbound strings and JSON-escapes outbound ones; if the manager mutex `Local\IDMEventMonitor` is absent and `HKLM\Software\Internet Download Manager\InstallStatus` is 0/3 it launches the manager through COM with `idmcmd://startnetmon/`. | observed (decomp) | `decomp/IDMMsgHost.exe.c` `FUN_00402830`, `FUN_00403530`, `FUN_00403750` |
| 5 | `IDMNetMon` serves the WebSocket on `127.0.0.1:1001` (RFC 6455, sub-protocol `plugin.v3.internetdownloadmanager.com`) and also plain HTTP `/client/<id>` and `/status/` endpoints (CORS `*`). | observed (decomp) | `decomp/IDMNetMon.exe.c` around the `HTTP/1.1 101` response |
| 6 | The extension disables itself unless `getPlatformInfo().os == "win"`. | observed | `background.js` IIFE at end of file |
| 7 | Server reply contents for message types 3/4 are produced inside IDMan.exe (not recovered in detail). reIDM's replies were chosen from how the extension *consumes* them. | inferred | `PROTOCOL.md` |
| 8 | The type-4 settings message's first two arguments are key masks over the extension's key table `va = {16:1, 17:2, 18:4, 45:8, 46:16}` (Shift, Ctrl, Alt, Insert, Delete): `args[0]` is the "force capture" mask (tested in `rb`), `args[1]` the "let the browser keep it" mask (tested in `qb`). IDM's defaults are Insert and Alt. | observed (extension) | `background.js` 6.43.1; `crates/reidm/src/session.rs` |
| 9 | Settings keys the manager doesn't send fall back, in the extension's lookup `Y()`, to a pattern that matches nothing — so omitting site rules is safe. The extension's registration/nag paths compare the manager version against 103430656 and 103431424; 6.43 build 15 (103485184) is above both. | observed (extension) | `background.js` 6.43.1 |
| 10 | IDM's `.ef2` export: `CUrlExporter::ReadFromIEFFile` reads records that open with `<\r\n` and close at `\r\n>\r\n`; the first line is the address, then `referer: `, `cookie: `, `pd: ` and `User-Agent: ` lines, matched by name. `pd: ` holds the request body of a form submission (POST). | observed (decomp, strings) | `decomp/IDMan.exe.c` `FUN_00550480` (its error text names it); `crates/reidm-core/src/lists.rs` |
| 11 | The 6.43.1 extension talks to the manager only through the WebSocket and native messaging; nothing in it calls `IDMNetMon`'s HTTP endpoints (finding 5), which serve IDM's other integrations. | observed | `background.js` 6.43.1 |

## Unknowns / not done

* IDMan.exe and the other ~180 files were **not** analysed beyond bulk decompilation of IDMan.exe and the targeted look at its `.ef2` reader (finding 10); no other claim is made about its behaviour. Its decompile (14 MB) is included as raw output only.
* Message types other than those in `PROTOCOL.md` (media grabber, scheduler, site rules) are unmapped.
* Licensing/registration code was not examined and is out of scope.
* Runtime behaviour of the original IDM was never observed (no Windows host / no execution of vendor binaries).

## Review of reIDM against these findings (0.4.0)

A pass over reIDM's implementation with the decompiled sources side by side. What it confirmed,
and what it found wrong in reIDM:

| | Result |
|---|---|
| Reply to a type-13 "the browser is saving this" frame: `[id, 3]` = taken, anything else = resume the browser's copy | matches; the extension cancels only on 3 |
| Hello ack and manager version (6.43 build 15) | matches; above the nag thresholds (finding 9) |
| Settings key masks | **was wrong**: reIDM sent `0, 0`, which silently disabled IDM's Insert (force) and Alt (keep in browser) keys. Now `8, 4`, with a test |
| `.ef2` import (new in 0.4.0) | **was incomplete** in its first draft: it didn't know `pd: `, so a form download would have been fetched with a GET, saving some other response. Caught before release; such entries are now reported and skipped, and exports never write `pd: ` or cookies |
| Native host starting the manager | reIDM's `reidm-host` starts the daemon when it isn't running, as `IDMMsgHost.exe` does (finding 4). (An earlier summary of this review said the vendor host never starts the manager; finding 4 shows it does.) |
| `IDMNetMon` HTTP mode | not needed for the extension (finding 11); not implemented |

## Reproduce

```sh
python3 -I tools/unpack_idm.py idman643build15.exe out/raw         # split overlay
GHIDRA_HOME=/path/to/ghidra tools/decomp.sh out/raw/126_*.bin decomp/IDMMsgHost.exe.c
python3 -I tools/make_manifest.py out/raw docs/installer-manifest.tsv
```
