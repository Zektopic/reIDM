# Browser integration

reIDM's browser extension makes reIDM — not the browser — download the files you click. This
page covers installing it, exactly what it does with a download, every capture setting, media
and the resource grabber, and what to check when it doesn't connect.

It works in Chromium-based browsers: **Google Chrome, Microsoft Edge, Brave, Vivaldi, Opera and
Chromium**, on Windows, macOS and Linux alike. (Tonec's own "IDM Integration Module" can also
talk to reIDM — see [EXTENSION.md](EXTENSION.md) — but reIDM's extension is the one to use.)

## Install

reIDM ships the extension with every installer and keeps a copy of it in a stable folder in your
profile, which is the folder to load:

| System | Folder |
|---|---|
| Windows | `%APPDATA%\Zektopic\reIDM\data\browser-extension` |
| macOS | `~/Library/Application Support/com.Zektopic.reIDM/browser-extension` |
| Linux | `~/.local/share/reidm/browser-extension` |

You don't have to remember it: **Help › Browser integration** in the reIDM window shows it with
*Open the folder* and *Copy the path* buttons, and `reidm extension` prints it.

Then, once per browser:

1. Open the extensions page — `chrome://extensions`, `edge://extensions`, `brave://extensions`,
   `vivaldi://extensions` or `opera://extensions`.
2. Turn on **Developer mode** (top right; bottom left in Edge).
3. Choose **Load unpacked** and select the folder.

The extension's ID is always `pgeonmdaagbiccimgojpafdjcipcfnlc`. Within a minute reIDM's status
bar changes from *Browser: extension not connected* to *Browser: Chrome 1xx* (or Edge, Brave…).

**Updates are automatic.** reIDM refreshes that folder whenever it starts, so after you update reIDM
the browser picks up the new extension the next time it starts — no reloading by hand.

**Without the installer** (a source build, or just the zip from the Releases page): unzip
`reidm-extension-<version>.zip` anywhere and load that folder instead; or load `extension/` from a
checkout of this repository.

## What happens when you download

There are two ways a download reaches reIDM, and they are layered so that nothing slips through.

### 1. File links are taken on click

When you click a link whose address ends in a file type on the capture list (`.zip`, `.iso`,
`.mp4`, `.exe` … — see [Capture rules](#capture-rules)), the extension stops the click and hands
the address to reIDM. **The browser never requests the file.** reIDM first fetches a single byte
to make sure it can download it with your cookies:

- If it can, the download starts in reIDM.
- If the address turns out to be a web page — a login wall, a "your download will start shortly"
  page — or reIDM can't reach it, the click is handed back and the browser does exactly what it
  would have done.

Only plain left clicks are taken. Ctrl/Shift/Cmd-clicks (new tab or window), clicks the page
handles itself (single-page apps, script-built links), and **Alt+click** go to the browser.

### 2. Every other download is taken before the browser saves it

Plenty of downloads can't be recognised from the link: `/download?id=123`, a button that runs a
script, a redirect to a CDN. Those reach the browser's own downloader, and the extension takes
them from there — before anything is saved:

1. Chrome decides a download's file name before it shows the download, opens a *Save as* dialog,
   or writes the file. The extension holds that decision.
2. It pauses the transfer and asks reIDM to take the download, sending the final address, the
   referring page, your cookies for that site, and the browser's user agent.
3. reIDM fetches the first byte to prove it can download it.
4. **Only then** is the browser's copy cancelled and removed from its download list. You see no
   download bubble, no *Save as* dialog, and no partial file.
5. If reIDM can't take it — it isn't running and can't be started, the site refuses it, it's a
   one-time link the browser has already used — the download is resumed and released exactly as
   it was. **The worst case is "the browser downloads it", never "nobody does".**

This holds for files of any size — even a few kilobytes — and with Chrome's *Ask where to save
each file* turned on.

### When reIDM isn't running

The extension starts it. reIDM registers a small helper with your browsers
(`com.zektopic.reidm`, the native-messaging host) that the extension uses to launch the reIDM
service and wait for it. The installers register it; reIDM also re-registers it every time it
starts, for every Chromium-based browser it finds, which keeps it right after reIDM is moved or
updated. If the helper is missing, downloads simply stay with the browser until you open reIDM —
the popup's **Start reIDM** button tries the helper too.

### What always stays with the browser

| | Why |
|---|---|
| Form submissions (`POST`) that return a file | reIDM would have to repeat the form, which the server would answer differently or not at all |
| Files a page builds itself (`blob:`, `data:`) | they exist only inside the page |
| Downloads started by other extensions | they asked for the browser's downloader specifically |
| Alt+click | IDM's convention for "let the browser have this one" |
| Sites on your never-capture list | your choice |
| Incognito windows, unless enabled | reIDM keeps a history of what it downloaded |
| Anything smaller than the minimum size, if you set one | your choice |

## The toolbar popup

Click the reIDM icon in the toolbar to see:

- whether reIDM is running (with **Start reIDM** if it isn't);
- **Take over browser downloads** — the master switch — and **Never on *this site***;
- **Just now** — the last downloads handed over, and for any the browser kept, the reason;
- **Media on this page** — video and audio the extension has seen, with type and size;
- **Grab resources…** — the resource grabber for the current page;
- a box to paste any link.

The badge counts the media found on the current tab. It briefly shows ✓ when a download was
handed over, = when reIDM already had the file, and ! when the browser kept one.

## Right-click menu

| On | Entry |
|---|---|
| a link | **Download with reIDM** |
| an image, video or audio | **Download this media with reIDM** |
| selected text that is a URL | **Download the selected link with reIDM** |
| the page | **Download all links on this page with reIDM** (up to 2,000, in one batch) |
| the page or a frame | **Grab resources from this page with reIDM…** |

## Video and audio

The extension watches what the page loads — by address (`.m3u8`, `.mpd`, `.mp4`, `.webm`, …) and
by the type the server reports (`video/*`, `audio/*`, HLS and DASH manifests) — and ignores the
small pieces a stream is made of. Manifests are fetched and parsed so that each **quality** is
listed separately.

When a video plays, a **Download with reIDM** button appears over it. It opens a list of what the
player actually offers — resolution, codec, frame rate, bitrate and size, best first, audio-only
tracks last. It understands YouTube's player data, HLS master playlists and DASH manifests.

Limits that come from the platforms, not from the extension:

- reIDM downloads the stream file you pick (a playlist or a single representation); it does not
  merge HLS/DASH segments into one video file.
- Most YouTube formats are signature-protected. They're listed but greyed out: unscrambling them
  needs the player's own code, which an extension must not reimplement.
- DRM-protected streams (Widevine, PlayReady, FairPlay) can't be captured by any tool.

## The resource grabber

**Grab resources…** (popup or right-click) opens a page listing everything the current page
offers, from every frame:

- links, with their text;
- images — the largest `srcset` candidate, lazy-loading attributes (`data-src` and friends), CSS
  background images, and social-card images;
- video and audio sources, posters and subtitle tracks;
- embedded objects and frames;
- **everything the page actually loaded** — which is how addresses built by scripts show up;
- what the extension saw on the network, with types and sizes from the response headers.

Filter by type (Video, Audio, Images, Documents, Archives, Programs, Streams, Subtitles), by text
or `/regular expression/`, by site, and by minimum image size (icons and spacers are hidden by
default; so are links to other web pages). **Get sizes** asks each server for its file size.
Then **Download with reIDM** sends the selection in one batch, with the page as the referrer and
your cookies for each site — or **Download later** to put it in the scheduled queue, **Copy
addresses**, or **Export** as an IDM `.ef2` list.

## Capture rules

Open the extension's **Options** (popup › Options, or right-click the toolbar icon).

| Setting | Default | |
|---|---|---|
| Take over browser downloads | on | The master switch. |
| Take file links before the browser opens them | on | Click interception, for links whose address names a type on the list. |
| Every download, whatever its type | on | Off: only downloads whose name has one of the listed types. |
| File types | IDM's list | Space-separated extensions; `R0*` style wildcards work. |
| Leave downloads smaller than (KB) | 0 | 0 takes everything. |
| Never take downloads from these sites | — | One host per line; subdomains are included. |
| Also in incognito windows | off | Also needs *Allow in incognito* on the extension's details page. |
| Start reIDM when it is not running | on | Uses the `com.zektopic.reidm` helper. |
| reIDM port | 61002 | Only change it if you run reIDM on another port. |
| Look for video and audio | on | Media detection, the popup's list and the quality picker. |
| Never capture or list addresses starting with | `blob:`, `data:`, … | Address prefixes the extension ignores entirely. |

## Privacy and permissions

The extension talks only to reIDM on your own machine (`127.0.0.1`). It reads cookies **only for
the address being handed over, at the moment it is handed over**, and sends them only to reIDM,
so reIDM can download what you, signed in, can. Nothing is collected, stored remotely or sent
anywhere else.

| Permission | Used for |
|---|---|
| `downloads` | holding, pausing, cancelling and resuming the browser's downloads |
| `cookies` | the cookies a hand-off needs, for that one address |
| `nativeMessaging` | starting reIDM when it isn't running |
| `webRequest` | noticing media and form submissions (observation only) |
| `contextMenus`, `tabs`, `scripting` | the right-click entries, the grabber, and per-tab media lists |
| `storage`, `alarms` | settings, and a once-a-minute check that reIDM is running |
| host access to all sites | reading media manifests, the grabber, and click interception |

## How it is secured

The service only listens on `127.0.0.1`. Its extension endpoints accept requests from browser
extensions and local programs, and refuse web pages: browsers set the `Origin` header themselves,
a page can't fake an extension's origin, and reIDM answers cross-origin preflights only for
extension origins. The control API used by the window needs a per-run token kept in reIDM's data
folder. The native-messaging helper admits only this extension's ID. See
[ARCHITECTURE.md](ARCHITECTURE.md#security-model) for the details and the limits of each.

## When it doesn't connect

See [TROUBLESHOOTING.md](TROUBLESHOOTING.md#browser-integration). The usual causes, quickly:

- **Status bar still says "not connected".** The extension pings reIDM once a minute; give it a
  minute, or open the popup, which checks immediately.
- **Downloads still go to the browser.** Check the popup: is takeover on, and is the site on the
  never-capture list? The popup's *Just now* list says why the browser kept a download.
- **Tonec's IDM extension is installed too.** Two extensions both taking over downloads will fight;
  disable one of them.
