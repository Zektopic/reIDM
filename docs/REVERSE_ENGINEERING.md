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

## Unknowns / not done

* IDMan.exe and the other ~180 files were **not** analysed beyond bulk decompilation of IDMan.exe; no claim is made about its behaviour. Its decompile (14 MB) is included as raw output only.
* Message types other than those in `PROTOCOL.md` (media grabber, scheduler, site rules) are unmapped.
* Licensing/registration code was not examined and is out of scope.
* Runtime behaviour of the original IDM was never observed (no Windows host / no execution of vendor binaries).

## Reproduce

```sh
python3 -I tools/unpack_idm.py idman643build15.exe out/raw         # split overlay
GHIDRA_HOME=/path/to/ghidra tools/decomp.sh out/raw/126_*.bin decomp/IDMMsgHost.exe.c
python3 -I tools/make_manifest.py out/raw docs/installer-manifest.tsv
```
