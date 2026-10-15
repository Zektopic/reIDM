# HTTP API

The reIDM service exposes everything the window can do as a small JSON API on
`http://127.0.0.1:61002`. The desktop window, the web view and the `reidm` command all use it;
so can your scripts.

- [Authentication](#authentication)
- [Downloads](#downloads) · [Adding](#adding) · [Controlling](#controlling) ·
  [Refreshing an address](#refreshing-an-address) · [Per-download speed limit](#per-download-speed-limit)
- [Queue](#queue) · [Settings](#settings) · [Status](#status) · [Export](#export) · [Site grabber](#site-grabber)
- [Browser-extension endpoints](#browser-extension-endpoints)
- [Errors](#errors)

The service listens on the loopback interface only. Nothing here is reachable from another
machine, and nothing here is versioned yet: the API follows reIDM's releases.

## Authentication

Every `/api/` request needs the header

```text
X-Reidm-Token: <token>
```

The token is random, created each time the service starts, and written to `api-token` in the
data folder — readable only by your user — and deleted when the service stops:

| | Token file |
|---|---|
| Windows | `%APPDATA%\Zektopic\reIDM\data\api-token` |
| macOS | `~/Library/Application Support/com.Zektopic.reIDM/api-token` |
| Linux | `~/.local/share/reidm/api-token` |

Two more rules keep web pages out (a page in your browser can send requests to `127.0.0.1`
too):

- the `Host` header must be `127.0.0.1:61002` or `localhost:61002`, which defeats DNS rebinding;
- if an `Origin` header is present, it must be the service's own (`http://127.0.0.1:61002`).

Command-line tools send no `Origin` and the right `Host`, so the token is all they need:

```sh
TOKEN=$(cat ~/.local/share/reidm/api-token)
curl -s -H "X-Reidm-Token: $TOKEN" http://127.0.0.1:61002/api/downloads
```

```powershell
$h = @{ 'X-Reidm-Token' = Get-Content "$env:APPDATA\Zektopic\reIDM\data\api-token" }
Invoke-RestMethod -Headers $h http://127.0.0.1:61002/api/downloads
```

Request bodies are JSON (`Content-Type: application/json`). Fields not listed are ignored;
optional fields may be left out.

## Downloads

### `GET /api/downloads`

Every download, in the order they were added. Cookies are never included.

```json
[
  {
    "id": 7,
    "request": {
      "url": "https://example.com/big.iso",
      "referer": "https://example.com/downloads",
      "cookie": null,
      "user_agent": null,
      "filename": null,
      "dir": null
    },
    "path": "/home/me/Downloads/big.iso",
    "size": 40000000,
    "downloaded": 4849664,
    "status": { "state": "Running" },
    "segments": [
      { "start": 0, "end": 4999999, "done": 606208, "finished": false, "active": true },
      { "start": 5000000, "end": 9999999, "done": 606208, "finished": false, "active": true }
    ],
    "resumable": true,
    "added": 1792019963,
    "last_try": 1792019963,
    "finished_at": null,
    "etag": "\"5f3a-1c9\"",
    "last_modified": null,
    "speed_limit": null,
    "scheduled": false
  }
]
```

| Field | |
|---|---|
| `id` | the download's number |
| `request` | what is fetched: address, and the referrer, user agent, file name and folder it was added with |
| `path` | where the finished file goes; while it runs, the data is in `path` + `.reidm` |
| `size` | total bytes, or `null` while unknown |
| `downloaded` | bytes so far |
| `status` | `{"state": "Queued" \| "Running" \| "Paused" \| "Done"}`, or `{"state": "Failed", "error": "…"}` |
| `segments` | the byte ranges (`end` inclusive) and each one's progress; `active` while a connection is writing to it |
| `resumable` | whether the server serves byte ranges |
| `added`, `last_try`, `finished_at` | Unix times, in seconds |
| `etag`, `last_modified` | the validators reIDM checks when it resumes |
| `speed_limit` | this download's own cap in bytes per second, or `null` |
| `scheduled` | in the scheduled queue (*Download Later*) |

The transfer rate isn't a field: take two readings of `downloaded` a few seconds apart.

### Adding

#### `POST /api/downloads`

```json
{
  "url": "https://example.com/big.iso",
  "referer": "https://example.com/downloads",
  "filename": "debian.iso",
  "cookie": "session=abc",
  "user_agent": "Mozilla/5.0 …",
  "dir": "/data/isos",
  "later": false
}
```

Only `url` is required. `filename` overrides the server's name; `dir` the download folder;
`later: true` puts it in the scheduled queue without starting it.

With `"media": { "format": "137+bestaudio/best[height<=1080]" }`, `url` is a page with a video
and reIDM fetches it through yt-dlp in that format (a
[yt-dlp format selector](https://github.com/yt-dlp/yt-dlp#format-selection): letters, digits and
`+/[]<>=!*_.,-:^$~?`, at most 200 characters). Without `filename`, yt-dlp names the file after
the video's title. The same page in two formats is two downloads.

```json
{ "id": 7, "outcome": "new" }
```

| `outcome` | |
|---|---|
| `new` | added |
| `existing` | reIDM already had this address; if it had stopped, it is running again |
| `already_complete` | already downloaded, and the file is still there |

Two addresses that differ only in the order of their query parameters or in their `#fragment`
are the same download.

#### `POST /api/downloads/batch`

Many at once. Each address is reported separately, so one bad address doesn't fail the rest.

```json
{
  "urls": ["https://example.com/a.zip", "https://example.com/b.zip"],
  "referer": "https://example.com/",
  "dir": "/data",
  "later": true
}
```

Instead of (or as well as) `urls`, `items` takes entries with their own headers — the shape an
`.ef2` import produces:

```json
{ "items": [ { "url": "https://example.com/a.zip", "referer": "https://example.com/", "cookie": "k=v", "user_agent": "UA/1" } ] }
```

`filename` is honoured only when the batch holds a single address. An item with `post_data`
(a form submission) is refused, since reIDM can't repeat a form.

```json
{
  "results": [
    { "url": "https://example.com/a.zip", "id": 8, "outcome": "new" },
    { "url": "not a url", "error": "invalid URL: relative URL without a base" }
  ]
}
```

### Controlling

| Request | |
|---|---|
| `POST /api/downloads/{id}/pause` | stop it, keeping what has arrived |
| `POST /api/downloads/{id}/resume` | carry on from where it stopped |
| `POST /api/downloads/{id}/restart` | discard what has arrived and start from the first byte |
| `POST /api/downloads/{id}/schedule` | put it in the scheduled queue (not a finished one) |
| `POST /api/downloads/{id}/unschedule` | take it out of the scheduled queue |
| `POST /api/downloads/{id}/open` | open the file, on the machine running reIDM |
| `POST /api/downloads/{id}/folder` | open its folder, on the machine running reIDM |
| `DELETE /api/downloads/{id}` | remove it from the list |
| `DELETE /api/downloads/{id}?purge=true` | remove it and delete the file (finished, or the partial `.reidm`) |

These take no body and answer `204 No Content`, or `400` with the reason as text — for
example `that download is already complete`.

### Refreshing an address

#### `POST /api/downloads/{id}/refresh`

Gives an unfinished download a new address for the same file — an expired link, a mirror — and
continues from where it was.

```json
{ "url": "https://cdn.example.com/file.bin?sig=fresh", "referer": "https://example.com/", "cookie": "session=new" }
```

reIDM stops the download, asks the new address for its size, and refuses an address that serves
a different size, or that can't serve byte ranges when part of the file is already done. The
new server's `ETag` is used from then on. `204` on success; `400` with the reason otherwise:

```text
the new address is a different file: it is 123 bytes, the download is 50000000 bytes
```

#### `POST /api/downloads/{id}/refresh-capture`

Makes the *next* download the browser extension hands over the candidate for this download's
new address, for three minutes. If that download is the same file, it becomes this download's
address and the browser doesn't download it a second time; if not, it is added as a normal
download and reIDM keeps waiting. `GET /api/status` reports the waiting download as
`refresh_armed`.

#### `POST /api/refresh-capture/cancel`

Stops waiting. `204`.

### Per-download speed limit

#### `POST /api/downloads/{id}/limit`

```json
{ "limit": 524288 }
```

Bytes per second; `null` or `0` removes this download's own cap. It applies immediately, and on
top of the global limit — the lower one wins. `204`.

## Queue

#### `POST /api/queue/start` · `POST /api/queue/stop`

Start every stopped download in the scheduled queue, or stop every one that runs. Downloads
still start only as slots free up (*downloads at the same time*).

```json
{ "count": 3 }
```

## Settings

#### `GET /api/settings`

```json
{
  "default_dir": "/home/me/Downloads",
  "connections": 8,
  "max_concurrent": 4,
  "speed_limit": 2097152,
  "limit_enabled": false,
  "schedule": { "enabled": true, "start": "23:00", "stop": "06:30", "days": [0, 1, 2, 3, 4] }
}
```

#### `POST /api/settings`

Any subset of the same fields; the rest stay as they are. Saved at once, and applied at once —
a new speed limit included, to transfers already running.

| Field | |
|---|---|
| `default_dir` | where new downloads go; created if missing |
| `connections` | per download, 1–32 |
| `max_concurrent` | downloads transferring at once, 1–64; the rest wait as `Queued` |
| `speed_limit` | the global cap in bytes per second, kept while the limiter is off |
| `limit_enabled` | whether the global cap is in force |
| `schedule.enabled` | run the scheduled queue at set times |
| `schedule.start`, `schedule.stop` | `"HH:MM"`, local time, 24-hour, or `null`. A stop earlier than the start means the next morning |
| `schedule.days` | Monday = 0 … Sunday = 6; empty means every day |

```sh
curl -s -H "X-Reidm-Token: $TOKEN" -H 'Content-Type: application/json' \
     -d '{"speed_limit": 1048576, "limit_enabled": true}' http://127.0.0.1:61002/api/settings
```

`204`, or `400` with the reason, for example `the start time "25:00" is not a time of day (use HH:MM)`.

## Status

#### `GET /api/status`

What the service is able to do right now.

```json
{
  "version": "0.4.0",
  "platform": "windows",
  "ui_port": 61002,
  "local_port": 61001,
  "idm_port": true,
  "idm_port_number": 1001,
  "extension": { "seconds_ago": 12, "version": "1.1.0", "browser": "Chrome 153" },
  "extension_dir": "C:\\Users\\me\\AppData\\Roaming\\Zektopic\\reIDM\\data\\browser-extension",
  "refresh_armed": null
}
```

| Field | |
|---|---|
| `idm_port` | whether the service holds port 1001, which Tonec's extension dials first |
| `extension` | when reIDM's browser extension last checked in, its version and browser; `null` if it hasn't since the service started. It checks in once a minute |
| `extension_dir` | the folder to load in the browser |
| `refresh_armed` | the download waiting for a new address from the browser, if any |

## Export

#### `GET /api/export?format=txt` · `GET /api/export?format=ef2`

The list as plain text: one address per line (`txt`, the default), or IDM's export format
(`ef2`) with each download's referrer and user agent. Never includes cookies.

## Site grabber

#### `POST /api/grab`

Reads a page and returns the files it links to. Nothing is downloaded.

```json
{
  "url": "https://example.com/papers/",
  "options": {
    "depth": 0,
    "types": ["pdf", "epub"],
    "same_host_only": true,
    "include": [],
    "exclude": ["draft"],
    "sizes": false,
    "max_pages": 50
  }
}
```

All options are optional. `depth` 0 reads only the page; 1 also the pages it links to, and so
on, up to `max_pages` pages. `types` empty means every link that looks like a file. `sizes`
asks each server for the size (one request per file).

```json
{
  "files": [
    { "url": "https://example.com/papers/a.pdf", "filename": "a.pdf", "ext": "pdf", "size": null, "found_on": "https://example.com/papers/" }
  ]
}
```

To download them, send the addresses to [`/api/downloads/batch`](#post-apidownloadsbatch).

## Browser-extension endpoints

reIDM's browser extension uses three endpoints of its own. A browser extension's service worker
can't read reIDM's token file, so these are authorised by **origin** instead: they accept
requests whose `Origin` is a browser extension (`chrome-extension://…`), or that have no `Origin`
at all (local programs, which could read the token anyway), and refuse everything else — a web
page can neither forge an extension's origin nor leave its own off. CORS preflights are answered
for extension origins only.

#### `GET /ext/ping?v=<extension version>`

Is reIDM running, and with what defaults? A ping that carries `v` is recorded as the extension
checking in (shown in `/api/status`).

```json
{ "app": "reIDM", "version": "0.5.0", "default_dir": "/home/me/Downloads", "connections": 8, "max_concurrent": 4, "media": true }
```

`media: true` means `/ext/add` accepts `media` (from 0.5.0). An older reIDM would save a video's
web page instead, so the extension checks this first.

#### `POST /ext/add`

```json
{
  "url": "https://example.com/get?id=123",
  "filename": "report.pdf",
  "referer": "https://example.com/reports",
  "cookie": "session=abc",
  "user_agent": "Mozilla/5.0 …",
  "verify": true,
  "refuse_html": true,
  "later": false
}
```

`media` works as in [`POST /api/downloads`](#post-apidownloads); `verify` is then ignored, since
yt-dlp finds the video's address when the download runs.

With `verify`, reIDM fetches the first byte before accepting, so that the extension can hand a
download back to the browser when reIDM can't fetch it; `refuse_html` also refuses an address
that answers with a web page (unless its name ends in `.html`). The extension sends both when it
is holding a browser download.

```json
{ "ok": true, "id": 9, "outcome": "new" }
```

`outcome` is `refreshed` when the address became the new address of a download waiting in
[`refresh-capture`](#post-apidownloadsidrefresh-capture). A refusal is `400` with
`{ "ok": false, "error": "the link answers with a web page, not a file" }`.

#### `POST /ext/add-batch`

```json
{
  "items": [ { "url": "https://example.com/a.jpg", "referer": "https://example.com/", "cookie": "k=v", "filename": null } ],
  "user_agent": "Mozilla/5.0 …",
  "later": false
}
```

Up to 5,000 items. The answer is per item, as for [`/api/downloads/batch`](#post-apidownloadsbatch),
with `"ok": true`.

## Errors

| Status | |
|---|---|
| `400` | refused, with the reason in plain text (JSON on `/ext/`) — or the body isn't valid JSON |
| `401` | missing or wrong `X-Reidm-Token` |
| `403` | wrong `Host`, or an `Origin` that isn't allowed |
| `404` | no such endpoint or action, or (for `open`/`folder`) no such download |
| `415` | a JSON body without `Content-Type: application/json` |
| `422` | valid JSON of the wrong shape, such as a missing `url` |

The extension protocol Tonec's *IDM Integration Module* speaks — a binary WebSocket protocol on
ports 61001 and 1001 — is described in [PROTOCOL.md](PROTOCOL.md).
