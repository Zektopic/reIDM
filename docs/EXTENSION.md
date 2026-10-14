# Browser integration

reIDM works with **either** of two extensions:

* **reIDM's own extension** (`extension/`, in this repository) — takes over browser downloads, finds
  media on a page, and adds right-click entries. Works on every platform, needs nothing from Tonec,
  and is the recommended path. See [its README](../extension/README.md).
* **Tonec's official "IDM Integration Module"** — but only unmodified on Windows, and only when its
  port is free. The rest of this document is about that path.

You can install both; they do not conflict.

## Using the official IDM extension with reIDM

**The extension is Tonec's and is not bundled here.** Install it from the Chrome Web Store
(`ngpampappnmepgilojfohadhhmbhlaek`), Edge Add-ons, or Firefox Add-ons as usual.

## Windows

1. Install reIDM (`reIDM-setup-*.exe`). The installer registers the native-messaging host `com.tonec.idm` for Chrome, Edge, Brave and Firefox (HKCU) and starts the daemon at login.
2. Do **not** run the original IDM at the same time (both want port 1001).
3. The unmodified extension connects to `ws://127.0.0.1:1001` or the native host. Nothing else to do.

When it hands a download over it *pauses* the browser's copy first and only cancels once reIDM's reply says it took
the URL; a refusal resumes the download. See `PROTOCOL.md` → "The browser hand-off is pause-then-decide".

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
* That check deliberately admits **any** extension origin, not a fixed list of IDs, because the protocol has no way to tell one extension from another — this is the same position the original IDM is in. A malicious extension you have installed can therefore queue downloads. Only install extensions you trust.
* The daemon binds `127.0.0.1` only.
* The control API (the page at `127.0.0.1:61002` and the desktop app) needs a per-run token, which the daemon writes to `api-token` in its data directory with mode `0600`. Any process running as you can read it, so this protects against other *users* and against web pages, not against local malware.
* Filenames from servers/pages are stripped of path components and reserved characters, including the Windows device names (`NUL`, `CON.txt`, `LPT1`, …).

## Known limitations

* If a server answers the first ranged request with `206` but later stops honouring `Range`, the download fails with a clear message rather than silently producing a wrong file. Retrying later, or deleting the row so it starts fresh, is the workaround.
* No resumption validator: reIDM does not store `ETag`/`Last-Modified` or send `If-Range`, so resuming against a URL whose content changed in place can stitch two revisions together. This matters for "latest build" style URLs.
* A daemon that cannot read its own `downloads.json` (for example after a truncated write) refuses to start rather than starting with an empty list, so no download history is silently lost — but there is no in-product recovery; the file has to be repaired or removed by hand.
