# reIDM browser extension audit (static, read-only)

Scope: `extension/` on branch `feature/torrents` (manifest 1.5.0). I also read the daemon side
of the refresh-capture flow (`crates/reidm/src/web.rs`, `crates/reidm-core/src/lib.rs`) because the
extension routes refreshed links through `/ext/add`. I ran nothing and edited nothing.

Severity: **H** high, **M** medium, **L** low. **[uncertain]** marks items that rely on Chrome
behaviour I could not check without running a browser. Each one comes with a test that settles it.

What is already sound: no dynamic `innerHTML` (the only two uses, `grabber.js:313` and `content.js:383`,
are constant strings; everything else uses `textContent`). Click interception checks `isTrusted`.
Every request to 127.0.0.1 comes from the service worker. Listeners are registered at top level.
Recent captures are capped at 20.

---

## HIGH

### H1. The popup's YouTube "best quality" row always fails: `ytdlpFormat(null)` throws
- **Where:** `service-worker.js:1078` passes `null` → `downloadVideoPage(…, f = null)` → `:956 ytdlpFormat(f)`. `:914 function ytdlpFormat(f = {})`: a default parameter applies only to `undefined`, not `null`, so `f.itag` throws a TypeError. Caller: `popup.js:168`.
- **Failure:** The user opens the popup on a YouTube watch page, ticks the video row and presses *Download selected*. The *Download file info* dialog opens and the user presses Start. `ytdlpFormat` then throws. Nothing is sent to reIDM, `choice.report` is never called, so the dialog stays open with Start and Later disabled. The message reply is never sent, and no toast appears.
- **Fix:** `function ytdlpFormat(f) { f = f || {}; … }`. Also wrap the bodies of `downloadVideoPage`, `downloadFormat`, `captureLink` and the other handlers in try/catch so that any exception calls `choice.report({ok:false,error})` and replies.
- **Test:** A unit test for `ytdlpFormat(null)`, `(undefined)`, `({})`, an audio-only format and a muxed format. In `capture-test.mjs`, send `{type:'download-video', page, title}` from an extension page with `askBeforeDownload` both on and off, and assert that reIDM receives an item whose `media.format` is `bestvideo…/best`.

### H2. The popup closes when the dialog opens, so a multi-item "Download selected" stops after the first item
- **Where:** `popup.js:162-176` runs `queue()` as a loop of awaited messages. `service-worker.js:523` opens the dialog with `focused: true`. A Chrome action popup closes as soon as it loses focus.
- **Failure:** The user selects three media rows. The first `download-video` opens a focused dialog, the popup closes, and its JS context dies mid-loop. Items 2 and 3 are never sent, and the popup's success or failure message is never seen. The same happens if a selected `add` follows a video row.
- **Fix:** Move the loop into the service worker with one `queue-media` message that carries all selected items, so the worker iterates and shows the dialogs. Better, show a single dialog for the batch (see M3). The popup should send and then close itself.
- **Test:** In the real-browser test, open the popup page as a tab (`chrome-extension://…/popup.html`), select two items with the dialog on, answer each dialog, and assert that both reach reIDM.

### H3. A download held in `onDeterminingFilename` can be lost for good when the dialog or worker goes away
- **Where:** `service-worker.js:273-284` (pending store), `:331`, `:362`, `:403-414` (`reconcilePending`), `:518-538` (`askUser`), `:540-588` (port handling); `confirm.js:7, 45` (no `onDisconnect` handler).
- **Failure scenarios:**
  1. **Window closed before the port connects.** If the user closes the popup window, or the window manager kills it, before `confirm.js` runs `chrome.runtime.connect`, no port ever exists, so `onDisconnect` never fires. The `asking` entry and the `takeOver` promise then never settle. The browser's copy stays paused, and its filename is never decided, so it is invisible in the download UI. The user clicked *download* and nothing ever happens.
  2. **Worker restarted mid-hand-off.** This can happen through an extension reload, a crash, or the user killing it. `reconcilePending` runs only when a worker starts, and it skips entries younger than 60 s ("still in flight"). Suppose the new worker was woken by a different download, and then stays alive for hours, which is likely because `webRequest` sees every request (see M5). The skipped entry is then never revisited, and the download stays paused.
  3. **Lost update.** `pendingUpdate` does a read, then a modify, then a write on a single key. Two downloads held at once (a page that starts two, or a zip plus its checksum file) can interleave as get, get, set(A), set(B), so A's entry is lost. If the worker then dies, A is never resumed.
  4. **Ordering.** The success path deletes the pending entry (`:362`) *before* it cancels the browser's copy (`:367`). If the worker dies in that gap, the copy stays paused with no record.
  5. **[uncertain]** Even when `reconcilePending` resumes a download, nothing can call the dead worker's `suggest`. I could not confirm whether Chrome releases the filename hold of a determiner whose service worker stopped. If it does not, a resumed download is still held.
- **Fix:**
  - Watch the dialog's window. Keep `windowId` from `chrome.windows.create`, add `chrome.windows.onRemoved`, and resolve that id as `cancel`. Also set an overall timeout, for example 10 minutes, after which the download is released to the browser (not cancelled).
  - In `confirm.js`, add `port.onDisconnect`. It should stop the heartbeat interval, which currently throws every 20 s on a dead port, and show "This download is no longer waiting — the browser will download it" with only a Close button.
  - Store pending entries one key each (`pending:<downloadId>`), so no read-modify-write is needed, and tag each with a per-worker `instanceId` (random at worker start). At start-up, every entry from another instance is orphaned whatever its age, so drop the 60 s heuristic.
  - Delete the pending entry only after cancel or resume has finished.
  - For orphans, if `chrome.downloads.search({id})` still shows the download `in_progress` with an empty `filename` after a resume, cancel it and restart it with `chrome.downloads.download({url, …})`. `keepReason` already leaves downloads with `byExtensionId` to the browser, so this restores "the browser downloads it".
