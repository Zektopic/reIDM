# reIDM Integration (browser extension)

A Manifest V3 extension for Chromium-based browsers (Chrome, Edge, Brave, Vivaldi, Opera,
Chromium; version 120 or later) that hands the browser's downloads, links and page media to the
**reIDM** download manager. It is reIDM's own and doesn't depend on, or talk to, Tonec's IDM
extension.

**For users:** installing, what it does with your downloads, every setting, and privacy are in
[docs/BROWSER_INTEGRATION.md](../docs/BROWSER_INTEGRATION.md). This file is for working on the
extension.

## Loading it from this folder

1. Open `chrome://extensions` (or `edge://`, `brave://`, `vivaldi://`, `opera://extensions`).
2. Turn on **Developer mode**.
3. **Load unpacked** → this `extension/` folder.

The ID is always `pgeonmdaagbiccimgojpafdjcipcfnlc`: it is derived from the public `key` in
`manifest.json`, so the native-messaging host manifests can name it ahead of time. Keep the key
when packing or publishing, or the ID — and with it the native host's `allowed_origins` — changes.

After editing a file, press the reload arrow on the extension's card. Run reIDM (`reidm daemon`,
or open the window) for it to talk to.

## How it works

| Part | Does |
|---|---|
| `service-worker.js` | Holds each browser download in `chrome.downloads.onDeterminingFilename`, pauses it, offers it to reIDM with the cookies for that address, and only then cancels the browser's copy (or resumes it, if reIDM refused). Also: the media list, context menus, the popup's data, starting reIDM through native messaging, and a once-a-minute check that reIDM is running. |
| `content.js` | Stops plain left clicks on links to listed file types and asks the service worker to hand them over, replaying the click for the browser when reIDM declines. Scans the page for media and answers the grabber. Injects `inject.js`. |
| `inject.js` | Runs in the page's own JavaScript context to see its `fetch`/XHR responses — YouTube player data, HLS and DASH manifests — which `chrome.webRequest` can't read. Observes only; never changes a request. |
| `grabber.html`, `grabber.js` | The resource grabber: everything a page offers, from every frame, with filters, sizes, and batch hand-off. |
| `popup.html`, `popup.js` | The toolbar popup: status, the capture switches, recent hand-offs, the media on this page. |
| `options.html`, `options.js` | Capture rules, file types, excluded sites, the port, media detection. |

The rules for which downloads stay with the browser are in `keepReason()` in
`service-worker.js`; the reasons it returns are what the popup shows under *Just now*.

### Talking to reIDM

Over HTTP to `http://127.0.0.1:61002`: `GET /ext/ping`, `POST /ext/add`, `POST /ext/add-batch`
([docs/API.md](../docs/API.md#browser-extension-endpoints)). Two details matter:

- **Requests come from the service worker, never from a content script.** A content script's
  `fetch` carries the *page's* origin and is subject to CORS; the service worker's POST carries
  `Origin: chrome-extension://<id>`, which reIDM accepts.
- **reIDM authorises these endpoints by origin, not by a token** — the extension can't read
  reIDM's token file. Browsers set `Origin` themselves, a page can't forge an extension's, and
  reIDM answers CORS preflights only for extension origins.

To start reIDM, it sends `{"cmd":"launch"}` to the native-messaging host `com.zektopic.reidm`
(`reidm-host`), which starts the service and replies once it answers.

### Debugging

Click *service worker* on the extension's card for its console. `globalThis.reidm` exposes
`reidm.status()`, `await reidm.ping()` and `await reidm.launchReidm()`.

The end-to-end test, `tools/capture-test.mjs`, loads this folder into a real Chromium and checks
the whole hand-off — see [docs/BUILDING.md](../docs/BUILDING.md#the-real-browser-tests). Run it
after any change to the capture path.

## Limits that come from the platform

- **Streams aren't merged.** reIDM downloads an HLS playlist or a DASH representation as a file;
  it doesn't mux segments into one video. DRM-protected streams (Widevine, PlayReady, FairPlay)
  can't be captured by any tool — the keys never reach the page.
- **Signature-protected formats are listed but greyed out.** Most YouTube formats carry a
  `signatureCipher` that only the player's own obfuscated code unscrambles. An extension
  mustn't reimplement that; making them downloadable needs a resolver on reIDM's side, which
  doesn't exist yet.
- **Addresses built and used inside a Web Worker, WebSocket or WebAssembly player** are outside
  the page hook's reach; `chrome.webRequest` still sees the resulting requests, so most media is
  found anyway.
- **Firefox and Safari** aren't supported: the hold-and-hand-over relies on Chromium's
  `onDeterminingFilename`.

## Permissions

| Permission | Why |
|---|---|
| `downloads` | hold, pause, cancel, erase and resume the browser's downloads |
| `cookies` | the cookies a hand-off needs, read for that one address at that moment |
| `nativeMessaging` | start reIDM when it isn't running |
| `webRequest` | notice media and form submissions (observation only) |
| `contextMenus` | the right-click entries |
| `tabs`, `scripting` | per-tab media lists, the grabber, and collecting a page's links (up to 2,000) |
| `storage`, `alarms` | settings and recent hand-offs; the once-a-minute check that reIDM is running |
| host access to all sites | media detection, click interception, the grabber |
