# Using the official IDM extension with reIDM

**The extension is Tonec's and is not bundled here.** Install it from the Chrome Web Store
(`ngpampappnmepgilojfohadhhmbhlaek`), Edge Add-ons, or Firefox Add-ons as usual.

## Windows

1. Install reIDM (`reIDM-setup-*.exe`). The installer registers the native-messaging host `com.tonec.idm` for Chrome, Edge, Brave and Firefox (HKCU) and starts the daemon at login.
2. Do **not** run the original IDM at the same time (both want port 1001).
3. The unmodified extension connects to `ws://127.0.0.1:1001` or the native host. Nothing else to do.

## Linux / macOS

The shipped extension contains this check (v6.43.1, `background.js`):

```js
if (a && a.os != "win") return Gb(!1);   // runtime.getPlatformInfo().os
```

so the **unmodified extension never starts outside Windows**, and it hard-codes port 1001, which is privileged on Unix.
`reidm patch-extension` writes a *local* copy of an extension **you** unpacked with that check removed and the port changed to 61001:

```sh
# 1. get the extension yourself (CRX download or from your browser profile) and unpack it
unzip -o idm.crx -d idm-src          # unzip warns about the CRX header; that is fine
# 2. patch a copy
reidm patch-extension idm-src idm-patched
# 3. run the daemon, then chrome://extensions -> Developer mode -> Load unpacked -> idm-patched
reidm daemon
```

Patch points are matched by exact string, so a different extension version fails loudly instead of being half-patched.
Because the patched copy has a different extension ID, it talks to the daemon over WebSocket (`:61001`) rather than the native host.
Browser updates may auto-disable unpacked extensions; Firefox needs `about:debugging` temporary loading.

## Security notes

* The WebSocket endpoint only accepts `chrome-extension://`, `moz-extension://` or origin-less (local process) clients; any `http(s)` page origin gets 403. Without this, any website could queue downloads on your machine.
* The daemon binds `127.0.0.1` only.
* Filenames from servers/pages are stripped of path components and reserved characters.