- **Test:** In CDP, hold a download with the dialog on, then (a) close the dialog window via `Target.closeTarget` right after it is created, and (b) stop the worker via `ServiceWorker.stopWorker` while the dialog is open. Within N seconds, assert that the download either completes in the browser or is visibly cancelled, and is not stuck `in_progress` and paused. Also start two downloads in parallel and kill the worker, then assert that both are recovered.

### H4. Every manifest request is fetched again by the worker, twice, with a possible feedback loop **[loop uncertain]**
- **Where:** `service-worker.js:828-842` (`noteManifest` always calls `formatsForManifest`), `:844-851` (`onBeforeRequest`, with **no** `tabId < 0` check), `:869` (`onHeadersReceived` also calls `noteManifest`).
- **Failure:**
  - A live HLS player re-fetches its media playlists every 2–6 s. For each poll, `onBeforeRequest` (the URL ends in `.m3u8`) and `onHeadersReceived` (the MIME type is mpegurl) each call `noteManifest`, and each issues a credentialed `fetch`. That triples playlist traffic for as long as the stream plays. Tokenised CDNs may rate-limit the stream or invalidate its tokens.
  - **[uncertain]** If the extension's own worker requests are visible to its own `webRequest` listener (tabId −1), `formatsForManifest`'s fetch fires `onBeforeRequest`, which calls `noteManifest(-1, url)`. `record()` returns early, but the fetch runs again, which is an unbounded loop.
- **Fix:** In `noteManifest`, return when `tabId < 0`. Re-fetch a URL only if its tab entry has no `formats` yet, and keep an in-flight `Set` to collapse duplicates. Skip media playlists (no `#EXT-X-STREAM-INF`) after the first parse. Ignore requests whose `initiator` is the extension's own origin.
- **Test:** Serve a live `.m3u8` that is polled every 2 s from a test page. Count server hits over 30 s, with and without the extension, and assert that the extension adds at most one request per distinct URL. Count requests carrying `Origin: chrome-extension://` to rule the loop in or out.

### H5. Incognito cookies are wrong everywhere: `storeId: 'incognito'` is not a Chrome store id, and several paths use the regular profile's cookies
- **Where:** `service-worker.js:334, 619, 954, 986` pass `'incognito'`. Chrome's store ids are `"0"` and `"1"` (`'incognito'` is not valid; Firefox uses `firefox-private`). `cookieHeader` swallows the error (`:228`), so the result is *no cookies*. Separately, `:1063` (popup `add`), `:1139` and `:1151` (context menus), and `:210` (`sendBatch`, used by the grabber and "all links") call `cookieHeader(url)` with no store, which reads the **regular** profile's cookies for an action taken in incognito. The context menus, popup add and grabber also ignore `captureIncognito`.
- **Failure:** In incognito with capture enabled, a signed-in download goes to reIDM without cookies and fails or saves a login page. Through the context menu, popup or grabber in an incognito tab, reIDM gets the *non-incognito* session, so the incognito activity is tied to the main profile's identity and written to reIDM's history.
- **Fix:** Resolve the store from the tab with `chrome.cookies.getAllCookieStores()`, which maps `tabIds` to store ids. For downloads, which have no tab, use the store whose `id` is not the default when `item.incognito` is set. Pass that store on every path. Apply `captureIncognito` (or at least a confirmation) to the context menu, popup and grabber, and pass the incognito flag through `add`, `add-batch` and `open-grabber`.
- **Test:** Enable the extension in incognito. Set a cookie in incognito only, and a different one in the normal profile. Start a download from incognito by each path (held download, click, context menu, grabber), then assert the `Cookie` header the test server receives from reIDM.

### H6. YouTube after SPA navigation: the previous video's formats and title are used **[likely; check]**
- **Where:** `inject.js:312-317`: `currentFormats()` returns as soon as `window.ytInitialPlayerResponse` has `streamingData`. That global is set on the first page load and is not updated by YouTube's SPA navigation, as the file's own header at `:17-19` says. Also `ytTitle()` at `:297`, and `content.js:224-233, 236-242`.
- **Failure:** The user opens video A and then clicks to video B. Pressing *Download with reIDM* calls `queryFormats`, which answers with **A's** formats and A's title, while `page` is B's URL (`location.href`). reIDM's yt-dlp downloads B, using A's itag or height as the selector and A's title as the file name ("A 1080p.mp4" containing B). A periodic re-query also overwrites the correct list taken from `/youtubei/v1/player`.
- **Fix:** Track the latest player response by `videoDetails.videoId`, from both the global and `/youtubei/v1/player`. Answer only with the one whose id matches the `v=` or `/shorts/` id in `location.href`, or read `document.getElementById('movie_player')?.getPlayerResponse?.()`. When none matches, send no itag and use `document.title` for the title.
- **Test:** In a real browser on YouTube, or with a fixture page that sets the global and then `pushState`s to another id while serving a different `/youtubei/v1/player` body, assert that the picker's title and the `media.format` itag sent belong to the second video.

---

## MEDIUM

