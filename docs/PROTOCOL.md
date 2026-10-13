# IDM browser-integration protocol (as implemented by reIDM)

Everything below was **observed** in (a) the official extension *IDM Integration Module 6.43.1*
(`background.js`) and (b) the decompiled `IDMMsgHost.exe` / `IDMNetMon.dll` from `idman643build15.exe`,
then confirmed by running the real extension against reIDM (see "Verification").
Anything marked *(inferred)* was derived from how the extension consumes a field, not from IDM's server code.

Sections marked **vendor** describe the original IDM binaries and are recorded as evidence for why
reIDM behaves the way it does. They are *not* a description of reIDM; the "reIDM" column or the
surrounding text says what this project actually does.

## Transports

| Path | How | Evidence |
|---|---|---|
| WebSocket *(vendor, and reIDM)* | The extension dials `ws://127.0.0.1:1001/?cid=<n>&rnd=<n>` with sub-protocol `plugin.v3.internetdownloadmanager.com`, falling back to `ws://0.1.0.1:1001`; **binary** frames in both directions. reIDM serves the same sub-protocol but binds `127.0.0.1` only — the `0.1.0.1` fallback is *not* served, and the unprivileged port 61001 is used outside Windows (see `docs/EXTENSION.md`). | `background.js` (`La`, `n.Ba`), `IDMNetMon` decomp (RFC 6455 handshake, `HTTP/1.1 101 ...`), `crates/reidm/src/ws.rs` |
| Native messaging *(vendor shape, reIDM reimplements the host)* | Chrome: `chrome.runtime.connectNative("com.tonec.idm")` → a host process on stdio. The vendor host is `IDMMsgHost.exe`; reIDM ships its own `reidm-host`, which is a plain WebSocket relay to the daemon and does **not** implement the vendor's named-pipe protocol. | `background.js`, host manifests in the installer (`allowed_origins` ngpampappnmepgilojfohadhhmbhlaek / llbjbkhnmlidjebalopleeepgdfgcpec, Firefox `mozilla_cc3@internetdownloadmanager.com`), `crates/reidm-host/src/main.rs` |
| Host → manager *(vendor only)* | The vendor's host talks to the manager over the named pipe `\\.\pipe\IDMNetworkMonitor.<n>`, prefixing a `MSG#0#17#1#0:9:<pid>;` hello and JSON-escaping/unescaping payloads (`FUN_00402420`, `FUN_00402550`, `FUN_00403530` in `decomp/IDMMsgHost.exe.c`). **reIDM has no pipe code and never sends a type-17 frame**; this row is background on the original design. | decompilation |

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
| 14 | ext→srv | explicit "Download with IDM" / link list (15 announces a batch) | queues it and replies `[id, 3]` when the request was queued, `[id, 0]` when it was rejected — the extension uses this to decide whether to cancel the browser's own download, so reporting "taken" for a link we could not queue would drop it *(the reply value is reIDM's choice; the vendor's exact reply is not recovered)* |
| 1 | both | reply `[request id, result]`; result 3 = taken over | — |
| 1001–1003 | cli→daemon | **reIDM-only** pause/resume/remove | — |

Not implemented: video/media sniffing (types 7–10, 16–25), per-site rules (keys 9, 11, 18–21), proxy auth,
POST-body replay (key 14/19), the IDM GUI.

## Verification

Reproducible from the tree:

1. `cargo test --workspace` — the codec, including a hello frame shaped exactly like the extension's,
   hostile-length rejection, and byte-length (not character-length) string values.
2. `cargo test -p reidm-core` — the engine against a real in-process HTTP server: segmented and
   resumed downloads are byte-identical, resume from a server that ignores `Range` is not corrupt,
   a lost part file restarts rather than leaving a hole, zero-byte files download, purge deletes the
   file, and concurrent `list`/`resume` does not deadlock.
3. `tools/extension-flow-test.py` — drives a live daemon over its WebSocket with the frames the
   extension actually sends: hello → ack + settings, then a type-13 capture frame → a `[id, 3]`
   "taken" reply, then the file must arrive byte-for-byte. Run against a daemon on Linux and on
   Windows; both pass with `manager_version` = `6<<24|43<<16|15<<8` = `103485184` (0x062B0F00).

Observed once, by hand, and not re-run by the test suite:

4. The real extension (6.43.1, locally patched per `EXTENSION.md`) loaded in Chromium under Xvfb:
   navigating to a `.zip` URL made the extension hand the download over, the browser saved nothing to
   `~/Downloads`, and the file reIDM produced was SHA-256-identical to the server's copy.

Checked directly, both platforms:

5. A page-origin WebSocket (`Origin: https://evil.example`) receives `403`; `chrome-extension://`,
   `moz-extension://` and origin-less local clients are accepted. On Windows the daemon binds the
   extension's default port 1001 when it is free, and logs a clear warning when another program
   (such as an installed IDM) holds it.

Not verified: macOS, Firefox, Edge with the real extension, and the native-messaging path with a real
browser (only `reidm-host` relaying a hand-built frame).
