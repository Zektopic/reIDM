# Using Tonec's IDM extension with reIDM

reIDM works with either of two browser extensions:

- **reIDM's own extension** — in this repository, shipped with every installer, for Chrome, Edge,
  Brave, Vivaldi, Opera and Chromium on every platform. It is the one to use; see
  [Browser integration](BROWSER_INTEGRATION.md).
- **Tonec's official *IDM Integration Module*** — because reIDM speaks the same protocol as
  IDM. Unmodified, it works only on Windows, and only when nothing else holds its port. This page
  is about that path. It is the only option for **Firefox**.

Don't run both in the same browser: each tries to take over the browser's downloads, and they get
in each other's way.

**The extension is Tonec's and is not bundled with reIDM.** Install it from the Chrome Web Store
(`ngpampappnmepgilojfohadhhmbhlaek`), Edge Add-ons or Firefox Add-ons as usual.

## Windows

1. Install reIDM (`reIDM-setup-*.exe`). The installer registers the native-messaging host
   `com.tonec.idm` for Chrome, Edge, Brave and Firefox, and starts reIDM at login.
2. Don't run the original IDM at the same time: both want port 1001, and whichever starts first
   gets it. reIDM says so in its log — *not serving the extension's default port 1001 … another
   download manager may be using it* — and in **Help › Browser integration**.
3. The unmodified extension connects to `ws://127.0.0.1:1001`, or through the native host.
   Nothing else to do.

When the extension hands a download over, it pauses the browser's copy first and cancels it only
once reIDM's reply says it took the address; a refusal resumes the browser's download. See
[PROTOCOL.md](PROTOCOL.md#the-browser-hand-off-is-pause-then-decide).

## macOS and Linux

The shipped extension (version 6.43.1, `background.js`) contains

```js
if (a && a.os != "win") return Gb(!1);   // runtime.getPlatformInfo().os
```

so **it never starts outside Windows**, and it dials port 1001, which is privileged on Unix.
`reidm patch-extension` writes a *local* copy of an extension **you** unpacked, with that check
removed and the port changed to 61001:

```sh
# 1. get the extension yourself (a CRX download, or from your browser profile) and unpack it
unzip -o idm.crx -d idm-src          # unzip warns about the CRX header; that is fine
# 2. patch a copy
reidm patch-extension idm-src idm-patched
# 3. run reIDM, then chrome://extensions › Developer mode › Load unpacked › idm-patched
reidm daemon
```

Patch points are matched by exact string, so a different version of the extension fails loudly
instead of being half-patched. The patched copy has a different extension ID, so it talks to reIDM
over the WebSocket (`:61001`) rather than the native host. Firefox loads an unpacked extension
only temporarily, from `about:debugging`.

## Security notes

- The WebSocket accepts only browser-extension origins (`chrome-extension://`,
  `moz-extension://`) and origin-less local programs; a web page's `http(s)` origin gets `403`.
  Without this, any website could queue downloads on your machine.
- That check admits **any** extension, not a list of IDs: the protocol has no way to tell one
  extension from another — the same position the original IDM is in. A malicious extension you
  have installed could queue downloads. Only install extensions you trust.

The rest of reIDM's security model is in [ARCHITECTURE.md](ARCHITECTURE.md#security-model).
