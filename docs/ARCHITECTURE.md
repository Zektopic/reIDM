# Architecture

How reIDM is put together: the pieces, how a download travels through them, how the engine
works, and what keeps the local service safe.

- [The pieces](#the-pieces)
- [How a browser download reaches reIDM](#how-a-browser-download-reaches-reidm)
- [The download engine](#the-download-engine)
- [Persistence](#persistence)
- [Security model](#security-model)
- [Source map](#source-map)

## The pieces

```mermaid
flowchart LR
  subgraph Browser
    CS[content.js<br>click interception]
    SW[service-worker.js<br>download takeover]
    GR[grabber page]
    IDMX[Tonec's IDM extension<br>optional]
  end
  subgraph reIDM
    D[reidm daemon<br>engine + service]
    H[reidm-host<br>native-messaging helper]
    G[reidm-gui<br>desktop window]
    C[reidm CLI]
  end
  CS --> SW
  GR --> SW
  SW -- "HTTP 61002 /ext/*" --> D
  SW -- "native messaging: launch" --> H
  H -- "starts" --> D
  IDMX -- "WebSocket 1001 / 61001" --> D
  IDMX -- "native messaging (com.tonec.idm)" --> H
  H -- "WebSocket 61001" --> D
  G -- "HTTP 61002 /api/* + token" --> D
  C -- "HTTP 61002 /api/* + token" --> D
```

| Component | What it is |
|---|---|
| **`reidm daemon`** | The service: the download engine, the HTTP API and web view on port 61002, and the IDM extension protocol on 61001 (and 1001 where it can be bound). Everything else is a client of it. |
| **`reidm-gui`** | The desktop window (egui). It polls `/api/downloads` about three times a second and renders it; every button is an API call. Starts the service if it isn't running. |
| **`reidm` CLI** | The same binary as the service, with commands that call the API ([CLI.md](CLI.md)). |
| **`reidm-host`** | The native-messaging helper the browser runs. For reIDM's extension (`com.zektopic.reidm`) it does one thing — start the service. For Tonec's extension (`com.tonec.idm`) it relays the IDM protocol to the service. |
| **reIDM's extension** | `extension/`, a Manifest V3 extension for Chromium browsers ([BROWSER_INTEGRATION.md](BROWSER_INTEGRATION.md)). |

All four programs run as the user, not as a system service, and talk over `127.0.0.1` only.

## How a browser download reaches reIDM

There are two paths, layered so that nothing slips through. Both end at the same endpoint,
`POST /ext/add`, with the cookies the extension read for that one address.

**Click interception** (`content.js`). A plain left click on a link whose address names a type
on the capture list is stopped before the browser follows it and sent to the service worker,
which asks reIDM to take it (`verify` + `refuse_html`). If reIDM refuses — the link is a web page,
or it can't be reached — the click is replayed for the browser.

**Download takeover** (`service-worker.js`). Every other download reaches Chrome's downloader,
and is taken from there before anything is saved:

```mermaid
sequenceDiagram
  participant P as Page
  participant C as Chrome downloader
  participant E as Extension
  participant R as reIDM
  P->>C: navigation or click starts a download
  C->>E: onDeterminingFilename (download held)
  E->>C: pause()
  E->>E: keep it? (POST, blob:, incognito, rules)
  E->>R: POST /ext/add {url, cookies, referer, UA, verify}
  R->>R: GET Range: bytes=0-0 with those cookies
  alt reIDM can fetch it
    R-->>E: 200 {ok, id}
    E->>C: cancel() + erase()
    E->>C: suggest() (releases the held callback)
  else reIDM can't
    R-->>E: 400 {error}
    E->>C: suggest() + resume()
    Note over C: the browser downloads it as if reIDM weren't there
  end
```

Holding `onDeterminingFilename` is what makes this invisible: Chrome decides the file name before
it shows the download bubble, opens a *Save as* dialog or writes the file, so a download that is
cancelled while held leaves nothing behind. The extension must call `suggest()` for every
download it holds, on every path, or Chrome waits forever; the code is structured around that.

If the service isn't running, the extension asks `reidm-host` (`{"cmd":"launch"}`) to start it.
The helper starts `reidm daemon` detached from the browser — in its own process group on Unix,
and outside the browser's job object on Windows, where Chrome would otherwise kill it along with
the helper — and answers once the service accepts connections (it waits up to 12 seconds). The
extension then sends the download again. Meanwhile the browser's download stays paused and
held, so nothing is lost either way.

Downloads that stay with the browser on purpose — form submissions, `blob:`/`data:`, other
extensions' downloads, Alt+click, excluded sites, incognito — are listed in
[BROWSER_INTEGRATION.md](BROWSER_INTEGRATION.md#what-always-stays-with-the-browser).

**Media and the resource grabber.** `content.js` injects `inject.js` into the page's own
JavaScript context, where it can see the page's `fetch`/XHR responses — player data, HLS and
DASH manifests — which `chrome.webRequest` can't read. It reports what it sees and never changes
a request. The grabber page asks every frame's content script for links, images, media and the
page's `performance` resource entries, merges in what the service worker saw on the network, and
sends the selection to `/ext/add-batch`.

**Videos that need a player to resolve (YouTube).** The page's own addresses are scrambled or
missing, so the extension sends `/ext/add` the *page* and a yt-dlp format selector (`media`)
instead of a file address. The engine runs such a download through yt-dlp instead of its own
connections (`media.rs`): it fetches yt-dlp (and Deno, and on Windows ffmpeg) into `tools/`,
checked against each release's published SHA-256, and runs it with a progress template it can
parse, a cookies file made from the browser's cookies, and the page after `--`. yt-dlp reports
the file name before downloading and the final path after merging; progress goes into the same
fields as any download's. Stopping kills yt-dlp's whole process tree (its process group on Unix,
`taskkill /T` on Windows); resuming runs it again and it carries on from its `.part` files.

**Tonec's extension** speaks the IDM protocol — binary messages over a WebSocket, or over native
messaging through `reidm-host` — described in [PROTOCOL.md](PROTOCOL.md). `session.rs` answers
it the way `IDMan.exe` does.

## The download engine

`reidm-core` holds the engine; one `Manager` owns the list, the settings and every running
transfer.

**Adding.** An address is normalised to a key (query parameters sorted, fragment dropped). A key
already in the list resumes or reports that download rather than adding a copy. New downloads
are `Queued` and go to the back of the queue (`Item.order`); `pump()` starts queued downloads,
lowest `order` first, while fewer than *downloads at the same time* are running, reserving the
slot under the same lock that checks it, so two callers can't start the same download twice.
Moving downloads in the queue (`queue.rs`) changes the `order` of only those that change place.

**Probing.** The first request is `GET` with `Range: bytes=0-0`. A `206` with a total size
means the server serves ranges; the answer also gives the file name (`Content-Disposition`), a
strong `ETag`, `Last-Modified` and the content type. The connection is dropped after the
headers. (A `416` means an empty file.) When the extension asked for a check, this probe is
done before the download is accepted, and reused to start it.

**Segmentation.** A resumable file of 1 MiB or more is cut into one range per connection.
Each connection *claims* a range, streams it to its offset in the part file, and claims another
when it's done. When nothing unclaimed is left, a connection splits the range with the most
remaining bytes — taking its second half, if that half is at least 256 KiB — so fast
connections keep working until the end instead of waiting for the slowest one. A server that
answers extra connections with `429`/`503` just ends up with fewer: the refused connection hands
its range back to the others, and every 30 seconds the download tries to get back to its full
number of connections. A connection that breaks off after making progress carries on from where
it got to without using up a retry; only failures that get nowhere count towards the retries.

A connection collects what it receives into a buffer (`write_buffer_kib`, 256 KiB by default, or
a second's worth when it is slow) and writes that at its offset with a positional write on the
blocking pool; the bytes count as downloaded only once the write has returned. A request that
gets no answer within `connect_timeout_secs` (30), or a body that goes quiet for
`idle_timeout_secs` (60), is retried, and Pause wakes a connection whatever it is waiting on — the
server, or the speed limit. The performance presets (`settings.rs`, `Performance::apply`) set the
connections, downloads at once and this buffer together.

**Network.** Every request goes through one `reqwest` client built from the settings: the proxy
(`system`, `none` or a manual HTTP/HTTPS/SOCKS5 one with a bypass list) and the two time limits.
A change builds a new client; a running download takes it at its next connection. yt-dlp is
given the proxy in a private options file (never on its command line), and torrents' peer
connections go through a SOCKS5 proxy when asked, from the next torrent session.

**Space.** Before a download starts, and with each checkpoint while it runs (every 10 s for videos
and torrents, whose writes reIDM doesn't see), `space.rs` checks that what is still to be written
fits on its drives above `min_free_space_mib` — space already reserved for the part file counts as
written, and a download folder on another drive needs the whole file for the final copy. If not,
the run stops like a Pause (everything written is kept and recorded), the download is `Paused`
with `waiting_for_space` and a note saying how much it needs, and every 30 s the downloads waiting
for space that now fit — counted against each drive's free space in queue order — are queued
again.

**Checksums.** A download given a checksum is hashed (MD5, SHA-1, SHA-256 or SHA-512) on the
blocking pool once it is in place; a mismatch fails the download and keeps the file. Resuming it
hashes the file again instead of downloading.

**Correctness checks.** Range requests carry `If-Range` with the strong `ETag` from the probe,
so a server whose file has changed answers with the whole new file (`200`) instead of the range.
That, or a `Content-Range` reporting a different total size, stops the download with *the file
changed on the server* rather than splicing two versions together. A response that starts at a
different byte than the one asked for is rejected before anything is written. Responses are
never transparently decompressed: the bytes on disk are the bytes the server stores.

**Failures.** A failed connection is retried `retries` times (5; 0 for ever), waiting
`retry_wait_secs` (3) first and twice as long each time, up to a minute; a connection that
delivers nothing for `idle_timeout_secs` counts as failed. Errors that retrying can't fix (a changed file,
a disk error) stop the download at once.

**Speed limits.** A token bucket (half a second of burst) shared by every connection of every
download enforces the global limit; each download has its own bucket for its own limit, and a
connection waits for both. Changing a limit applies to transfers already running.

**Scheduler.** A task checks every 15 seconds whether the schedule's start or stop time was
crossed since the last check — in local time, on the selected days — and starts or stops the
scheduled queue. A stop time before the start time means the next morning. Missing a start
while asleep starts the queue on waking, unless the stop time also passed.

**Power.** Once a second the service looks at whether anything transfers (`power.rs`). While
something does, and for 30 s after, it holds the system's sleep inhibitor (`keep_awake`; also
while seeding with `keep_awake_seeding`): a `systemd-inhibit --what=sleep … cat` child on Linux and
`caffeinate -i -w <pid>` on macOS — each ends by itself if reIDM dies, `cat` when reIDM's end of its
input pipe closes — and a thread holding `SetThreadExecutionState(ES_SYSTEM_REQUIRED)` on Windows.
The same look runs the when-done countdown: once something has run since an action was chosen
and nothing is running or queued, 60 s, then the list is saved and the action done — `exit`
through the service's shutdown, the others through the system (`PowerActions`). Both the
inhibitor and the actions are traits, faked in the tests.

## Persistence

| File (in the data folder) | Written |
|---|---|
| `downloads.json` | the list, including each range's progress: within 2 seconds of a change (longer for a list that takes long to save, up to 30 s), at once for adds, removals and status changes (mode 0600: it holds cookies) |
| `downloads.json.bak` | the previous copy of the list, kept by each save |
| `settings.json` | on every settings change (mode 0600, with a `.bak` as well) |
| `api-token` | at service start (mode 0600); removed at shutdown |
| `browser-extension/` | synced from the bundled extension at service start |
| `gui.json` | the window's preferences |

The list lives in memory behind one lock (`store.rs`), its items copy-on-write. Whatever changes
it through that lock moves the revision (`Manager::revision`, for clients polling for changes) on
and schedules a save; one thread does the saving. Taking an item for writing also records its id
against the new revision (`Manager::changed_since`, the last 65,536 ids), so the service's row
cache serialises only the rows that changed — with running downloads, whose progress moves the
revision without such a record, always counted as changed. The saver waits up to 25 times as long
as the last save took (2–30 s) for more changes, so a long list costs at most a few per cent of a
core to keep on disk. It copies the list's pointers under the lock and
serialises and writes outside it, so a list of thousands never holds up the service while it is
written. A save goes to a temporary file, which is flushed to the disk, then renamed over the old
one; the old one becomes `.bak` (a hard link), and the folder is flushed too. On start, a damaged
list or settings file is kept aside as `<name>.corrupt-<time>` and its `.bak` used; with neither,
reIDM starts empty and says so (`Manager::warnings`). A file that exists but can't be read is an
error instead, since saving an empty list over it would destroy it. `Manager::flush` writes
what is pending at once and waits for it, for shutting down.

The data being downloaded goes to `<id>-<file>.reidm` in the temporary folder (`parts/` in the
data folder unless set); its location is recorded in the list, so changing the temporary folder
does not orphan it. It is given its full size when the size is known: on Linux the space is
reserved (`fallocate`), so a disk too small for it says so at once; on Windows the file is made
sparse first, or NTFS would fill everything below the first write with zeros before that write
returns. When complete it is moved to its folder with a rename that refuses to replace anything:
a file that has appeared there since (the browser's own copy, one the user saved) is kept, and the
download takes the next free name. Across drives it is copied beside its destination under a
temporary name first. Only *Redownload* of a finished download replaces that download's own file.
The finished file is marked as downloaded from the internet — `Zone.Identifier` on Windows,
`com.apple.quarantine` on macOS — as the browser would have marked it. The folder is
its type's (`category.rs`: Compressed, Documents, Images, Music, Programs, Video, else the
download folder), chosen from the likely name when added and again from the server's name after
the probe. A part file already beside its final file (from before 0.6) is used where it is.

On start, downloads that were running are marked paused. Progress reaches the list only through a
checkpoint (every 3 seconds, and when a run ends), which reads the ranges, flushes the part file to
the disk, and only then records them: every byte the list counts was written before that flush.
So a crash, a power cut or a full disk can cost the last few seconds, never a hole in the file.
Before resuming, the engine still checks that the part file is at least as long as the progress
recorded; if not — the part file was deleted or truncated — it starts that download over.

## Security model

The service accepts downloads, reads cookies passed to it, and writes files, so who may talk to
it matters. Its rule: **the local user may; web pages may not.** Any web page can send requests
to `127.0.0.1`, so each entry point says how it tells the two apart.

| Entry point | Who is let in | How |
|---|---|---|
| `/api/*` (port 61002) | the window, the CLI, your scripts | a random token, new at every start, in a file only your user can read, sent as `X-Reidm-Token` and compared in constant time. The `Host` must be `127.0.0.1:61002`/`localhost:61002`/`[::1]:61002` (stops DNS rebinding) and any `Origin` must be the service's own. |
| `/` (the web view) | your browser, typing the address | served without the token check, with the token embedded in the page. `Host` and `Origin` checks as above; `frame-ancestors 'none'` so no page can frame it. |
| `/ext/*` (port 61002) | reIDM's browser extension | the same `Host` rule as `/api/*`, and an `Origin` that is a browser extension, or no `Origin` at all. Browsers set `Origin` themselves; a page can't claim an extension's origin, and can't leave its own off. CORS preflights are answered only for extension origins, so a page can't even send the JSON request. A ping a browser marks `Sec-Fetch-Site: cross-site` (an `<img>` on a page) is answered but not counted as the extension checking in. |
| WebSocket (61001, 1001) | Tonec's extension, `reidm-host` | the same `Origin` rule, checked during the handshake. Only the IDM protocol is spoken: nothing on it can pause, remove or delete a download. |
| `reidm-host` | the two extensions | each native-messaging manifest lists exactly one extension ID in `allowed_origins`; the browser refuses every other caller, and the host itself refuses to relay for any caller but those two. It passes on only the IDM protocol's own message types. |

What this does and doesn't protect against:

- A **web page** can't add downloads, read the list, change settings or learn the token: it
  can't read the token file, and the browser won't let it fake the headers the checks rely on.
- A **program running as you** can do all of these. It could read the token file anyway, just
  as it could read your browser's cookies; reIDM doesn't try to defend your account against
  itself. Note that the `Origin` checks are no barrier to a local program, which can send any
  header it likes — they exist to tell web pages apart.
- **Other users** on the same machine can't read your token (mode 0600 on Unix; on Windows the
  data folder is in your profile), so they can't use `/api/`. They *can* reach the extension
  endpoints and the WebSocket, which can't tell a local program from the extension, and so could
  add downloads to your list — but not pause, remove or delete any. IDM's own port has the same
  property. On a personal machine it doesn't arise; on a shared one, bear it in mind.
- **Cookies.** The extension reads cookies only for the address it is handing over, only when it
  hands it over. reIDM keeps them with the download so it can resume, never returns them from
  the API, and never writes them into exports.
- **Downloads go where you said.** Server-supplied file names are sanitised — path separators,
  `..`, reserved Windows names and control characters are removed — so a server can't place a
  file outside the download folder.
- **Updates.** reIDM checks GitHub for a newer release and tells you; it never downloads or runs
  anything by itself.

## Source map

| Path | |
|---|---|
| `crates/reidm-proto` | the IDM wire format: message encoding and the native-messaging framing |
| `crates/reidm-core/src/lib.rs` | the engine: `Manager`, probing, segmentation, retries |
| `crates/reidm-core/src/store.rs` | the list in memory, its saver thread, and crash-safe loading and saving |
| `crates/reidm-core/src/disk.rs` | positional writes, preallocation, the no-replace rename, Mark-of-the-Web, free space |
| `crates/reidm-core/src/settings.rs` | the settings, their patch, the performance presets and the proxy |
| `crates/reidm-core/src/queue.rs` | the queue's order and moving downloads in it |
| `crates/reidm-core/src/space.rs` | the not-enough-space pause and its retry |
| `crates/reidm-core/src/checksum.rs` | expected checksums and hashing files |
| `crates/reidm-core/src/power.rs` | the sleep inhibitor and the when-done countdown and actions |
| `crates/reidm-core/src/limiter.rs` | the token-bucket speed limiter, shared by every download (videos and torrents charge their bytes to it) |
| `crates/reidm-core/src/schedule.rs` | the scheduler's time windows |
| `crates/reidm-core/src/lists.rs` | `.ef2` import/export, batch patterns |
| `crates/reidm-core/src/torrent.rs` | torrents through [librqbit](https://github.com/ikatson/rqbit): one session for all, matched to the list by info hash; downloading, seeding to the share ratio, and the session's records in `torrents/` |
| `crates/reidm-core/src/media.rs` | video downloads through yt-dlp: fetching and checking yt-dlp, Deno and ffmpeg, running yt-dlp, its progress, stopping it |
| `crates/reidm-core/src/grabber.rs` | the site grabber, URL-list parsing |
| `crates/reidm-core/src/update.rs` | the release check |
| `crates/reidm/src/main.rs` | the CLI and service start-up |
| `crates/reidm/src/web.rs` | the HTTP API, extension endpoints and web view |
| `crates/reidm/src/ws.rs`, `session.rs` | the IDM protocol over WebSocket |
| `crates/reidm/src/install.rs` | native-messaging registration, extension staging |
| `crates/reidm-host` | the native-messaging helper |
| `crates/reidm-gui` | the desktop window |
| `extension/` | reIDM's browser extension |
| `tools/capture-test.mjs` | the real-browser test suite ([BUILDING.md](BUILDING.md#the-real-browser-tests)) |
| `decomp/` | decompiler output of IDM, kept as interoperability reference ([NOTICE.md](../NOTICE.md)) |
