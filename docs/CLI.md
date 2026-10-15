# Command line

`reidm` is the service and its command-line client in one program. Most commands talk to the
running service (`reidm daemon`); a few work on their own. Every command has `--help`.

```text
reidm <command> [options]
```

| Command | Needs the service | |
|---|:---:|---|
| [`daemon`](#daemon) | — | run the service |
| [`get`](#get) | no | download one address in this terminal |
| [`add`](#add) | yes | queue addresses |
| [`batch`](#batch) | yes | queue a numbered or lettered series |
| [`import`](#import) | yes | queue a list or an IDM `.ef2` file |
| [`export`](#export) | no¹ | write the list as addresses or `.ef2` |
| [`list`](#list) | no | show the downloads |
| [`pause`, `resume`, `rm`](#pause-resume-rm) | yes | control one download |
| [`restart`](#restart) | yes | start a download again from the first byte |
| [`refresh`](#refresh) | yes | give a download a new address |
| [`limit`](#limit) | yes | show or set the speed limit |
| [`queue`](#queue) | yes | start or stop the scheduled queue |
| [`grab`](#grab) | only with `--add` | find the files a web page links to |
| [`extension`](#extension) | no | print the browser extension's folder |
| [`ui`](#ui) | starts it | open the web view |
| [`update-check`](#update-check) | no | look for a newer release |
| [`install-host`, `uninstall-host`](#install-host-uninstall-host) | no | register the browser helper |
| [`patch-extension`](#patch-extension) | no | adapt Tonec's extension for macOS/Linux |

¹ `export` uses the service when it runs, and reads the saved list when it doesn't.

Downloads are identified by the number `reidm list` shows in its first column.

## Starting the service

On Windows the installer starts the service at login. Elsewhere — or to run it by hand —
`reidm daemon`. Opening the desktop window (`reidm-gui`), running `reidm ui`, or a download in
the browser (through the extension's helper) also start it when it isn't running. Only one
service runs at a time; a second one finds the ports taken and says so.

### `daemon`

```text
reidm daemon [-c N] [-d DIR] [--no-idm-port]
```

| Option | |
|---|---|
| `-c`, `--connections N` | connections per download (default 8) |
| `-d`, `--dir DIR` | download folder (default: your Downloads folder) |
| `--no-idm-port` | don't try to serve Tonec's extension on port 1001 |

`--connections` and `--dir` are the starting values for a new installation. Once options have
been saved (in the window, or through the API), the saved ones win.

The service listens on `127.0.0.1` only:

| Port | |
|---|---|
| 61002 | the HTTP API, the web view, and reIDM's browser extension |
| 61001 | the IDM extension protocol (WebSocket), and `pause`/`resume`/`rm` below |
| 1001 | the same protocol on the port Tonec's extension dials first — Windows, or wherever it may be bound; skipped with a warning otherwise |

At start-up it also refreshes the browser-extension folder and re-registers the native-messaging
helper with every Chromium-based browser it finds; both only write what changed.

Stop it with Ctrl+C (or by ending the process). Downloads that were running are listed as
paused the next time; *Resume* (or `reidm resume`) continues each from where it stopped.

## Adding downloads

### `get`

```text
reidm get URL [-c N] [-d DIR] [-o NAME] [--referer URL] [--cookie COOKIES]
```

Downloads one address in the foreground, with a progress line, and exits when it is done. It
doesn't need — or touch — the service and its list.

```sh
reidm get https://example.com/big.iso -c 16
reidm get 'https://example.com/report?id=7' -o report.pdf --cookie 'session=abc'
```

### `add`

```text
reidm add URL... [--referer URL] [-o NAME] [--later] [--video [FORMAT]]
```

Queues one or more addresses in the service. `--output` names the file and is only allowed with
a single address; `--later` puts them in the scheduled queue without starting them.

`--video` says the addresses are pages with a video — YouTube, and the
[many other sites yt-dlp supports](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md)
— and reIDM fetches each through yt-dlp (see
[YouTube](BROWSER_INTEGRATION.md#youtube) for what it fetches the first time). `FORMAT` is a
[yt-dlp format selector](https://github.com/yt-dlp/yt-dlp#format-selection); without one, the
best video with its sound. Without `--output`, the file is named after the video's title and
height:

```text
$ reidm add --video 'bestvideo[height<=720]+bestaudio/best[height<=720]' 'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
1 queued, 0 resumed, 0 already complete, 0 failed
$ reidm list
   7  Done        153.7 MiB/153.7 MiB   /home/me/Downloads/Big Buck Bunny 60fps 4K - Official Blender Foundation Short Film [aqz-KE-bpKQ] 720p.mp4
```

Adding an address reIDM already has doesn't create a duplicate: a stopped download carries on,
and one that is finished — with the file still there — is reported as already complete. The
summary line says which happened:

```text
$ reidm add https://example.com/a.zip https://example.com/b.zip
  already have: https://example.com/a.zip
1 queued, 0 resumed, 1 already complete, 0 failed
```

### `batch`

```text
reidm batch PATTERN --to LAST [--from FIRST] [--width N] [--dry-run] [--later]
```

IDM's *batch download*: the address has one `*`, replaced by every value from `--from`
(default 1) to `--to`. Numbers can be zero-padded with `--width`; a single letter for both ends
makes a letter range in one case. One pattern can make up to 10,000 addresses.

```sh
reidm batch 'https://example.com/scans/page*.png' --from 1 --to 120 --width 3   # page001 … page120
reidm batch 'https://example.com/archive.part*.rar' --from a --to f              # parta … partf
reidm batch 'https://example.com/v*.mp4' --to 5 --dry-run                        # just print them
```

### `import`

```text
reidm import FILE [-d DIR] [--later]
```

Queues every address in `FILE` (`-` reads standard input). The file is either

- a plain list: one address per line, blank lines and `#` comments ignored, or
- an **IDM export** (`.ef2`, from IDM's *Tasks › Export*): each download's referrer, cookies and
  user agent are imported with it. Downloads IDM recorded as form submissions (`pd:` — POST data)
  are reported as failed rather than fetched with a GET, which would save some other answer.

The format is detected from the content, not the file name.

```sh
reidm import ~/links.txt
reidm import exported-from-idm.ef2 --later
grep -o 'https://[^"]*\.pdf' page.html | reidm import -
```

## Looking at downloads

### `list`

```text
reidm list
```

Prints each download's number, state, progress and file, and under a failed one, why:

```text
   1  Done          1.2 GiB/1.2 GiB     /home/me/Downloads/debian-13.iso
   2  Running     310.4 MiB/2.0 GiB     /home/me/Downloads/dataset.tar.zst
   3  Paused        4.0 MiB/?           /home/me/Downloads/stream.mp4
   4  Scheduled       0.0 B/?           /home/me/Downloads/backup.7z
   5  Failed          0.0 B/?           /home/me/Downloads/old.zip
      HTTP status client error (404 Not Found) for url (https://example.com/old.zip)
```

The states are *Queued* (waiting for a free slot), *Running*, *Paused*, *Scheduled* (in the
scheduled queue, not started), *Done* and *Failed*.

It reads the saved list, so it works when the service isn't running; while it is, the list is
saved every few seconds, so very recent progress may lag slightly. The [HTTP API](API.md) has the
live view.

### `export`

```text
reidm export [FILE] [--format txt|ef2]
```

Writes every download's address to `FILE` (default `-`, standard output). The format follows
the file's extension — `.ef2` writes IDM's export format, which IDM itself can import — or
`--format`. Exports never include cookies.

```sh
reidm export backup.ef2
reidm export --format txt | wc -l
```

## Controlling downloads

### `pause`, `resume`, `rm`

```text
reidm pause ID
reidm resume ID
reidm rm ID [--purge]
```

`resume` continues a paused or failed download from where it stopped. `rm` takes it off the
list; `--purge` also deletes the file — finished, or the partial `.reidm` file.

### `restart`

```text
reidm restart ID
```

Throws away what has been downloaded and fetches the file again from the first byte. This is the
way out when a download stopped because *the file changed on the server*.

### `refresh`

```text
reidm refresh ID NEW-URL
```

IDM's *Refresh download address*: points an unfinished download at a new address — an expired
signed link replaced by a fresh one, or a mirror — and carries on from where it was. reIDM first
checks the new address: it must serve a file of the same size and, if part of the download is
already done, serve byte ranges so it can continue. An address that fails either check is
refused and the download is left as it was.

```text
$ reidm refresh 7 'https://cdn.example.com/file.bin?expires=…&sig=…'
download #7 continues from https://cdn.example.com/file.bin?expires=…&sig=…
$ reidm refresh 7 https://example.com/something-else.bin
Error: the new address is a different file: it is 123 bytes, the download is 50000000 bytes
```

### `limit`

```text
reidm limit            # show it
reidm limit RATE       # set it and turn it on
reidm limit off        # turn it off (the rate is kept)
reidm limit on         # turn it back on
```

The global speed limit, shared by every download and connection, applied at once to transfers
already running. `RATE` is per second, in binary units as IDM uses them: `500K` = 500 KiB/s,
`2M` = 2 MiB/s, `1.5MB`, `800kb/s`, or plain bytes (`65536`).

A limit for a single download is set in its Properties window, or with
[`POST /api/downloads/{id}/limit`](API.md#per-download-speed-limit).

### `queue`

```text
reidm queue start
reidm queue stop
```

Starts or stops every download in the scheduled queue (those added with `--later` or moved there
in the window), and prints how many — `1 download(s) started`. No more than *downloads at the same time* run at once; the
rest wait their turn. The times and days the queue starts by itself are set in the window's
*Scheduler*, or through [`/api/settings`](API.md#settings).

## Finding files on a page

### `grab`

```text
reidm grab URL [-d DEPTH] [-t TYPES] [--same-host] [--include TEXT]... [--exclude TEXT]...
               [--sizes] [--export FILE] [--add]
```

The site grabber: reads a page and lists the files it links to, without downloading anything.

| Option | |
|---|---|
| `-d`, `--depth N` | 0 = only this page; 1 = also the pages it links to, and so on (at most 50 pages are read) |
| `-t`, `--types LIST` | extensions to keep, comma-separated (`pdf,epub`); default: common file types |
| `--same-host` | only links on the page's own host |
| `--include TEXT` | only addresses containing `TEXT` (repeatable) |
| `--exclude TEXT` | drop addresses containing `TEXT` (repeatable) |
| `--sizes` | ask each server for the size (one request per file) |
| `--export FILE` | write the addresses to `FILE` (`-` for standard output) instead of a table |
| `--add` | queue everything found |

```sh
reidm grab https://example.com/papers/ -t pdf --same-host --sizes
reidm grab https://example.com/gallery/ -d 1 -t jpg,png --export images.txt
```

It reads the HTML as the server sends it, so pages that build their links with JavaScript, or
that need you signed in, show little; the browser extension's *Resource grabber* sees those.

## Browser integration

### `extension`

```text
reidm extension
```

Prints the folder to choose in the browser's *Load unpacked* (and refreshes it). See
[Browser integration](BROWSER_INTEGRATION.md).

### `install-host`, `uninstall-host`

```text
reidm install-host
reidm uninstall-host
```

Registers — or removes — the native-messaging helpers with Chrome, Chromium, Edge, Brave,
Vivaldi and Opera for the current user:

- `com.zektopic.reidm`, which reIDM's extension uses to start the service, and
- `com.tonec.idm`, the name Tonec's extension uses ([EXTENSION.md](EXTENSION.md)).

The service registers `com.zektopic.reidm` by itself at every start, so `install-host` is mostly
for repairing or for Tonec's extension. A registration made by an installer is left alone.

### `patch-extension`

```text
reidm patch-extension SRC DST
```

Writes a copy of an unpacked *IDM Integration Module* (Tonec's extension, version 6.43.1) that
works on macOS and Linux — see [EXTENSION.md](EXTENSION.md). `SRC` is left unchanged; nothing
of Tonec's is shipped with reIDM.

## Other

### `ui`

```text
reidm ui
```

Opens the web view, `http://127.0.0.1:61002/`, in your default browser, starting the service
first if it isn't running.

### `update-check`

```text
reidm update-check
```

Asks GitHub whether a newer release exists and prints its notes and the package for this system.
It downloads and installs nothing. Exit status 1 if the check itself failed.

## Environment

| Variable | |
|---|---|
| `RUST_LOG` | log detail of `reidm daemon`, e.g. `RUST_LOG=reidm=debug,reidm_core=debug` (default `info`) |
| `REIDM_GH_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN` | a GitHub token for `update-check`, only needed against rate limits or a private fork |
| `REIDM_DATA_DIR` | uses this folder as the data folder, on any system — for a second, separate instance or a test (it still needs ports 61001 and 61002 free) |
| `XDG_DATA_HOME` | (Linux) moves the data folder, `$XDG_DATA_HOME/reidm` |

## Exit status

0 on success; 1 with a message on standard error otherwise — including when a command needs the
service and it isn't running:

```text
Error: cannot reach the daemon at http://127.0.0.1:61002/api/downloads/batch (start it with `reidm daemon`)
```