### M1. Pressing Enter on Cancel or *Download later* starts the download
- **Where:** `confirm.js:61-64`, a document-level `keydown` where Enter means `answer('start')`, together with `:49-50`.
- **Failure:** When the user tabs to Cancel and presses Enter, `keydown` fires first, so `answer('start')` runs and sets `answered`. The button's click then sends `cancel`, which the worker ignores because the dialog was already answered. The download starts against the user's choice. The same applies to *Download later* and the checkbox.
- **Fix:** Handle Enter only when `e.target` is the name input or the body: `if (e.key==='Enter' && (e.target===$('name') || e.target===document.body))`. Or put the fields in a `<form>` with Start as the submit button, and let Escape cancel.
- **Test:** A CDP key test: focus `#cancel`, press Enter, and assert that the download was cancelled.

### M2. The dialog's error state shows buttons that do nothing and wording that is wrong
- **Where:** `confirm.js:36-39` re-enables Start and Later, but `answered` stays true (`:50`), and the worker has already removed the `asking` entry (`service-worker.js:576-578`). Also `service-worker.js:545` (an unknown id) and `:552`.
- **Failure:** reIDM refuses, for example because the verify failed. For a held download, the browser has **already resumed it** (`:375`), yet the dialog says "reIDM could not take it" and offers Start and Later, which do nothing. The dialog never closes. For an unknown id (a stale window after a worker restart), Start does nothing.
- **Fix:** On `error`, swap the footer for a single **Close** button and give the outcome: "reIDM couldn't take it (…). The browser is downloading it instead." for a held download, or "…Nothing was downloaded." for a click. Otherwise, keep the entry and allow a retry.
- **Test:** Make reIDM refuse (a 403 from the test server) with the dialog on. Assert the dialog text and that a single button closes it.

### M3. Several dialogs at once stack on top of each other, steal focus, and have no limit
- **Where:** `service-worker.js:518-538` opens a new focused popup window at the same centred position for each request.
- **Failure:** A page that starts five downloads (or a user clicking quickly) gets five identical windows exactly on top of each other. The user sees one, answers it, and the next appears in the same place, which looks like the dialog "came back". Each one takes focus from what the user is doing. A hostile page can open a new focus-stealing window for every automatic download Chrome allows.
- **Fix:** Keep one dialog window and queue requests into it ("2 more waiting", with *Start all*). Or offset each window and cap concurrent dialogs, for example at 3, with the rest waiting. Rate-limit dialogs per tab origin.
- **Test:** Start three downloads from one page. Assert that a single dialog window exists and that all three answers are delivered.

### M4. A page can fill `storage.session`, which silently disables the pending-captures safety net
- **Where:** `content.js:203-221` forwards any `postMessage({__reidm:true, items})` from the page, and checks only `e.source === window`, which the page itself passes. `service-worker.js:1005-1019` stores `m.formats` with no limit on length or size. `:683-695` (`persistMedia`) serialises **every tab's** map in one `set`. `pendingUpdate` and `recordCapture` share the same 10 MB `storage.session` quota and swallow errors (`:281`, `:424`).
- **Failure:** Any script on a page (including third-party ads) posts 200 items with large `formats` arrays. Session storage reaches its quota, and from then on `pendingUpdate` fails silently, so H3's reconcile has nothing to work from. *Just now* stops updating too. Even without an attacker, YouTube's ~30 formats per entry across many tabs make every `persistMedia` call write megabytes every 250 ms.
- **Fix:**
  - Validate in the worker. Accept only known `kind` values. Require `isHttp(url)` with length ≤ 2 KB. Cap `formats` at about 64 entries with whitelisted fields, and cap items per message.
  - Persist only `{url, kind, mime, size, at}`, with formats kept in memory or truncated, and persist each tab under its own key (`media:<tabId>`).
  - Keep pending entries in their own keys so they stay small, and log write failures with `console.warn`.
- **Test:** A fixture page posts 10 MB of fake formats. Then hold a download, kill the worker, and assert that reconcile still resumes it.

### M5. Two `webRequest` listeners see every HTTP request in the browser, which keeps the worker alive on a busy or low-end PC
- **Where:** `service-worker.js:844-851` (`onBeforeRequest`, all types and all URLs) and `:855-877` (`onHeadersReceived` with `responseHeaders` for media, xhr, other, object and sub_frame).
- **Failure:** Every image, script, font and XHR on every tab is sent to the extension process and runs regexes. Requesting `responseHeaders` copies the headers of every XHR. The worker almost never idles, so it holds its memory all the time and runs the top-level `ping()` on each wake. On a busy page this is noticeable on a low-end machine.
- **Fix:**
  - Give `onBeforeRequest` URL filters, which Chrome applies in the browser process: `['*://*/*.m3u8*','*://*/*.mpd*','*://*/*.mp4*', …]` for each `MEDIA_EXT` entry, and `types: ['media','xmlhttprequest','other','object']`.
  - Limit `onHeadersReceived` to `['media','xmlhttprequest']`.
  - When `sniffMedia` is off, `removeListener` both (re-adding them on change is fine, because they are not needed to wake the worker).
- **Test:** A performance check: load a page making 500 image and XHR requests, and count listener invocations with a counter exposed on `globalThis.reidm`. The count should be about 0 for non-media requests.

