# IDM browser-integration protocol (as implemented by reIDM)

Everything below was **observed** in (a) the official extension *IDM Integration Module 6.43.1*
(`background.js`) and (b) the decompiled `IDMMsgHost.exe` / `IDMNetMon.dll` from `idman643build15.exe`,
then confirmed by running the real extension against reIDM (see "Verification").
Anything marked *(inferred)* was derived from how the extension consumes a field, not from IDM's server code.

## Transports

| Path | How | Evidence |
|---|---|---|
| WebSocket | extension dials `ws://127.0.0.1:1001/?cid=<n>&rnd=<n>` with sub-protocol `plugin.v3.internetdownloadmanager.com`, then `ws://0.1.0.1:1001`; **binary** frames in both directions | `background.js` (`La`, `n.Ba`), `IDMNetMon` decomp (RFC 6455 handshake, `HTTP/1.1 101 ...`) |
| Native messaging | `chrome.runtime.connectNative("com.tonec.idm")` → `IDMMsgHost.exe` (stdio) | `background.js`, host manifests in the installer (`allowed_origins` ngpampappnmepgilojfohadhhmbhlaek / llbjbkhnmlidjebalopleeepgdfgcpec, Firefox `mozilla_cc3@internetdownloadmanager.com`) |
| Host → manager | named pipe `\\.\pipe\IDMNetworkMonitor.<n>`; host prefixes a `MSG#0#17#1#0:9:<pid>;` hello, unescapes JSON strings to UTF-8 and re-escapes replies (`FUN_00402420`, `FUN_00402550`, `FUN_00403530` in `decomp/IDMMsgHost.exe.c`) | decompilation |

Native-messaging payloads are **JSON strings** (one frame per string) inside Chrome's `u32-LE length + JSON` envelope.

## Frame grammar

```
MSG#<id>#<type>#<class>#<flags>[:<arg>]*[,<key>=<value>]*;
value := <int> | <utf8-byte-length>:<bytes>
```

* `flags` bit 0 = sender expects a type‑1 reply carrying the request id.
* Arrays of header lines are sent as one string value (`<total-bytes>:<concatenated text>`).
* For native transport with server revision ≥ 99 the extension may base64 binary arrays as `<len>%<b64>`; reIDM does not use those fields.

## Messages handled by reIDM

| type | dir | meaning | reIDM behaviour |
|---|---|---|---|
| 2 | ext→srv | hello `[113,93,1031,0,16845059,cid,15,manifest_version]`, keys 112 browser, 113 engine, 116 UI language | answers with 3 and 4 |
| 3 | srv→ext | hello ack `[proto=113,0,0,0,dpi=96,manager_version,client_id,0,zoom=100]` | `manager_version` = 6.43 build 15 (`6<<24\|43<<16\|15<<8`), so the extension skips its registration/nag paths *(inferred from `xc()`)* |
| 4 | srv→ext | capture settings. `flags`: 64 auto-capture, 32 sniff UI, 256/512 menu entries, 1024 extended. key 1 = file-type list, key 23 = page-hook names | list in `crates/reidm/src/session.rs` |
| 0 | ext→srv | keep-alive (every 15 s) | type‑1 reply |
| 13, flags&128 | ext→srv | browser started saving a file; keys 6 url, 4 ext, 100 name, 103 size, 127 mime, 50 referrer, 51 cookie, 54 UA | queues it, replies `[id, 3]` ⇒ extension cancels the browser's own download |
| 13, other | ext→srv | passive request sniffing | replies `[id, 0]` (not captured) |
| 14 | ext→srv | explicit "Download with IDM" / link list (15 announces a batch) | queues it |
| 1 | both | reply `[request id, result]`; result 3 = taken over | — |
| 1001–1003 | cli→daemon | **reIDM-only** pause/resume/remove | — |

Not implemented: video/media sniffing (types 7–10, 16–25), per-site rules (keys 9, 11, 18–21), proxy auth,
POST-body replay (key 14/19), the IDM GUI.

## Verification

1. `cargo test` – codec round-trips including a hello frame shaped exactly like the extension's.
2. Real extension (6.43.1, locally patched – see `EXTENSION.md`) loaded in Thorium/Chromium under Xvfb:
   daemon log shows `extension hello (args=[113, 93, 1031, 0, 16845059, 0, 15, 3])`.
3. Navigating that browser to a `.zip` URL: the extension paused the browser download, sent type 13/129, reIDM
   downloaded the file (SHA-256 identical to the server's copy) and the browser saved nothing to `~/Downloads`.
4. A page-origin WebSocket (`Origin: https://evil.example`) receives `403`; extension origins and origin-less local clients get `101`.

Not verified: Windows, macOS, Firefox, Edge, and the native-messaging path with the real extension (only a hand-built frame through `reidm-host`).
