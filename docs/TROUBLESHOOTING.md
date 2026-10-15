# Troubleshooting

- [Browser integration](#browser-integration)
- [Downloads](#downloads)
- [The service](#the-service)
- [Installing](#installing)
- [Getting more detail](#getting-more-detail)

## Browser integration

**The status bar says *Browser: extension not connected*.**

1. Is the extension loaded and switched on? Open `chrome://extensions` (or `edge://`,
   `brave://`…). reIDM's card should be there, enabled, with the ID
   `pgeonmdaagbiccimgojpafdjcipcfnlc`. If it isn't, load it: **Help › Browser integration** in
   reIDM shows the folder.
2. Give it a minute. The extension checks in once a minute; opening its popup checks at once.
3. Does the card show **Errors**? Click it — and please report what it says.
4. Was the folder it loaded from moved or deleted? Load it again from the folder reIDM shows;
   that one is kept up to date.

**Downloads still go to the browser.** Click the reIDM icon in the toolbar. The **Just now** list
says, for each download the browser kept, why:

| Reason | What to do |
|---|---|
| *capture is off* | turn on **Take over browser downloads** |
| *site is on the never-capture list* | remove the site in Options |
| *file type not on the list* | turn on **Every download, whatever its type**, or add the type |
| *smaller than the minimum size* | lower *Leave downloads smaller than* in Options |
| *incognito* | turn on *Also in incognito windows*, and *Allow in incognito* on the extension's details page |
| *a form submission (POST) cannot be repeated by reIDM* | by design: the server answers a form once |
| *made by the page itself (blob/data)* | by design: the file exists only inside the page |
| *the user asked the browser to keep it* | an Alt+click — or a clicked link reIDM couldn't take, which was handed back to the browser |
| *started by an extension* | another extension (or reIDM's own fallback) asked for the browser's downloader |
| an error from reIDM | reIDM tried and couldn't fetch the file — the link needs something the browser has and reIDM doesn't, or it can be used only once. The browser carried on, so nothing is lost |

If a download doesn't appear in *Just now* at all, the extension never saw it: check that it is
enabled, and that **Tonec's IDM extension** isn't installed alongside it. Two extensions that both
take over downloads interfere with each other; disable one.

**The popup says *reIDM is not running*, and Start reIDM does nothing.** The extension starts
reIDM through its native-messaging helper, `com.zektopic.reidm`.

- Start reIDM once by hand. The service registers the helper for every Chromium-based browser it
  finds, every time it starts.
- Or run `reidm install-host`, which also reports what it registered.
- Browsers installed as **Snap or Flatpak** packages run in a sandbox that usually can't start
  programs outside it. Capture still works while reIDM is running — start it at login instead
  (on Linux, `systemctl --user enable --now reidm` with the `.deb`).
- Where the registration lives, to check it by hand:

  | | |
  |---|---|
  | Windows | registry `HKCU\Software\<Google\Chrome \| Microsoft\Edge \| BraveSoftware\Brave-Browser \| Vivaldi>\NativeMessagingHosts\com.zektopic.reidm` → a `.json` file (Opera reads Chrome's key) |
  | macOS | `~/Library/Application Support/<Google/Chrome \| Microsoft Edge \| BraveSoftware/Brave-Browser \| Chromium \| Vivaldi \| com.operasoftware.Opera>/NativeMessagingHosts/com.zektopic.reidm.json` |
  | Linux | `~/.config/<google-chrome \| chromium \| microsoft-edge \| BraveSoftware/Brave-Browser \| vivaldi \| opera>/NativeMessagingHosts/com.zektopic.reidm.json` (the `.deb` also installs it system-wide under `/etc/opt/chrome`, `/etc/chromium` and `/etc/opt/edge`) |

  The `.json` file's `path` must point at an existing `reidm-host`, and its `allowed_origins`
  must list `chrome-extension://pgeonmdaagbiccimgojpafdjcipcfnlc/`.

**Load unpacked is missing or greyed out.** Turn on **Developer mode** on the extensions page.
On a managed (work) computer, a policy may forbid unpacked extensions; ask the administrator, or
use reIDM without the extension (paste links into *Add URL*).

**After updating reIDM, the extension is still the old version.** reIDM refreshes the folder
when it starts, and the browser reads it again when the browser restarts — or immediately if you
press the reload arrow on the extension's card.

**Tonec's IDM extension doesn't connect to reIDM.** See [EXTENSION.md](EXTENSION.md): on Windows
it needs port 1001, which a real IDM installation takes first (the service's log says
*not serving the extension's default port 1001 … another download manager may be using it*); on
macOS and Linux it needs `reidm patch-extension`. reIDM's own extension avoids both problems.

## Downloads

**"The file changed on the server since this download began".** The server now has a different
file at that address (a new version, a different size). reIDM stopped instead of joining the two.
Choose **Restart** (right-click) to fetch the new one from the start.

**A stopped download fails with `403`, `404` or `410` when resumed.** Its link has expired — signed
CDN links often last minutes or hours. Use **Refresh download address…** and either paste a fresh
link or click the download link in the browser again; reIDM continues where it stopped.

**The browser can download it, but reIDM gets `403`.** The site wants something the browser sends:
cookies, a referrer, or a particular user agent. Downloads handed over by the extension carry all
three. From the command line, pass `--cookie` and `--referer` to `reidm get`, or add the link
through the browser (right-click › *Download with reIDM*).

**Resume capability: No.** The server doesn't serve byte ranges. The download uses one connection,
and stopping it means starting again. Nothing in reIDM can change that.

**It's slower than expected.**

- Is a speed limit on? The status bar shows it; so does the download's Properties (its own limit).
- Some servers allow only one or two connections per client; reIDM detects refusals and
  carries on with fewer — *Downloading (2)* in the status column says how many are working.
- More connections (Options) help on servers that limit each connection's speed, not on a slow
  line.

**Downloads stay *Queued*.** No more than *Downloads at the same time* (Options) run at once; the
rest wait. Raise it, or stop something.

**Files ending in `.reidm` in the download folder.** Downloads in progress. Each is renamed to
its real name when complete. A leftover one belongs to a download you removed without *delete the
file*; it's safe to delete.

**A finished download reports *already downloaded* when added again.** reIDM recognises the
address. Delete the file (or remove the download with *delete the file*) to download it again,
or use **Restart**.

## The service

**Commands say "cannot reach the daemon".** The service isn't running. Start the window, or
`reidm daemon`.

**"web UI unavailable … (is another reidm running?)" / the service exits at once.** Ports 61001
and 61002 are taken — almost always by another reIDM. Find it (`ss -ltnp | grep 6100`,
`lsof -i :61002`, or on Windows `netstat -ano | findstr 6100`) and stop it, or just use that one.

**The service won't start: "corrupt downloads.json".** reIDM refuses to start with an unreadable
list rather than start with an empty one and lose your downloads. It writes the list atomically,
so this takes something outside reIDM — a disk error, an edit by hand. Move `downloads.json` out
of the data folder (or fix it) and start again; the files already downloaded are untouched.

**The window says it can't reach the service, but it's running.** The window authenticates with
the token in the data folder's `api-token`, written when the service starts. If the service runs
as a different user, or with a different data folder (`REIDM_DATA_DIR`, or `XDG_DATA_HOME` on Linux), the window can't
read it. Run both as the same user, with the same environment.

## Installing

**Windows: "Windows protected your PC".** The installer isn't code-signed yet. Choose *More info*
› *Run anyway*. See [PACKAGING.md](PACKAGING.md).

**Windows: "DeleteFile failed; code 5" while installing, or "reIDM is still running, and
Setup can't stop it".** Setup couldn't replace a reIDM program that was running. Setup stops
reIDM itself (from 0.4.1), but not one started as administrator (for example from an elevated
terminal or a remote session): end `reidm.exe` and `reidm-gui.exe` in Task Manager's *Details*
tab, or restart Windows, and run Setup again.

**macOS: "reIDM is damaged and can't be opened" or "can't be opened because Apple cannot check
it".** Builds are not notarized unless the project's Developer ID is configured. Right-click the
app › **Open** › **Open**, once. If macOS still refuses:

```sh
xattr -dr com.apple.quarantine /Applications/reIDM.app
```

**Linux: the window doesn't start.** Run `reidm-gui` from a terminal to see why — usually a
missing graphics library (`libxkbcommon0`, `libgl1`, `libwayland-client0`); the `.deb`
installs them.

## Getting more detail

- **The service's log.** Stop reIDM and run it in a terminal:

  ```sh
  RUST_LOG=reidm=debug,reidm_core=debug reidm daemon
  ```

  (PowerShell: `$env:RUST_LOG="reidm=debug,reidm_core=debug"; reidm daemon`.)
- **The extension's log.** On the extensions page, click *service worker* on reIDM's card. The
  console shows each decision, and `reidm.status()` and `await reidm.ping()` report the
  connection.
- **The list itself.** `downloads.json` in the data folder ([where](USER_GUIDE.md#where-reidm-keeps-things)).
  Stop reIDM before editing it.

When reporting a problem, include your system, browser and version, reIDM's version
(`reidm --version`), what you did, and the relevant log lines — with any addresses or cookies
removed.