### M6. A storage write every minute wakes every content script in every frame
- **Where:** `service-worker.js:134-137`: `publishStatus` always writes `reidm-status` with a new `at: Date.now()`. It is called by the 1-minute alarm (`:1185-1188`), by every worker start (`:1198`), and by `post()`. `content.js:51-58` listens to `storage.onChanged` in every frame.
- **Failure:** With 40 tabs averaging 5 frames, about 200 content-script wake-ups happen every minute forever, plus more on each worker restart, for a value that has not changed.
- **Fix:** Write only when `up`, `version`, `media`, `dir` or `folders` changed, and keep `at` in memory.
- **Test:** A unit test that `publishStatus` with an identical status does not call `storage.local.set`.

### M7. The content script's cost on every page and frame
- **Where:** `content.js:572-577`, a `MutationObserver` on `documentElement` with `subtree` plus `attributes` (src, href, type), in **every frame** (`manifest.json` `all_frames: true`, `document_start`).
  - Each burst runs `scan()` (`:299-319`, `querySelectorAll('a[href]')` over the whole DOM plus a regex on each) and sends all matches again (not a diff).
  - It also runs `queryFormats()`, a window `postMessage` that the page's own `message` listeners also receive, and `refreshFromWorker()`, another message to the worker.
  - `:405 setInterval(position, 1000)` calls `getBoundingClientRect`, a forced layout, every second in any frame that has *any* `<video>`, including muted autoplay ads, for the life of the page.
  - `:582-592` polls `location.href` every 1.5 s, and `inject.js:384-389` does the same every 2 s.
- **Failure:** On chat apps, feeds or Gmail (constant mutations), each frame does a full-DOM scan plus three cross-context messages about every 700 ms. On a low-end PC that adds measurable scripting time and worker wake-ups.
- **Fix:**
  - Gate everything on `sniffMedia` (see M8).
  - Observe `childList` only, and inspect only added nodes (`MediaRecord.addedNodes`) rather than rescanning everything.
  - Raise the debounce to about 2 s using `requestIdleCallback`. Send only URLs not sent before (a `Set`).
  - Call `refreshFromWorker` and `queryFormats` only when the button is shown or clicked.
  - Run `position()` from `ResizeObserver` or `IntersectionObserver` on the video plus scroll and resize, not from a 1 s interval.
  - Skip frames below a minimum size, and only create the button for a video that is actually playing (`playing` event).
  - Replace the URL polls with `navigation` events (`window.navigation?.addEventListener('navigate')`) where available.
- **Test:** Use a benchmark page that mutates 1,000 nodes per second. Measure `Performance.getMetrics` ScriptDuration with and without the extension, and assert the extension adds less than a few percent.

### M8. "Look for video and audio: off" does not stop the content script, `inject.js` or the video button
- **Where:** `content.js:43` (`CAPTURE_DEFAULTS` has no `sniffMedia`), `:180-192` (it always injects), `:299-319` and `:369`.
- **Failure:** A user who turns media detection off for performance or privacy still has `fetch` and XHR hooked on every page, the DOM scanned, and the button shown. The option does not do what it says.
- **Fix:** Read `sniffMedia` before injecting, and when it is off do nothing except the click handler. Because the setting is read asynchronously while injection should happen early, move `inject.js` into a manifest `world: "MAIN"` content script (see M9) and have it check a flag, or register and unregister it with `chrome.scripting.registerContentScripts` when the setting changes.
- **Test:** Turn `sniffMedia` off, load a page, and assert that `window.fetch.toString()` contains `[native code]` and that no reIDM shadow host exists.

### M9. `inject.js` is injected through a `<script src>` web-accessible resource: it races page scripts, can be blocked, and lets any site detect the extension
- **Where:** `content.js:180-192`; `manifest.json:57-66` (`web_accessible_resources` for `<all_urls>`).
- **Failure:**
  - The external script loads asynchronously, so inline player scripts run before the hook. The code admits this and compensates with the worker re-fetch from H4.
  - Some strict CSPs or Trusted Types setups break it **[uncertain which]**.
  - Because the ID is fixed by `key`, any web page can probe `chrome-extension://pgeonmdaagbiccimgojpafdjcipcfnlc/inject.js` to detect that reIDM is installed, which is a fingerprinting signal.
- **Fix:** Use a second manifest content script with `"world": "MAIN"`, `"run_at": "document_start"` and `"all_frames": true`, which Chrome 111+ supports (the minimum here is 120). It runs before page scripts, ignores page CSP, and needs no web-accessible resource. Then remove `web_accessible_resources`, or set `use_dynamic_url: true`.
- **Test:** On a page whose inline `<script>` fetches a manifest during parse, assert that the hook reports it without the worker's re-fetch. On another page, assert that `fetch('chrome-extension://…/inject.js')` fails.

### M10. The MSE hook adds the page itself as a "media" item, sometimes ticked by default
- **Where:** `inject.js:273-287` reports `{url: location.href, kind: 'mse'}`. `content.js:219-220` forwards it, `service-worker.js:1007` records it, and `popup.js:223-225, 150` lists it, with the first row ticked by default.
- **Failure:** On a non-YouTube site with an MSE player and no stream URL seen, the popup lists the page URL as media. *Download selected* then saves an HTML page. The page also appears as a "video" in the grabber (`grabber.js:210`).
- **Fix:** Send MSE as a per-tab *hint* message, not a media item, or filter `kind === 'mse'` out of the popup and grabber and show "a stream is playing; use the button on the video" instead.
- **Test:** A fixture page that uses `MediaSource` only. Assert the popup's list holds no item whose URL equals the tab URL.

