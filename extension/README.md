# reIDM Integration (browser extension)

A Manifest V3 extension that hands downloads, links and page media to the **reIDM** download
manager. It is entirely reIDM's own — it does not depend on, or talk to, Tonec's IDM extension.

## What it does

| | |
|---|---|
| **Takes over browser downloads** | When Chromium starts saving a file, the extension pauses it, offers the URL to reIDM, and then either cancels the browser's copy (reIDM took it) or resumes it (reIDM did not). |
| **Right-click a link** | *Download with reIDM*. Also works on video, audio and image elements. |
| **Right-click a page** | *Download all links on this page with reIDM* — up to 500 links, duplicates skipped. |
| **Right-click a selection** | *Download the selected link with reIDM*. |
| **Finds media** | Watches network requests for HLS/DASH manifests and media files, plus a page-side scan of `<video>`/`<audio>`/`<source>`, and lists what it found in the popup so you can pick. This works inside cross-origin iframes, where a content script cannot see into the DOM. |
| **Picks a video quality** | A *Download with reIDM* button appears over a playing video and opens a picker listing what the player actually offers — resolution, codec, fps, bitrate and size, best first. Understands YouTube's `streamingData`, HLS `#EXT-X-STREAM-INF` and DASH `<Representation>`. |
| **Answers reIDM's own questions** | The toolbar badge shows how many media URLs were seen on the current tab. |

## Installing

reIDM must be running (`reidm daemon`, or just open the reIDM window).

1. Open `chrome://extensions` (or `brave://extensions`, `edge://extensions`).
2. Turn on **Developer mode**.
3. **Load unpacked** → select this `extension/` directory.

To keep it without developer mode, pack it (**Pack extension**) and install the `.crx`, or publish
it. Note that packing generates a key, which fixes the extension ID — see *Origin* below.

## Options

Open the extension's **Options** page to set the reIDM port (default `61002`), turn capture or
media sniffing off, and list URL prefixes that should never be captured.

## How it talks to reIDM

`http://127.0.0.1:61002/ext/ping` and `/ext/add`. Two details matter:

* **Requests are made from the service worker, never from a content script.** A content script's
  `fetch` carries the *page's* origin and is subject to CORS; the service worker's POST carries
  `Origin: chrome-extension://<id>`, which reIDM accepts.
* **reIDM authorises by origin, not by a token.** A web page cannot get past it: browsers set
  `Origin` themselves, page script cannot forge it, and reIDM only answers a CORS preflight when
  the asking origin is an extension's. `Host` is checked too, so DNS rebinding does not help.
  `Origin` is still not a *credential* — any local process can send that header — but such a
  process can already read reIDM's token file, so this grants it nothing new.

## Honest limitations

These are properties of the browser platform, not of this extension:

* **Very small downloads can slip past.** Chrome starts a sub-megabyte download immediately, and a
  pause arriving a millisecond later may lose the race; the file then lands in the browser's own
  download folder as well as reIDM's. There is no workaround. The extension always pauses *first*
  and resumes on failure, so the failure mode is a duplicate, never a lost file.
* **Stream URLs are not the video.** reIDM can download an `.m3u8` manifest and its segments, but it
  does not mux them into a single playable file, and DRM-protected streams (Widevine, PlayReady,
  FairPlay) cannot be captured by any tool — the keys never reach the page.
* **URLs built entirely in JavaScript may be missed.** The page-side hook watches `fetch` and
  `XMLHttpRequest`, which covers most players, but a URL constructed and used through a WebSocket,
  a Web Worker, or a WebAssembly player is outside its reach. `chrome.webRequest` still sees the
  resulting network requests, so most of these are caught anyway.
* **A page with no extension in the URLs** (a signed CDN link, say) is only found through
  `webRequest` if it looks like media by extension. There is no reliable content sniffing available
  to an extension.
* **Signature-protected formats are listed but greyed out.** Most real YouTube formats carry a
  `signatureCipher` that must be unscrambled with the player's own obfuscated logic. An extension
  must not reimplement that: it changes constantly and it is against YouTube's terms. Those rows are
  shown so you can see they exist, and marked as unavailable rather than failing when clicked.
  Making them downloadable needs a resolver on the reIDM side (a yt-dlp-style component), which is
  not built yet.

## Permissions, and why

| Permission | Why |
|---|---|
| `downloads` | pause / cancel / resume browser downloads |
| `contextMenus` | the right-click entries |
| `webRequest` | observe media requests (observation only — MV3 does not allow blocking for ordinary extensions) |
| `storage` | remember the port and toggles |
| `tabs` | know which tab a media URL belongs to |
| `scripting` | collect a page's links for "download all" |
| `host_permissions: <all_urls>` | required for `webRequest` to see media, and to scan pages. This is the broad permission the Web Store review will question most; it is genuinely needed for media sniffing, and media sniffing can be turned off to avoid it. |

## Files

| File | Role |
|---|---|
| `manifest.json` | MV3 manifest |
| `service-worker.js` | download interception, media sniffing, context menus, messaging |
| `content.js` | page scan; injects the MAIN-world hook |
| `inject.js` | runs in the page's own context to see `fetch`/`XHR` and read manifest bodies |
| `popup.html` / `popup.js` | the media list and manual URL entry |
| `options.html` / `options.js` | port, capture and ignore settings |
