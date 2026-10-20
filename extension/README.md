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
| `defaults.js` | Every setting's default, loaded by the worker, the content scripts and the options page alike. |
| `service-worker.js` | Holds each browser download in `chrome.downloads.onDeterminingFilename`, pauses it, writes it down (`storage.local`, `reidm-pending:<id>`, with the holding worker's id), offers it to reIDM with the cookies for that address from the right cookie store, and only then cancels the browser's copy (or resumes it, if reIDM refused). A worker that starts resumes whatever another worker left held. Also: the dialog queue, the media list, context menus, the popup's data, starting reIDM through native messaging, and a once-a-minute check that reIDM is running. |
| `confirm.html`, `confirm.js` | The *Download file info* dialog: one window, a queue of requests, over a port named `confirm`. |
| `content.js` | Stops plain left clicks on links to listed file types and asks the service worker to hand them over, replaying the click for the browser when reIDM declines. While media detection is on: reports media it sees in the DOM (only added nodes, debounced into idle time) and shows the video button and picker. |
| `inject.js` | Registered by the service worker as a `document_start` content script in the page's own world (`chrome.scripting.registerContentScripts`, `world: "MAIN"`), only while media detection is on. Sees the page's `fetch`/XHR responses — YouTube player data (kept per video id), HLS and DASH manifests — which `chrome.webRequest` can't read. Observes only; never changes a request. It is not a web-accessible resource, so pages can't probe for it. |
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

Every `/ext/add` carries a `request_id` (a random UUID), the same on the one retry after a
timeout, so reIDM can answer a repeat with its first answer. Before each hand-off of an http(s)
link the worker pings reIDM: if `/ext/ping` reports `refresh_armed: {id, name}`, the dialog first
asks "Use this link to refresh *name*?", and the answer goes to `/ext/add` as `refresh_id: <id>`
(Refresh) or `no_refresh: true` (New download). Without the field, nothing is asked or sent.

To start reIDM, it sends `{"cmd":"launch"}` to the native-messaging host `com.zektopic.reidm`
(`reidm-host`), which starts the service and replies once it answers.

### Debugging

Click *service worker* on the extension's card for its console. `globalThis.reidm` exposes
`reidm.status()`, `await reidm.ping()`, `await reidm.launchReidm()`, and `reidm.stats` — how
often the network listeners ran, how many manifests were re-fetched, how often the status was
written. On an idle browser all three should stand still.

Messages from content scripts are limited to `media`, `formats-for-tab`, `capture-link`,
`bypass`, `open-tab` and `download-format`, and what they carry is checked; everything else is
accepted only from the extension's own pages.

The end-to-end test, `tools/capture-test.mjs`, loads this folder into a real Chromium and checks
the whole hand-off — see [docs/BUILDING.md](../docs/BUILDING.md#the-real-browser-tests). Run it
after any change to the capture path.

## Limits that come from the platform

- **DASH streams aren't merged.** An HLS quality goes to reIDM's yt-dlp, which joins the pieces;
  a DASH representation is downloaded as the file it is. DRM-protected streams (Widevine, PlayReady, FairPlay)
  can't be captured by any tool — the keys never reach the page.
- **YouTube's formats are resolved by reIDM, not here.** They carry a `signatureCipher` (or no
  address at all) that only the player's own obfuscated code unscrambles; an extension mustn't
  reimplement that. On YouTube the picker sends the page and a yt-dlp format selector instead
  (`media` in `/ext/add`, built by `ytdlpFormat()` in `service-worker.js`), and reIDM runs
  yt-dlp. A reIDM older than 0.5 says nothing about `media` in `/ext/ping`; the extension then
  refuses rather than have it save the watch page.
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
| `tabs`, `scripting` | per-tab media lists, the grabber, and registering the page hook |
| `storage`, `alarms` | settings, held downloads and recent hand-offs; the once-a-minute check that reIDM is running |
| `notifications` | *Downloading in reIDM*, and why reIDM couldn't take a download |
| host access to all sites | media detection, click interception, the grabber, and (with `cookies`) the cookies of the address handed over |

Host access is not made optional (`optional_host_permissions`, review L18): without it no
cookies can be read, so a fresh install would hand reIDM signed-in downloads it then can't fetch,
and click interception wouldn't run. Asking per site would make the first download from each
site behave differently from the next; asking for all sites at first run is the same grant with
an extra prompt.