### M11. The badge is global, not per tab
- **Where:** `service-worker.js:824` (`badge(String(tab.size))` with no `tabId`), `:888` and `:1035` (clearing it whenever *any* tab navigates), and `:459-464`.
- **Failure:** A background tab playing media keeps overwriting the count shown for the active tab. Navigating any tab clears it for all tabs. The ✓, = and ! flashes then hide the counts.
- **Fix:** `chrome.action.setBadgeText({tabId, text})` for media counts. Keep the global badge only for the transient ✓, = and ! flashes, and restore per-tab counts afterwards.
- **Test:** Two tabs with different media counts. Switch between them and read `chrome.action.getBadgeText({tabId})`.

### M12. The video picker leaks timers and shows an empty panel after the first download
- **Where:** `content.js:532` (`close()` on success), `:434-437`, `:369-409` (`ensureUi` adds scroll and resize listeners plus a new `setInterval` each time), and `:285-296` (`lastSig` is not reset).
- **Failure:**
  - After a successful pick, `close()` removes the host. The next `scan()` calls `ensureUi()`, which builds a new host *and another* interval and listener pair, so each download adds one more 1 s interval.
  - `lastSig` still equals the old signature, so `refresh()` returns early and the new panel stays empty. Clicking the button again shows a blank panel for the same video.
- **Fix:** Don't destroy the host. Hide the panel and show "Sent to reIDM ✓" for a moment. If destroying it is kept, reset `lastSig` and clear the interval and listeners in `close()`.
- **Test:** In the real browser, download a format from the picker, open the picker again, and assert that it lists formats and that only one interval is running (instrument it).

### M13. "Download all links on this page" queues up to 2,000 addresses at once, with no dialog
- **Where:** `service-worker.js:1160-1172`.
- **Failure:** One misclick on a news or forum page queues hundreds of HTML pages, navigation links and logout or "delete" GET links, all fetched with the user's cookies. A GET that changes state, such as `/logout` or `/unsubscribe?id=`, actually runs. The setting ignores incognito and `capture`.
- **Fix:** Open the grabber with links pre-filtered to file types and pre-selected, so the user confirms. At minimum, keep only `typeListed(url)` links and confirm the count first.
- **Test:** A page with 50 navigation links and 2 zip links. The menu action must not send the navigation links to reIDM.

### M14. The refresh-capture flow ("refresh download address") is invisible to the extension
- **Where (extension):** `service-worker.js:344, 610, 1133`: the normal *Download file info* dialog is shown for the link that will refresh. `:450-456` (`announce`) says "Downloading in reIDM" for `outcome: 'refreshed'`. `popup.js:128` does handle `refreshed`.
- **Where (daemon):** `crates/reidm/src/web.rs:476-486`: any `/ext/add` without `media` is tried as the refresh first. `crates/reidm-core/src/lib.rs:1408`: `stop_and_wait(id)` runs **before** the new address's size and range checks (`:1425-1436`). `REFRESH_WINDOW` is 180 s (`:43`).
- **Failure:**
  - The user arms *Refresh address* in reIDM and clicks the link again. The extension shows a *new download* dialog. If the user presses Cancel, believing it a duplicate, the refresh never happens. *Download later* is ignored.
  - Any unrelated download the browser starts within the 3-minute window is tried against the armed item first. That **stops the armed download** if it was still running (the mirror use case), then fails the size check and leaves it stopped **[daemon behaviour; verify the status after a failed refresh]**. It also adds the probe's latency to the unrelated download.
  - A magnet or YouTube link is safely skipped.
- **Fix:**
  - Expose `refresh_armed` (id and file name) in `/ext/ping` or through a cheap `/ext/refresh-target`. The extension can then skip the dialog, or show "Use this link to refresh *name*?", and announce "Address refreshed — *name* continues".
  - In the daemon, probe and validate the new address **before** `stop_and_wait`, and restore the previous status if validation fails.
- **Test:** An integration test with two armed scenarios: (a) the same file → `outcome: refreshed` and no new item; (b) a different file during the window → the armed download's status is unchanged and the new file is added. In the extension, assert the dialog text when refresh is armed.

### M15. Downloads handed to reIDM skip Chrome's download protection and get no Mark-of-the-Web or quarantine flag
- **Where:** The design of `takeOver` and `captureLink`. A search of `crates/` finds no `Zone.Identifier` or quarantine code.
- **Failure:** Chrome's Safe Browsing verdicts, its "dangerous file" warnings and the per-site automatic-download permission (for click captures) are skipped. Files saved by reIDM carry no MOTW on Windows and no `com.apple.quarantine` on macOS, so SmartScreen and Gatekeeper prompts do not appear. With *Ask before each download* off, a drive-by `.exe` from any page is saved silently.
- **Fix:** In the daemon, write `Zone.Identifier` (ZoneId=3, ReferrerUrl, HostUrl) on Windows and the `com.apple.quarantine` xattr on macOS. In the extension, consider keeping downloads that Chrome flags (`item.danger !== 'safe'`) with the browser. Warn about executable types when the dialog is off, or always show the dialog for them.
- **Test:** Download an `.exe` through reIDM on Windows and check `Get-Content file -Stream Zone.Identifier`.

### M16. The dialog's size probe adds a request, which can use up one-time links
- **Where:** `service-worker.js:487-509` (`probeSize`) and `:561-563`.
- **Failure:** For a clicked link, the server sees the probe (the extension's credentialed `Range: bytes=0-0`), then reIDM's verify probe, then the real download. A single-use or count-limited link is spent by the first request, reIDM's verify fails, and the hand-back to the browser fails too.
- **Fix:** Don't probe when reIDM will verify anyway. Have the daemon return size and name from a `dry_run` or `verify` call, or show "size: checked when it starts". Probe only after an explicit request.
- **Test:** A test server URL valid for one GET. Click capture with the dialog on, and assert that exactly one download succeeds in reIDM or the browser.

### M17. A page can spoof the picker's formats, page URL, title and type
- **Where:** `content.js:203-233` trusts `{__reidm:true, kind:'formats-answer'|items}` from any script running in the page. `service-worker.js:973-996` and `:936-965` send `msg.page` to yt-dlp with `cookieHeader(page)`. `formatFilename` (`:895-906`) takes the extension from a page-supplied `mime` (for example, `video/exe` gives `.exe`).
- **Failure:** A hostile page, or a third-party script on it, makes the reIDM button offer "1080p", which really saves `https://other-site/…` using the user's cookies for that site. It can also name the file `<title> 1080p.exe`. The user does have to click, and with the dialog off nothing else is shown.
- **Fix:**
  - Validate formats in the worker: `url` must be http(s) and length-limited, `mime` must match `^(video|audio)/[\w.+-]+$`, and the extension comes from a whitelist (mp4, webm, m4a, mkv, mov, mp3, ogg, opus, m3u8, mpd).
  - For `source === 'youtube'`, require `isYouTube(sender.tab.url)` and use `sender.tab.url` rather than `msg.page`.
  - In general, use `sender.tab.url` and `sender.frameId` instead of page-supplied URLs wherever possible.
- **Test:** A fixture page posts a spoofed `formats-answer` with `source:'youtube'` and `page:'https://example.org/x'`. Assert the worker refuses it.

### M18. Race on worker wake: media seen before restore is overwritten
- **Where:** `service-worker.js:697-710` (restore skips tabs that already exist) and `:801-826` (`record` does not await `mediaReady`).
- **Failure:** The worker is woken by a webRequest event on a media tab. `record` creates a new map for that tab, and `restoreMedia` then skips the tab. The 250 ms persist timer then saves the partial map over the full one, so the popup loses the stream and its qualities.
- **Fix:** Merge in `restoreMedia` (add any URLs not present). Or queue `record` calls until `mediaReady` resolves.
- **Test:** A unit test with mocked `chrome.storage.session`: a `record` before restore resolves keeps the stored entries.

### M19. Client timeouts can lead to both reIDM and the browser downloading **[uncertain]**
- **Where:** `service-worker.js:157-178` (25 s timeout; an abort is not treated as "unreachable") and `:356-361` (retry with `item.url`).
- **Failure:** With a slow origin, reIDM's verify takes more than 25 s. The extension times out and retries for another 25 s, then resumes the browser's copy, while reIDM may already have accepted the first attempt. The result is two downloads, or one in reIDM with an error toast.
- **Fix:** After a timeout, ask reIDM whether it took the download, for example `GET /ext/status?url=` or an idempotency key in `/ext/add`, before resuming. Don't retry after a timeout.
- **Test:** Make the test server delay the probe by 30 s. Assert exactly one download in total.

### M20. A cancelled download appears in *Just now* as "browser kept it"
- **Where:** `service-worker.js:351` (`recordCapture({error:'cancelled'})`) and `popup.js:126-129`.
- **Failure:** The user pressed Cancel and nobody downloaded the file, but the popup claims the browser kept it. Click and context-menu cancels are not recorded at all, which is inconsistent.
- **Fix:** Record `outcome: 'cancelled'` and render "cancelled". Record cancels from every path.
- **Test:** Cancel in the dialog and assert the popup row text.

### M21. `inject.js` keeps manifests forever, so after SPA navigation it can offer another video's qualities
- **Where:** `inject.js:38, 200, 253` (`seenManifests` has no limit and is never cleared) and `:323-327` (it picks the manifest with the most variants).
- **Failure:** On an SPA video site (Vimeo, Twitch VODs, news players), moving to the next video still lists the previous one's variants, because "most variants wins". Memory also grows on live streams with rotating token URLs.
- **Fix:** Clear `seenManifests` when `location.href` changes, prefer the most recent entry over the largest, and cap the map at about 20 entries.
- **Test:** A fixture that loads manifest A, then `pushState`s and loads manifest B with fewer variants. Assert the picker shows B.

### M22. Manifests requested with relative URLs through `fetch` are dropped
- **Where:** `inject.js:186` (a string `input` is used as is, so a relative URL stays relative), `:197` (`formatsFromHLS(body, '/x.m3u8')`, where `new URL` throws, so variant URLs are `null`), and `content.js:212` (`isHttp` filters out the relative URL).
- **Failure:** A player that calls `fetch('/hls/master.m3u8')` gets no formats from the hook. The XHR path already resolves relative URLs (`:235`).
- **Fix:** `url = new URL(url, document.baseURI).href` in the fetch wrapper. For `Request` objects, `input.url` is already absolute.
- **Test:** A fixture with a relative fetch of an HLS master. Assert the picker lists its variants.

---

## LOW

### L1. Privileged message types are accepted from content scripts
- **Where:** `service-worker.js:1000-1098`. `add`, `add-batch`, `download-video`, `launch`, `clear-tab`, `media-for-tab` (with any `tabId`), `open-grabber` and `open-tab` come from any sender. The page cannot send these today (no `externally_connectable`), but a compromised renderer can. `add` skips the dialog and reads cookies for any URL.
- **Fix:** Keep a list of types allowed from tabs (`media`, `formats-for-tab`, `capture-link`, `bypass`, `open-tab`, `download-format`). Accept everything else only when `sender.url` starts with `chrome.runtime.getURL('')`. For `formats-for-tab` from a tab, ignore `msg.tabId`.
- **Test:** Send `{type:'add'}` from a content-script context (`chrome.scripting.executeScript` with an isolated world) and assert it is refused.

### L2. `open-tab` does not pass `windowId` **[uncertain]**
- **Where:** `service-worker.js:1056`.
- **Failure:** With `index` and `openerTabId` but no `windowId`, the new tab may open in the last-focused window, which could be a normal window for an incognito link.
- **Fix:** Pass `windowId: t.windowId`.
- **Test:** Decline a `target=_blank` link in an incognito tab and check `tab.incognito` on the new tab.

### L3. The content script intercepts clicks in incognito when the worker will refuse them
- **Where:** `content.js:147`.
- **Failure:** The click makes a needless round trip and is then replayed with `a.click()`, so the page's click handlers (analytics, toggles) run twice.
- **Fix:** Also check `chrome.extension.inIncognitoContext && !cap.captureIncognito`, and add `captureIncognito` to `CAPTURE_DEFAULTS`.

### L4. The click hand-back replays the click into the page, and `handedBack` is dead code
- **Where:** `content.js:100-118, 162-166`.
- **Failure:** The replayed click runs the page's handlers again. If one of them calls `preventDefault`, the link is never followed. `handedBack` is only ever deleted from.
- **Fix:** Use `chrome.tabs.update` or `chrome.downloads.download` from the worker, with a bypass entry, to follow the link without replaying it. Remove the `WeakSet`.

### L5. The fallback file-type list in `content.js` differs from `DEFAULTS`
- **Where:** `content.js:68` against `service-worker.js:44-47` and `options.js:10-13`.
- **Failure:** ACE, AIF, PPT, TIF and others are taken when the browser downloads them, but are not intercepted on click.
- **Fix:** Move the defaults into one `defaults.js`, loaded first in the content-script `js` array and imported by the worker and options page.

### L6. Removing a setting from storage turns it into `undefined`
- **Where:** `service-worker.js:75-78`.
- **Failure:** After `storage.sync.clear()` or a removed key, `settings[k]` becomes `undefined`. Capture silently turns off and the port becomes `undefined`.
- **Fix:** `settings[k] = v.newValue ?? DEFAULTS[k]`.

### L7. Options: hidden save, unconfirmed reset, unchecked port
- **Where:** `options.js:56-70, 80, 99-104`.
- **Failure:** *Test connection* saves the port without telling the user. *Restore defaults* wipes `skipHosts` with no confirmation. A port such as 70000 is accepted. The page does not pick up changes made in the popup while it is open, so Save overwrites them. `skipHosts` keeps `:port`.
- **Fix:** Test the port without persisting it (pass it in the `ping` message). Confirm before reset, and leave the site list alone. Clamp the port to 1–65535. Listen to `storage.onChanged`. Strip ports from site entries.

### L8. Popup "Never on this site" disagrees with how site entries are matched
- **Where:** `popup.js:96` checks the exact host, but the worker skips subdomains.
- **Failure:** With `example.com` on the list, the checkbox shows unchecked on `www.example.com` even though that site is skipped.
- **Fix:** Use the same `hostSkipped` logic in the popup.

### L9. Grabber "same site" filter
- **Where:** `grabber.js:256, 269`.
- **Failure:** `endsWith(host)` without a dot boundary means `notexample.com` matches `example.com`. Taking the last two labels means `co.uk` matches every UK site.
- **Fix:** Match `h === host || h.endsWith('.'+host)`, and accept the last-two-labels approximation only with a note.

### L10. Grabber *Get sizes* sends credentialed GETs to page-supplied URLs
- **Where:** `grabber.js:451-475`.
- **Failure:** Any GET that changes state on those URLs runs.
- **Fix:** Exclude links that have no file extension.

### L11. `Content-Disposition` parsing
- **Where:** `service-worker.js:495`.
- **Failure:** `UTF-8''` is matched case-sensitively, so `utf-8''name` keeps the prefix in the file name. Quoted `filename*` values are not handled.
- **Fix:** Use a case-insensitive regex and strip the charset and language prefix generally.

### L12. *Don't ask again* takes effect even on Cancel
- **Where:** `service-worker.js:572-575`.
- **Failure:** Ticking the box and pressing Cancel turns the dialog off for good, and nothing says where to turn it back on.
- **Fix:** Apply it only on Start or Later, and add "(turn back on in Options)".

### L13. Muxing and DRC edge cases in `ytdlpFormat` and the file name **[uncertain, depends on reIDM's yt-dlp flags]**
- **Where:** `service-worker.js:914-923, 895-906`.
- **Failure:** The selector is `itag+bestaudio[ext=m4a]` for a webm video, while the name ends in `.webm` (from the MIME type). yt-dlp may write `.mkv` or `.mp4`. A "stable volume" (DRC) audio choice maps to the plain itag, so the user gets the version without DRC.
- **Fix:** Let reIDM set the final extension from yt-dlp's output and send the name without an extension. Pass the DRC format id string.

### L14. HLS and DASH "qualities" outside YouTube save a playlist, not a video
- **Where:** `service-worker.js:979-996`; documented in README.md.
- **Failure:** The user picks "720p" and gets a `.m3u8` text file.
- **Fix:** reIDM already ships yt-dlp, so send `media: {format}` with the manifest URL and let yt-dlp fetch and merge. Otherwise, label these rows "playlist file (not a video)".

### L15. PDF is on the click-interception list
- **Where:** `DEFAULTS.fileTypes` at `service-worker.js:44-47`.
- **Failure:** Clicking a PDF link to *read* it sends it to reIDM. This is IDM-compatible, but it is a common complaint.
- **Fix:** Leave PDF out of click interception by default, or intercept only links that carry the `download` attribute.

### L16. A paused browser connection can time out while the dialog is open
- **Where:** `service-worker.js:330, 344`.
- **Failure:** The user leaves the dialog for minutes and reIDM then refuses. The resumed browser download may fail if the server dropped the idle connection and does not support ranges.
- **Fix:** The dialog should say the server may time out. Hand back sooner (a dialog timeout), or retry with `chrome.downloads.download`.

### L17. Documentation drift
- **Where:** The README permission table (`extension/README.md`) and `docs/BROWSER_INTEGRATION.md` (the privacy table) leave out `notifications`.
- **Fix:** List it in both tables, and state that the cookie *values* (including HttpOnly cookies) go to reIDM.

### L18. Host permissions are broad
- **Where:** `manifest.json` grants `<all_urls>` hosts plus `cookies`, which together can read every cookie.
- **Failure:** None by itself; this is a breadth note.
- **Fix:** Justify the breadth in the docs, as is partly done already. Consider `optional_host_permissions` for media detection and the grabber, so a user who wants only download takeover can grant less. Download takeover itself needs no host permission; `cookies` does.

---

## UX notes

**Dialog (`confirm.html`)**
- Wording: "Download later" → **"Add to queue"** (subtitle "starts on reIDM's schedule"). "Start download" is fine. The window title "Download file info — reIDM" can stay for IDM familiarity, but say *where the download came from* (the page host) as well as the address.
- The folder cannot be changed ("Change it in reIDM's Options"). IDM lets you change it per download. Add a *Change…* button that sends `dir`, or say "reIDM chooses the folder by file type".
- Keyboard: M1. Enter should act only from the name field. Focus starts in the name field with the base name selected (not the extension), as IDM does. Today `select()` selects the whole name, extension included.
- Several dialogs at once: M3. Use one window and a queue.
- After an error: M2.
- The name field accepts `/` and `\`. reIDM sanitises the name, but the dialog should say so or strip them as the user types.

**Popup**
- Rows show the full URL, which is noisy. Show host and size, and put the URL in a tooltip.
- *Download selected* with the first row ticked by default, combined with M10, makes it easy to download the wrong thing. Tick nothing by default, or tick only on YouTube or for a single stream.
- *Just now*: distinguish *cancelled*, *browser kept it (reason)*, *in reIDM* and *address refreshed* (M20).
- *Take over browser downloads* and *Never on this site* sit side by side with no state hint. When takeover is off, grey out the site toggle.
- When reIDM is down: the message plus the *Start reIDM* button is good. Add "Downloads go to the browser meanwhile".

**Options: proposed simple/advanced split.** Today the page shows 13 controls with the same weight.
- *Simple* (always visible): Take over browser downloads · Ask before each download · Never on these sites · Show notifications · Also in incognito.
- *Advanced* (collapsed `<details>`): Take file links on click · Every download vs. listed types + File types · Minimum size · Start reIDM automatically · Port + Test · Look for video and audio · Ignored address prefixes.
- Save automatically on change, with an inline "Saved", to get rid of the sticky Save bar and the overwrite problem in L7. *Restore defaults* goes into Advanced and asks for confirmation.

---

## Code-quality notes a senior reviewer would raise
1. **Duplicated logic:** the HLS and DASH parsers and `codecNames` exist in both `inject.js` and `service-worker.js`. `DEFAULTS` exists in three files and differs in one (L5). `extOf`, `hostSkipped` and `humanBytes` are copied 3–4 times. Share them as modules for the worker and pages, and as a shared content-script file.
2. **Errors swallowed everywhere** (`catch {}` with no log). Several bugs above, H1 and M4 among them, were silent because of it. Log at `debug` at least, and add one top-level try/catch per message handler that always replies.
3. **No message schema.** `onMessage` trusts the shape of `msg`. Add a validator for each type (L1, M4, M17).
4. **Pure logic has no unit tests:** `keepReason`, `ytdlpFormat`, `formatFilename`, `typeListed`, `parseHLS`/`parseDASH`, `hostSkipped`. The worker is already an ES module, so move them to `lib.js` and test them with `node --test`. Only the end-to-end `capture-test.mjs` exists today.
5. **`takeOver` mixes concerns**: policy, pausing, the dialog, sending, cleanup and the UI side effects. Make it a small state machine (`held → asked → sent → (taken|returned|cancelled)`) that persists each transition. That also fixes H3.
6. Callbacks and promises are mixed (the `callback()` helper, `chrome.downloads.*` with callbacks). Chrome 120 has promise forms of every API used.
7. `formatsForManifest` reads the whole body and *then* slices it to 2 MB (`:792`). Read with a size limit, or check `content-length` first.
8. `LIKELY_MANIFEST` (`inject.js:33`) is a subset of `MANIFEST`, so it is dead code. So is `handedBack` (L4).
9. `downloadVideoPage`'s file-name expression (`:950`) is hard to follow. Compute `defaultName` once.
10. Magic numbers (60000, 120000, 15000, 200, 25000, 700, 1000, 1500) should be named constants.
