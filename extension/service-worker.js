// reIDM Integration — background service worker.
//
// Jobs:
//   1. Take over the downloads the browser starts, so that reIDM — not the browser — downloads
//      them. This is the extension's main job and most of this file.
//   2. Take over download links the page's content script intercepted on click, before the
//      browser has requested anything.
//   3. Watch network requests for media (HLS/DASH manifests, progressive video/audio) so the
//      popup and the quality picker can offer them, including inside cross-origin iframes.
//   4. Answer the popup, the resource grabber page and the context menus.
//
// How a browser download is taken over (the same mechanism Tonec's own extension uses; see
// `background.js` 6.43.1: `onCreated` pauses, `onDeterminingFilename` decides, then
// `downloads.cancel()` for "taken", `downloads.resume()` otherwise):
//
//   * Chrome does not show a download in its download bubble or shelf, does not open a "Save as"
//     dialog, and cannot finish writing the file, until the file name has been decided. An
//     extension that answers `onDeterminingFilename` asynchronously holds that decision.
//   * So the download is held there and paused, and reIDM is asked to take the URL — with the
//     browser's cookies and user agent, and with `verify`, which makes reIDM fetch the first
//     byte before saying yes.
//   * Only once reIDM has said yes is the browser's copy cancelled and erased; the user never
//     sees the browser download anything. If reIDM says no, or is not running and cannot be
//     started, the download is resumed and released exactly as it was. The failure mode is
//     "the browser downloads it", never "nobody does".
//
// Other architecture notes:
//   * Requests to 127.0.0.1 go out from *here*, never from a content script: a content script's
//     fetch carries the page's origin and would be subject to CORS, while this service worker's
//     requests carry `Origin: chrome-extension://<id>`, which reIDM accepts.
//   * Every listener is registered synchronously at the top level. Chrome wakes a service worker
//     for an event only if the listener was declared in the first run of the script; a listener
//     added after an `await` can miss the very event that woke the worker.

const DEFAULTS = {
  port: 61002,
  capture: true,
  // Take download links on click, before the browser requests them. Only for links whose
  // address ends in one of `fileTypes`, or that carry a `download` attribute.
  interceptClicks: true,
  // Take every download the browser starts. Off: only those whose name matches `fileTypes`.
  captureAll: true,
  // IDM's stock list.
  fileTypes:
    '3GP 7Z AAC ACE AIF APK ARJ ASF AVI BIN BZ2 DEB DMG EXE FLAC GZ GZIP IMG ISO LZH M4A M4V MKV MOV MP3 MP4 ' +
    'MPA MPE MPEG MPG MSI MSU OGG OGV PDF PKG PLJ PPS PPT QT RA RAR RM RMVB RPM SEA SIT SITX TAR TGZ TIF TIFF ' +
    'WAV WEBM WMA WMV XZ Z ZIP ZST',
  // Leave downloads smaller than this (KB) to the browser. 0 = take everything.
  minSizeKB: 0,
  // Never take downloads from these hosts (and their subdomains).
  skipHosts: [],
  captureIncognito: false,
  // Start reIDM through the native host when it is not running.
  launchReidm: true,
  // A desktop notification when reIDM takes a download, or can't.
  notifications: true,
  // IDM's "Download file info" dialog before each download: Start, Download later, Cancel.
  askBeforeDownload: true,
  sniffMedia: true,
  ignore: ['blob:', 'data:', 'chrome:', 'chrome-extension://', 'about:', 'file://', 'filesystem:'],
};

/** Starts at the defaults and is corrected as soon as storage answers; handlers that decide
 *  something important `await settingsReady` first, so a worker woken by a download never
 *  acts on defaults the user has changed. */
const settings = { ...DEFAULTS };

const settingsReady = (async () => {
  try {
    Object.assign(settings, await chrome.storage.sync.get(DEFAULTS));
  } catch {
    /* keep the defaults */
  }
})();
chrome.storage.onChanged.addListener((changes, area) => {
  if (area !== 'sync') return;
  for (const [k, v] of Object.entries(changes)) settings[k] = v.newValue;
});

const VERSION = chrome.runtime.getManifest().version;
const NATIVE_HOST = 'com.zektopic.reidm';
const api = (path) => `http://127.0.0.1:${settings.port}${path}`;
const isHttp = (url) => /^https?:/i.test(url || '');
const isIgnored = (url) => !url || (settings.ignore || []).some((p) => url.startsWith(p));

function hostOf(url) {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return '';
  }
}

/** `a.example.com` is skipped by an entry `example.com`. */
function hostSkipped(url) {
  const h = hostOf(url);
  return !!h && (settings.skipHosts || []).some((s) => {
    const e = String(s).trim().toLowerCase().replace(/^\*\./, '');
    return e && (h === e || h.endsWith(`.${e}`));
  });
}

function extOf(name) {
  const m = /\.([a-z0-9]{1,8})$/i.exec(String(name || '').split(/[?#]/)[0]);
  return m ? m[1].toUpperCase() : '';
}

function typeListed(name) {
  const ext = extOf(name);
  if (!ext) return false;
  const list = String(settings.fileTypes || '').toUpperCase().split(/[\s,;|]+/).filter(Boolean);
  return list.some((t) => (t.endsWith('*') ? ext.startsWith(t.slice(0, -1)) : t === ext));
}

/** A sensible file name for a URL, so reIDM does not have to guess. */
function nameFromUrl(url) {
  try {
    const u = new URL(url);
    const last = decodeURIComponent(u.pathname.split('/').filter(Boolean).pop() || '');
    return last || null;
  } catch {
    return null;
  }
}

const baseName = (p) => (p ? String(p).split(/[\\/]/).pop() || null : null);

// ── talking to reIDM ────────────────────────────────────────────────────

/** The last thing we know about reIDM, mirrored to `storage.local` so content scripts can
 *  decide synchronously whether to intercept a click. */
let status = { up: false, version: null, at: 0 };

function publishStatus(next) {
  status = { ...next, at: Date.now() };
  chrome.storage.local.set({ 'reidm-status': status }).catch(() => {});
}

async function ping() {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), 3000);
  try {
    const r = await fetch(api(`/ext/ping?v=${encodeURIComponent(VERSION)}`), { cache: 'no-store', signal: ctl.signal });
    if (!r.ok) return null;
    const info = await r.json();
    publishStatus({ up: true, version: info.version || null, media: !!info.media, dir: info.default_dir || null, folders: info.folders || null });
    return info;
  } catch {
    publishStatus({ up: false, version: null });
    return null;
  } finally {
    clearTimeout(t);
  }
}

/** POST to reIDM. Never throws: `{ok:false, unreachable?, error}` on any failure. */
async function post(path, body, timeoutMs = 25000) {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const r = await fetch(api(path), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
      signal: ctl.signal,
    });
    const j = await r.json().catch(() => ({}));
    if (!status.up) publishStatus({ ...status, up: true });
    if (!r.ok) return { ok: false, error: j.error || `HTTP ${r.status}` };
    return j;
  } catch (e) {
    if (e && e.name === 'AbortError') return { ok: false, error: 'reIDM did not answer in time' };
    publishStatus({ up: false, version: null });
    return { ok: false, unreachable: true, error: 'reIDM is not running' };
  } finally {
    clearTimeout(t);
  }
}

/** Start reIDM through the native host. Resolves to whether it is now reachable. One launch
 *  at a time: a burst of downloads must not start a burst of daemons. */
let launching = null;
function launchReidm() {
  if (!settings.launchReidm || !chrome.runtime.sendNativeMessage) return Promise.resolve(false);
  if (!launching) {
    launching = (async () => {
      try {
        const r = await chrome.runtime.sendNativeMessage(NATIVE_HOST, { cmd: 'launch' });
        if (!(r && r.ok)) return false;
        return !!(await ping());
      } catch {
        return false; // host not registered: reIDM was installed without it, or not at all
      } finally {
        setTimeout(() => (launching = null), 2000);
      }
    })();
  }
  return launching;
}

/** Hand one URL to reIDM, starting reIDM first if it is not running. */
async function send(details) {
  let r = await post('/ext/add', details);
  if (!r.ok && r.unreachable && (await launchReidm())) r = await post('/ext/add', details);
  return r;
}

async function sendBatch(items, referer, later = false) {
  const withCookies = await Promise.all(
    items.map(async (it) => ({ ...it, referer: it.referer || referer, cookie: await cookieHeader(it.url) })),
  );
  const body = { items: withCookies, user_agent: navigator.userAgent, later: !!later };
  let r = await post('/ext/add-batch', body, 60000);
  if (!r.ok && r.unreachable && (await launchReidm())) r = await post('/ext/add-batch', body, 60000);
  return r;
}

/** The `Cookie` header the browser itself would send for `url`, so reIDM can fetch what the
 *  signed-in user can. Read only at the moment of a hand-off, for that one URL. */
async function cookieHeader(url, storeId) {
  if (!chrome.cookies || !isHttp(url)) return undefined;
  try {
    const list = await chrome.cookies.getAll(storeId ? { url, storeId } : { url });
    if (!list.length) return undefined;
    // Longer paths first, as browsers send them.
    list.sort((a, b) => b.path.length - a.path.length);
    return list.map((c) => `${c.name}=${c.value}`).join('; ');
  } catch {
    return undefined;
  }
}

// ── things the browser must keep ────────────────────────────────────────

/** Form submissions that turned into downloads. reIDM would fetch the URL with GET, which the
 *  server would answer differently (or not at all), so these stay with the browser. */
const recentPosts = new Map(); // url -> time

/** URLs the user asked the browser to handle itself: Alt+click, or a hand-off reIDM refused
 *  that the content script then gave back to the browser. */
const bypass = new Map(); // url -> expiry

function prune(map, maxAgeMs) {
  const now = Date.now();
  for (const [k, t] of map) if (now - t > maxAgeMs) map.delete(k);
}

chrome.webRequest.onBeforeRequest.addListener(
  (d) => {
    if (d.method && d.method !== 'GET') {
      recentPosts.set(d.url, Date.now());
      if (recentPosts.size > 200) prune(recentPosts, 120000);
    }
  },
  { urls: ['http://*/*', 'https://*/*'], types: ['main_frame', 'sub_frame'] },
);

function bypassed(...urls) {
  const now = Date.now();
  for (const u of urls) {
    if (!u) continue;
    const until = bypass.get(u);
    if (until && until > now) {
      bypass.delete(u);
      return true;
    }
  }
  return false;
}

// ── 1. taking over browser downloads ────────────────────────────────────

/** In-flight hand-offs, so a worker torn down mid-decision can be reconciled. */
const PENDING_KEY = 'reidm-pending-captures';

async function pendingUpdate(fn) {
  try {
    const p = (await chrome.storage.session.get(PENDING_KEY))[PENDING_KEY] || {};
    fn(p);
    await chrome.storage.session.set({ [PENDING_KEY]: p });
  } catch {
    /* no session storage; the reconcile step simply does nothing */
  }
}

/** Why a download should stay with the browser, or null to take it. */
function keepReason(item) {
  if (!settings.capture) return 'capture is off';
  if (item.byExtensionId) return 'started by an extension'; // including our own fall-backs
  if (item.state && item.state !== 'in_progress') return 'not a new download';
  const url = item.finalUrl || item.url;
  if (!isHttp(item.url) || !isHttp(url)) return 'made by the page itself (blob/data), nothing to hand over';
  if (isIgnored(item.url) || isIgnored(url)) return 'ignored address';
  if (item.incognito && !settings.captureIncognito) return 'incognito';
  if (hostSkipped(item.url) || hostSkipped(url)) return 'site is on the never-capture list';
  if (bypassed(item.url, item.finalUrl)) return 'the user asked the browser to keep it';
  const posted = recentPosts.get(item.url) || recentPosts.get(item.finalUrl);
  if (posted && Date.now() - posted < 120000) return 'a form submission (POST) cannot be repeated by reIDM';
  const kb = Number(settings.minSizeKB) || 0;
  if (kb > 0 && item.totalBytes > 0 && item.totalBytes < kb * 1024) return 'smaller than the minimum size';
  if (!settings.captureAll && !typeListed(item.filename) && !typeListed(url)) return 'file type not on the list';
  return null;
}

const callback = (fn) => new Promise((resolve) => fn(() => resolve(!chrome.runtime.lastError)));

/** The core hand-off. `suggest` releases Chrome's filename decision; it must be called exactly
 *  once, and — when reIDM took the download — only after the browser's copy is cancelled, or
 *  Chrome would carry on and open its "Save as" dialog. */
async function takeOver(item, suggest) {
  let released = false;
  const release = () => {
    if (released) return;
    released = true;
    try {
      suggest();
    } catch {
      /* the download is gone already */
    }
  };
  try {
    await settingsReady;
    const why = keepReason(item);
    if (why) {
      console.debug(`reIDM: leaving download ${item.id} to the browser: ${why}`);
      return release();
    }

    // The reversible half first: stop the transfer while reIDM is asked.
    const paused = await callback((done) => chrome.downloads.pause(item.id, done));
    await pendingUpdate((p) => (p[item.id] = { paused, at: Date.now() }));

    const url = isHttp(item.finalUrl) ? item.finalUrl : item.url;
    const store = item.incognito ? 'incognito' : undefined;
    const details = {
      url,
      filename: baseName(item.filename) || nameFromUrl(url),
      referer: item.referrer || undefined,
      cookie: await cookieHeader(url, store),
      user_agent: navigator.userAgent,
      verify: true,
    };
    // The browser's copy stays paused, and its "Save as" unopened, while the user decides.
    const choice = await maybeAsk({ url, filename: details.filename, size: item.totalBytes > 0 ? item.totalBytes : null, incognito: item.incognito, nameFromServer: false });
    if (choice.action === 'cancel') {
      // IDM's Cancel: nobody downloads it, neither reIDM nor the browser.
      await callback((done) => chrome.downloads.cancel(item.id, done));
      await callback((done) => chrome.downloads.erase({ id: item.id }, done));
      await pendingUpdate((p) => delete p[item.id]);
      release();
      recordCapture({ url, filename: details.filename, error: 'cancelled' });
      return;
    }
    details.filename = choice.filename || details.filename;
    if (choice.action === 'later') details.later = true;
    let r = await send(details);
    // A redirect's target can be bound to the request that produced it; the original address
    // is worth one more try before giving up.
    if (!r.ok && !r.unreachable && item.url !== url && isHttp(item.url)) {
      r = await send({ ...details, url: item.url, cookie: await cookieHeader(item.url, store) });
    }
    await pendingUpdate((p) => delete p[item.id]);

    choice.report(r);
    if (r.ok) {
      // reIDM has it, and has already fetched its first byte: the browser's copy is redundant.
      await callback((done) => chrome.downloads.cancel(item.id, done));
      await callback((done) => chrome.downloads.erase({ id: item.id }, done));
      release();
      recordCapture({ url, filename: details.filename, outcome: r.outcome, id: r.id });
      badge(r.outcome === 'already_complete' ? '=' : '✓', '#2ea053', 2500);
      announce(r, details.filename || nameFromUrl(url));
      return;
    }
    if (paused) await callback((done) => chrome.downloads.resume(item.id, done));
    release();
    recordCapture({ url, filename: details.filename, error: r.error || 'refused' });
    notify('the browser kept the download', r.error || 'reIDM refused it');
  } catch (e) {
    console.warn('reIDM: hand-off failed', e);
    try {
      chrome.downloads.resume(item.id, () => void chrome.runtime.lastError);
    } catch {
      /* ignore */
    }
    release();
  }
}

if (chrome.downloads.onDeterminingFilename) {
  chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
    takeOver(item, suggest);
    return true; // answered asynchronously, by `takeOver`
  });
} else {
  // A browser without `onDeterminingFilename` (not Chromium): pause on creation instead. Small
  // files may finish before the pause lands there; the browser then keeps its copy too.
  chrome.downloads.onCreated.addListener((item) => {
    if (item.state === 'in_progress') takeOver(item, () => {});
  });
}

/** A decision that never arrived must not leave a download stuck paused. */
async function reconcilePending() {
  const stale = [];
  await pendingUpdate((p) => {
    for (const [id, e] of Object.entries(p)) {
      if (Date.now() - e.at < 60000) continue; // still in flight
      if (e.paused) stale.push(Number(id));
      delete p[id];
    }
  });
  for (const id of stale) chrome.downloads.resume(id, () => void chrome.runtime.lastError);
}

/** Recent hand-offs, for the popup's "what just happened". */
const CAPTURES_KEY = 'reidm-recent-captures';
async function recordCapture(entry) {
  try {
    const list = (await chrome.storage.session.get(CAPTURES_KEY))[CAPTURES_KEY] || [];
    list.unshift({ ...entry, at: Date.now() });
    await chrome.storage.session.set({ [CAPTURES_KEY]: list.slice(0, 20) });
  } catch {
    /* best effort */
  }
}

function notify(title, message) {
  console.warn(`reIDM: ${title}: ${message}`);
  badge('!', '#c02828', 4000);
  toast(`reIDM: ${title}`, message);
}

/** A desktop notification, unless the user turned them off. Never throws. */
function toast(title, message, id) {
  if (!settings.notifications || !chrome.notifications) return;
  try {
    chrome.notifications.create(
      id || `reidm-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      { type: 'basic', iconUrl: 'icons/128.png', title, message: String(message || '').slice(0, 300), priority: 0, silent: true },
      () => void chrome.runtime.lastError,
    );
  } catch {
    /* notifications are a courtesy */
  }
}

/** Tell the user reIDM has a download: the browser's own download bubble no longer appears for
 *  what reIDM takes, so this is how they know it started. */
function announce(r, name) {
  if (!r || !r.ok) return;
  const what = name || 'the download';
  if (r.outcome === 'already_complete') toast('Already downloaded', `${what} is already in reIDM, finished.`, `reidm-${r.id}`);
  else if (r.outcome === 'existing') toast('reIDM carries on', `${what} was already in reIDM; it continues from where it stopped.`, `reidm-${r.id}`);
  else toast('Downloading in reIDM', what, `reidm-${r.id}`);
}

let badgeTimer = null;
function badge(text, color, clearAfterMs) {
  chrome.action.setBadgeText({ text }).catch(() => {});
  chrome.action.setBadgeBackgroundColor({ color }).catch(() => {});
  if (badgeTimer) clearTimeout(badgeTimer);
  if (clearAfterMs) badgeTimer = setTimeout(() => chrome.action.setBadgeText({ text: '' }).catch(() => {}), clearAfterMs);
}

// ── the "Download file info" dialog ─────────────────────────────────────

/** Dialogs waiting for an answer: id -> { req, resolve, port }. */
const asking = new Map();
let askSeq = 0;

/** The folder reIDM will save `name` in, as far as the extension knows: its category's folder
 *  when reIDM sorts by category, else the download folder. */
function folderFor(name) {
  const f = status.folders;
  if (f && f.sort && f.dirs) {
    const ext = String(name || '').split('.').pop().toLowerCase();
    for (const [cat, exts] of Object.entries(f.types || {})) if (exts.includes(ext) && f.dirs[cat]) return f.dirs[cat];
    if (f.dirs.General) return f.dirs.General;
  }
  return status.dir || null;
}

/** The first byte of `url`, for its size and the server's file name. The browser's cookies go
 *  with it, as they would with the download. Never for incognito: that would use the normal
 *  profile's cookies. */
async function probeSize(url) {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), 8000);
  try {
    const r = await fetch(url, { headers: { Range: 'bytes=0-0' }, credentials: 'include', cache: 'no-store', signal: ctl.signal });
    const total = /\/(\d+)\s*$/.exec(r.headers.get('content-range') || '');
    const size = total ? Number(total[1]) : r.status === 200 ? Number(r.headers.get('content-length')) || null : null;
    const cd = r.headers.get('content-disposition') || '';
    const m = /filename\*=(?:UTF-8'')?([^;]+)/i.exec(cd) || /filename="?([^";]+)"?/i.exec(cd);
    let filename = null;
    try {
      filename = m ? decodeURIComponent(m[1].trim().replace(/^"|"$/g, '')) : null;
    } catch {
      filename = m ? m[1] : null;
    }
    ctl.abort();
    return { size, filename };
  } catch {
    return { size: null, filename: null };
  } finally {
    clearTimeout(t);
  }
}

/**
 * Ask the user first, as IDM's "Download file info" dialog does: what is about to be
 * downloaded, where, and how big, with Start download, Download later and Cancel.
 * `req`: { url, filename, size?, sizeNote?, probe?: false, incognito? }. Resolves to
 * { action: 'start' | 'later' | 'cancel', filename, report(result) }; `report` closes the dialog,
 * or shows reIDM's refusal in it.
 */
async function askUser(req) {
  const id = `${Date.now().toString(36)}-${++askSeq}`;
  const answer = new Promise((resolve) => asking.set(id, { req, resolve, port: null }));
  const width = 560;
  const height = 380;
  const opts = { url: chrome.runtime.getURL(`confirm.html?id=${id}`), type: 'popup', width, height, focused: true };
  const w = await chrome.windows.getLastFocused().catch(() => null);
  if (w && w.width && w.left != null) {
    opts.left = Math.max(0, Math.round(w.left + (w.width - width) / 2));
    opts.top = Math.max(0, Math.round(w.top + (w.height - height) / 3));
  }
  try {
    await chrome.windows.create(opts);
  } catch (e) {
    // No window can be opened (a kiosk, a policy): don't hold the download hostage to it.
    console.warn('reIDM: could not open the download dialog', e);
    asking.delete(id);
    return { action: 'start', filename: req.filename, report: () => {} };
  }
  return answer;
}

chrome.runtime.onConnect.addListener((port) => {
  if (!port.name.startsWith('confirm:')) return;
  const id = port.name.slice('confirm:'.length);
  const a = asking.get(id);
  if (!a) {
    port.postMessage({ type: 'error', error: 'This download is no longer waiting. Start it again from the page.' });
    return;
  }
  a.port = port;
  const report = (r) => {
    try {
      if (r && r.ok) port.postMessage({ type: 'done' });
      else port.postMessage({ type: 'error', error: `reIDM could not take it: ${(r && r.error) || 'unknown error'}` });
    } catch {
      /* the dialog was closed */
    }
  };
  port.onMessage.addListener(async (m) => {
    if (m.type === 'hello') {
      const r = a.req;
      port.postMessage({ type: 'info', url: r.url, filename: r.filename, size: r.size, sizeNote: r.sizeNote, dir: folderFor(r.filename) });
      if (!(r.size > 0) && r.probe !== false && !r.incognito && isHttp(r.url)) {
        const p = await probeSize(r.url);
        try {
          port.postMessage({ type: 'size', size: p.size, note: 'unknown — the server does not say', filename: r.nameFromServer ? p.filename : null });
        } catch {
          /* closed meanwhile */
        }
      } else if (!(r.size > 0)) {
        port.postMessage({ type: 'size', size: null, note: r.sizeNote || 'unknown' });
      }
    } else if (m.type === 'answer') {
      if (m.noAsk) {
        settings.askBeforeDownload = false;
        chrome.storage.sync.set({ askBeforeDownload: false }).catch(() => {});
      }
      if (!asking.has(id)) return;
      asking.delete(id);
      a.resolve({ action: m.action, filename: m.filename || a.req.filename, report });
    }
  });
  port.onDisconnect.addListener(() => {
    // Closing the window is Cancel, as in IDM.
    if (asking.has(id)) {
      asking.delete(id);
      a.resolve({ action: 'cancel', filename: a.req.filename, report: () => {} });
    }
  });
});

/** Ask when the user wants to be asked; otherwise the answer is "start", with nothing to report to. */
async function maybeAsk(req) {
  await settingsReady;
  if (!settings.askBeforeDownload) return { action: 'start', filename: req.filename, report: () => {} };
  return askUser(req);
}

// ── 2. links the content script took on click ───────────────────────────

/** A link the user clicked, which the content script stopped before the browser requested
 *  it. reIDM checks it first; `refuse_html` makes it decline a "file" link that actually
 *  answers with a web page (a login wall, a download page), which the browser should then open. */
async function captureLink(msg, sender) {
  await settingsReady;
  const url = msg.url;
  if (!settings.capture || !isHttp(url) || isIgnored(url) || hostSkipped(url)) return { ok: false, error: 'not captured' };
  const incognito = !!(sender.tab && sender.tab.incognito);
  if (incognito && !settings.captureIncognito) return { ok: false, error: 'incognito' };
  let filename = msg.filename || nameFromUrl(url) || undefined;
  const choice = await maybeAsk({ url, filename, incognito, nameFromServer: !msg.filename });
  // Cancelled: the click is used up and nothing downloads. `ok` so the page doesn't hand the
  // click back to the browser.
  if (choice.action === 'cancel') return { ok: true, cancelled: true };
  filename = choice.filename || filename;
  const r = await send({
    url,
    filename,
    referer: msg.referer || (sender.tab && sender.tab.url) || undefined,
    cookie: await cookieHeader(url, incognito ? 'incognito' : undefined),
    user_agent: navigator.userAgent,
    verify: true,
    refuse_html: true,
    later: choice.action === 'later',
  });
  choice.report(r);
  if (r.ok) {
    recordCapture({ url, filename, outcome: r.outcome, id: r.id });
    badge(r.outcome === 'already_complete' ? '=' : '✓', '#2ea053', 2500);
    announce(r, filename || nameFromUrl(url));
  } else {
    recordCapture({ url, filename, error: r.error || 'refused' });
  }
  return r;
}

// ── 3. media sniffing ───────────────────────────────────────────────────

const MEDIA_EXT = /\.(m3u8|mpd|mp4|m4v|webm|mkv|mov|flv|mp3|m4a|flac|ogg|opus|wav|aac)(\?|#|$)/i;
const MANIFEST_EXT = /\.(m3u8|mpd)(\?|#|$)/i;
/** Pieces of a stream, which are useless on their own and would bury the real entries. */
const SEGMENT = /\.(ts|m4s|cmfv|cmfa)(\?|#|$)|[/_-](seg|segment|chunk|frag|fragment)[-_]?\d+|\/range\/\d+-\d+/i;
const MANIFEST_MIME = /mpegurl|dash\+xml/i;
const MEDIA_MIME = /^(video|audio)\//i;
/** YouTube's own player traffic: its formats come through the quality picker instead. */
const SKIP_MEDIA_HOST = /(^|\.)googlevideo\.com$/i;

/** tabId -> Map(url -> {url, kind, at, mime?, size?, formats?}) */
const mediaByTab = new Map();

// An MV3 service worker is stopped whenever Chrome likes, and module-scope state goes with it.
// The per-tab media index is mirrored to session storage, which lives exactly as long as "a page
// that is still open" can.
const MEDIA_KEY = 'reidm-media-by-tab';
let persistTimer = null;

function persistMedia() {
  if (persistTimer) return;
  persistTimer = setTimeout(async () => {
    persistTimer = null;
    try {
      const obj = {};
      for (const [tab, map] of mediaByTab) obj[tab] = [...map.values()];
      await chrome.storage.session.set({ [MEDIA_KEY]: obj });
    } catch {
      /* storage unavailable; the in-memory map still works for this worker's lifetime */
    }
  }, 250);
}

async function restoreMedia() {
  try {
    const o = (await chrome.storage.session.get(MEDIA_KEY))[MEDIA_KEY] || {};
    for (const [tab, list] of Object.entries(o)) {
      if (!mediaByTab.has(Number(tab))) mediaByTab.set(Number(tab), new Map(list.map((e) => [e.url, e])));
    }
  } catch {
    /* nothing to restore */
  }
}

/** Every reader awaits this: a worker that has just been woken would otherwise report an empty
 *  list for a page that is plainly playing video. */
const mediaReady = restoreMedia();

// ── manifest parsing, in the worker ─────────────────────────────────────
//
// The MAIN-world hook also parses manifests, but it can lose a race: a page that fetches its
// manifest from an inline <script> does so before the hook exists. `webRequest` sees the request
// regardless, so the worker re-fetches the manifest itself (it has host permissions).

function codecNames(codecs) {
  const c = String(codecs || '').toLowerCase();
  return {
    video: c.includes('av01') ? 'AV1'
      : c.includes('vp9') || c.includes('vp09') ? 'VP9'
      : c.includes('avc1') || c.includes('h264') ? 'H.264'
      : c.includes('hev1') || c.includes('hvc1') ? 'H.265' : null,
    audio: c.includes('opus') ? 'Opus' : c.includes('mp4a') ? 'AAC' : null,
  };
}

function parseHLS(body, baseUrl) {
  const out = [];
  const lines = body.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line.startsWith('#EXT-X-STREAM-INF')) continue;
    const res = /RESOLUTION=(\d+)x(\d+)/i.exec(line);
    const bw = /BANDWIDTH=(\d+)/i.exec(line);
    const codecs = /CODECS="([^"]+)"/i.exec(line);
    const name = /NAME="([^"]+)"/i.exec(line);
    let uri = null;
    for (let k = i + 1; k < lines.length; k++) {
      const t = lines[k].trim();
      if (t && !t.startsWith('#')) { uri = t; break; }
    }
    const n = codecNames(codecs && codecs[1]);
    out.push({
      quality: name ? name[1] : res ? `${res[2]}p` : null,
      width: res ? Number(res[1]) : null,
      height: res ? Number(res[2]) : null,
      bitrate: bw ? Number(bw[1]) : null,
      videoCodec: n.video,
      audioCodec: n.audio,
      codecs: codecs ? codecs[1] : null,
      url: uri ? new URL(uri, baseUrl).href : null,
      kind: 'hls-variant',
    });
  }
  return out;
}

function parseDASH(body, baseUrl) {
  const out = [];
  const re = /<Representation\b([^>]*)>([\s\S]*?)<\/Representation>/gi;
  let m;
  while ((m = re.exec(body))) {
    const attr = (n) => {
      const r = new RegExp(`${n}="([^"]*)"`, 'i').exec(m[1]);
      return r ? r[1] : null;
    };
    const base = /<BaseURL>([^<]+)<\/BaseURL>/i.exec(m[2]);
    const n = codecNames(attr('codecs'));
    out.push({
      quality: attr('height') ? `${attr('height')}p` : attr('id'),
      width: attr('width') ? Number(attr('width')) : null,
      height: attr('height') ? Number(attr('height')) : null,
      bitrate: attr('bandwidth') ? Number(attr('bandwidth')) : null,
      mime: attr('mimeType'),
      codecs: attr('codecs'),
      videoCodec: n.video,
      audioCodec: n.audio,
      url: base ? new URL(base[1], baseUrl).href : baseUrl.split('#')[0],
      kind: 'dash-representation',
    });
  }
  return out;
}

/** Re-fetch a manifest and return its variants. Resolves to [] on anything unexpected. */
async function formatsForManifest(url) {
  try {
    const r = await fetch(url, { credentials: 'include', cache: 'no-store' });
    if (!r.ok) return [];
    const body = (await r.text()).slice(0, 2 * 1024 * 1024);
    if (body.includes('#EXTM3U')) return parseHLS(body, url);
    if (body.includes('<MPD') || body.includes('<Representation')) return parseDASH(body, url);
  } catch {
    /* a signed, POST-only or otherwise unfetchable URL; the page hook may still have it */
  }
  return [];
}

function record(tabId, url, kind, extra = {}) {
  if (tabId === undefined || tabId < 0 || !isHttp(url) || isIgnored(url)) return;
  if (!mediaByTab.has(tabId)) mediaByTab.set(tabId, new Map());
  const tab = mediaByTab.get(tabId);
  const prev = tab.get(url);
  if (prev) {
    // A later sighting may know more (a size, a MIME type); keep the first sighting's time.
    let changed = false;
    for (const [k, v] of Object.entries(extra)) {
      if (v != null && prev[k] == null) {
        prev[k] = v;
        changed = true;
      }
    }
    if (changed) persistMedia();
    return;
  }
  tab.set(url, { url, kind, at: Date.now(), ...extra });
  // Keep the per-tab list from growing without bound on a long-running player.
  if (tab.size > 200) {
    const oldest = [...tab.entries()].sort((a, b) => a[1].at - b[1].at)[0];
    tab.delete(oldest[0]);
  }
  badge(String(tab.size), '#0a60c8', null);
  persistMedia();
}

function noteManifest(tabId, url, extra) {
  record(tabId, url, 'stream', extra);
  // Fetch and parse it out-of-band; the picker can then offer real variants even if the page
  // fetched the manifest before our hook existed.
  formatsForManifest(url).then(async (formats) => {
    if (!formats.length) return;
    await mediaReady;
    const tab = mediaByTab.get(tabId);
    const entry = tab && tab.get(url);
    if (entry) {
      entry.formats = formats;
      persistMedia();
    }
  });
}

chrome.webRequest.onBeforeRequest.addListener(
  (d) => {
    if (!settings.sniffMedia || SKIP_MEDIA_HOST.test(hostOf(d.url))) return;
    if (MANIFEST_EXT.test(d.url)) noteManifest(d.tabId, d.url);
    else if (MEDIA_EXT.test(d.url) && !SEGMENT.test(d.url)) record(d.tabId, d.url, 'media');
  },
  { urls: ['http://*/*', 'https://*/*'] },
);

// Many players fetch media from addresses with no recognisable extension. The response's own
// Content-Type says what it is, and Content-Length/Content-Range how big.
chrome.webRequest.onHeadersReceived.addListener(
  (d) => {
    if (!settings.sniffMedia || d.tabId < 0 || SKIP_MEDIA_HOST.test(hostOf(d.url))) return;
    if (d.statusCode >= 400 || SEGMENT.test(d.url)) return;
    let mime = '';
    let size = null;
    for (const h of d.responseHeaders || []) {
      const n = h.name.toLowerCase();
      if (n === 'content-type') mime = String(h.value || '').split(';')[0].trim().toLowerCase();
      else if (n === 'content-range') {
        const m = /\/(\d+)\s*$/.exec(h.value || '');
        if (m) size = Number(m[1]);
      } else if (n === 'content-length' && size == null && d.statusCode === 200) size = Number(h.value) || null;
    }
    if (MANIFEST_MIME.test(mime)) noteManifest(d.tabId, d.url, { mime });
    else if (MEDIA_MIME.test(mime) && mime !== 'video/mp2t') {
      // Tiny responses are a player's probes and preview sprites, not something to download.
      if (size == null || size > 64 * 1024) record(d.tabId, d.url, mime.startsWith('audio/') ? 'audio' : 'video', { mime, size });
    }
  },
  { urls: ['http://*/*', 'https://*/*'], types: ['media', 'xmlhttprequest', 'other', 'object', 'sub_frame'] },
  ['responseHeaders'],
);

chrome.tabs.onRemoved.addListener((tabId) => {
  mediaByTab.delete(tabId);
  persistMedia();
});
chrome.tabs.onUpdated.addListener((tabId, info) => {
  if (info.status === 'loading' && info.url) {
    // A new document in the tab: the previous page's media is no longer relevant.
    mediaByTab.delete(tabId);
    persistMedia();
    chrome.action.setBadgeText({ text: '' }).catch(() => {});
  }
});

// ── 3b. handing a chosen video format to reIDM ──────────────────────────

/** A safe-ish file name: "Title 1080p.mp4". reIDM sanitises it again on its side. */
function formatFilename(title, format, url) {
  const ext = (() => {
    const m = /^video\/(\w+)|^audio\/(\w+)/.exec(format.mime || '');
    if (m) return (m[1] || m[2]).replace('mpegurl', 'm3u8');
    const u = /\.([a-z0-9]{2,4})(?:\?|#|$)/i.exec(url || '');
    return u ? u[1] : 'mp4';
  })();
  const base = (title || 'video').replace(/[/\\?%*:|"<>]/g, '_').replace(/\s+/g, ' ').trim().slice(0, 120);
  const q = format.quality ? ` ${format.quality}` : '';
  const a = format.audioOnly ? ' (audio)' : '';
  return `${base}${q}${a}.${ext}`;
}

/**
 * The yt-dlp format selector for a format picked in the list: that exact format when yt-dlp
 * offers it (its list is not always the page's), else the best at the same height — H.264 with
 * AAC first, which plays anywhere. A format without sound gets the best audio joined to it.
 * With nothing picked, the best there is.
 */
function ytdlpFormat(f = {}) {
  const id = Number.isInteger(f.itag) ? String(f.itag) : null;
  if (f.audioOnly) return [id, 'bestaudio[ext=m4a]', 'bestaudio'].filter(Boolean).join('/');
  const h = Number(f.height) || parseInt(f.quality, 10) || 0;
  const cap = h ? `[height<=${h}]` : '';
  const alts = [];
  if (id) alts.push(...(f.hasAudio ? [id] : [`${id}+bestaudio[ext=m4a]`, `${id}+bestaudio`]));
  alts.push(`bestvideo${cap}[vcodec^=avc1]+bestaudio[ext=m4a]`, `bestvideo${cap}+bestaudio`, `best${cap}`);
  return alts.join('/');
}

/** Whether the reIDM that answers can fetch videos through yt-dlp (0.5 and later); starts it
 *  if it is not running. An older one would save the video's web page instead. */
async function reidmResolves() {
  let info = await ping();
  if (!info && (await launchReidm())) info = await ping();
  if (!info) return { ok: false, error: 'reIDM is not running' };
  if (!info.media) return { ok: false, error: `downloading videos from this site needs reIDM 0.5 or later (this is ${info.version || 'an older one'}); update reIDM` };
  return { ok: true };
}

/** A video on a page that only yt-dlp can resolve (YouTube): hand reIDM the page and a format. */
async function downloadVideoPage(page, title, f, incognito) {
  const can = await reidmResolves();
  if (!can.ok) return can;
  const proposed = f && (f.itag != null || f.quality) ? formatFilename(title, f, page) : undefined;
  const choice = await maybeAsk({
    url: page,
    filename: proposed || `${title || 'video'}.mp4`,
    size: f && f.size > 0 ? f.size : null,
    sizeNote: 'known once the download starts',
    probe: false,
  });
  if (choice.action === 'cancel') return { ok: true, cancelled: true };
  const r = await send({
    url: page,
    filename: choice.filename !== (proposed || `${title || 'video'}.mp4`) ? choice.filename : proposed,
    later: choice.action === 'later',
    referer: page,
    // The signed-in user's cookies let yt-dlp fetch what they can watch (age checks, members).
    cookie: await cookieHeader(page, incognito ? 'incognito' : undefined),
    user_agent: navigator.userAgent,
    media: { format: ytdlpFormat(f) },
  });
  choice.report(r);
  if (r.ok) {
    badge('✓', '#2ea053', 2500);
    const q = f && f.quality ? ` (${f.quality})` : '';
    announce(r, `${title || 'the video'}${q}`);
  } else notify('reIDM could not take the video', r.error || 'unknown error');
  return r;
}

const isYouTube = (u) => /^https?:\/\/([a-z0-9-]+\.)*(youtube\.com|youtube-nocookie\.com|youtu\.be)\//i.test(u || '');

/**
 * The user picked a format. On YouTube it goes through reIDM's yt-dlp; anywhere else a format
 * with an address is just a download.
 */
async function downloadFormat(msg, sender) {
  const f = msg.format || {};
  const incognito = !!(sender.tab && sender.tab.incognito);
  if (msg.source === 'youtube' || (!msg.url && isYouTube(msg.page))) {
    return downloadVideoPage(msg.page, msg.title, f, incognito);
  }
  if (!msg.url) return { ok: false, error: 'this format has no address to download from' };
  const choice = await maybeAsk({ url: msg.url, filename: formatFilename(msg.title, f, msg.url), size: f.size > 0 ? f.size : null, incognito });
  if (choice.action === 'cancel') return { ok: true, cancelled: true };
  const r = await send({
    url: msg.url,
    filename: choice.filename || formatFilename(msg.title, f, msg.url),
    referer: msg.page || undefined,
    cookie: await cookieHeader(msg.url, incognito ? 'incognito' : undefined),
    user_agent: navigator.userAgent,
    later: choice.action === 'later',
  });
  choice.report(r);
  if (r.ok) {
    badge('✓', '#2ea053', 2500);
    announce(r, choice.filename || formatFilename(msg.title, f, msg.url));
  } else notify('reIDM could not take the video', r.error || 'unknown error');
  return r;
}

// ── 4. messages ─────────────────────────────────────────────────────────

chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  if (!msg || typeof msg !== 'object') return false;
  const tabId = sender.tab ? sender.tab.id : msg.tabId;

  switch (msg.type) {
    case 'media': {
      for (const m of msg.items || []) {
        record(tabId, m.url, m.kind || 'media', { page: m.page, formats: m.formats });
        // A later report for the same URL may carry the variants the first one lacked.
        if (m.formats && m.formats.length) {
          const tab = mediaByTab.get(tabId);
          const e = tab && tab.get(m.url);
          if (e) {
            e.formats = m.formats;
            persistMedia();
          }
        }
      }
      reply({ ok: true });
      return false;
    }
    case 'formats-for-tab': {
      const want = msg.tabId !== undefined ? msg.tabId : tabId;
      mediaReady.then(() => {
        const entries = [...(mediaByTab.get(want) || new Map()).values()].filter((i) => i.formats && i.formats.length);
        reply({ ok: true, entries });
      });
      return true;
    }
    case 'media-for-tab':
      mediaReady.then(() => reply({ items: [...(mediaByTab.get(msg.tabId) || new Map()).values()] }));
      return true;
    case 'clear-tab':
      mediaByTab.delete(msg.tabId);
      persistMedia();
      chrome.action.setBadgeText({ text: '' }).catch(() => {});
      reply({ ok: true });
      return false;
    case 'capture-link':
      captureLink(msg, sender).then(reply);
      return true;
    case 'bypass':
      // The content script is about to let the browser have this URL.
      if (isHttp(msg.url)) bypass.set(msg.url, Date.now() + 15000);
      prune(bypass, 60000);
      reply({ ok: true });
      return false;
    case 'open-tab': {
      // A `target=_blank` link reIDM declined: open it as the click would have. The tab is the
      // worker's to open because the page's user activation may have lapsed by now.
      if (!isHttp(msg.url)) {
        reply({ ok: false });
        return false;
      }
      bypass.set(msg.url, Date.now() + 15000);
      const t = sender.tab;
      chrome.tabs.create({ url: msg.url, index: t ? t.index + 1 : undefined, openerTabId: t ? t.id : undefined }, () =>
        reply({ ok: !chrome.runtime.lastError }),
      );
      return true;
    }
    case 'add': {
      const d = msg.details || {};
      (async () => reply(await send({ ...d, cookie: d.cookie || (await cookieHeader(d.url)), user_agent: navigator.userAgent })))();
      return true;
    }
    case 'add-batch':
      sendBatch(msg.items || [], msg.referer, msg.later).then((r) => {
        const n = ((r && r.results) || []).filter((x) => x.outcome === 'new').length;
        if (r && r.ok && n) toast(msg.later ? 'Added to reIDM for later' : 'Downloading in reIDM', `${n} download${n === 1 ? '' : 's'} added`);
        reply(r);
      });
      return true;
    case 'download-format':
      downloadFormat(msg, sender).then(reply);
      return true;
    case 'download-video':
      // From the popup: the video on this YouTube page, in the best quality.
      downloadVideoPage(msg.page, msg.title, null, !!msg.incognito).then(reply);
      return true;
    case 'ping':
      ping().then((info) => reply({ ok: !!info, info }));
      return true;
    case 'launch':
      launchReidm().then((ok) => reply({ ok }));
      return true;
    case 'recent-captures':
      chrome.storage.session
        .get(CAPTURES_KEY)
        .then((o) => reply({ items: o[CAPTURES_KEY] || [] }))
        .catch(() => reply({ items: [] }));
      return true;
    case 'open-grabber':
      openGrabber(msg.tabId).then(() => reply({ ok: true }));
      return true;
    default:
      return false;
  }
});

// ── 5. context menus ────────────────────────────────────────────────────

const MENU = {
  link: 'reidm-link',
  page: 'reidm-page',
  media: 'reidm-media',
  selection: 'reidm-selection',
  grab: 'reidm-grab',
};

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({ id: MENU.link, title: 'Download with reIDM', contexts: ['link'] });
    chrome.contextMenus.create({ id: MENU.media, title: 'Download this media with reIDM', contexts: ['video', 'audio', 'image'] });
    chrome.contextMenus.create({ id: MENU.page, title: 'Download all links on this page with reIDM', contexts: ['page'] });
    chrome.contextMenus.create({ id: MENU.grab, title: 'Grab resources from this page with reIDM…', contexts: ['page', 'frame'] });
    chrome.contextMenus.create({ id: MENU.selection, title: 'Download the selected link with reIDM', contexts: ['selection'] });
  });
  ping();
});

async function openGrabber(tabId) {
  if (tabId === undefined) return;
  await chrome.tabs.create({ url: chrome.runtime.getURL(`grabber.html?tab=${tabId}`) });
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const referer = tab && tab.url ? tab.url : undefined;
  const incognito = !!(tab && tab.incognito);

  if (info.menuItemId === MENU.link || info.menuItemId === MENU.media) {
    const url = info.linkUrl || info.srcUrl;
    if (!url) return;
    const choice = await maybeAsk({ url, filename: nameFromUrl(url) || undefined, incognito, nameFromServer: true });
    if (choice.action === 'cancel') return;
    const r = await send({
      url,
      referer,
      filename: choice.filename || nameFromUrl(url) || undefined,
      cookie: await cookieHeader(url, incognito ? 'incognito' : undefined),
      user_agent: navigator.userAgent,
      later: choice.action === 'later',
    });
    choice.report(r);
    flash(r, choice.filename || nameFromUrl(url));
    return;
  }

  if (info.menuItemId === MENU.selection) {
    const url = (info.selectionText || '').trim();
    if (!isHttp(url)) return flash({ ok: false, error: 'the selection is not a link' });
    flash(await send({ url, referer, cookie: await cookieHeader(url), user_agent: navigator.userAgent }), nameFromUrl(url));
    return;
  }

  if (info.menuItemId === MENU.grab) {
    if (tab) openGrabber(tab.id);
    return;
  }

  if (info.menuItemId === MENU.page) {
    const [res] = await chrome.scripting
      .executeScript({
        target: { tabId: tab.id },
        func: () => [...new Set([...document.querySelectorAll('a[href]')].map((a) => a.href).filter((h) => /^https?:/i.test(h)))].slice(0, 2000),
      })
      .catch(() => []);
    const urls = (res && res.result) || [];
    if (!urls.length) return flash({ ok: false, error: 'no links found' });
    const r = await sendBatch(urls.map((url) => ({ url })), referer);
    const queued = (r.results || []).filter((x) => x.outcome === 'new').length;
    flash(r.ok ? { ok: true, queued } : r);
  }
});

function flash(r, name) {
  if (r && r.ok) {
    badge(r.queued ? String(r.queued) : '✓', '#2ea053', 2500);
    if (r.queued != null) toast('Downloading in reIDM', `${r.queued} download${r.queued === 1 ? '' : 's'} added`);
    else announce(r, name);
  } else notify('reIDM could not take it', (r && r.error) || 'unknown error');
}

// ── keeping in touch ────────────────────────────────────────────────────

chrome.alarms.create('reidm-ping', { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((a) => {
  if (a.name === 'reidm-ping') ping();
});

chrome.runtime.onStartup.addListener(() => {
  ping().then((info) => badge(info ? '' : '?', info ? '#2ea053' : '#d98a1f', 3000));
  reconcilePending();
});

// A module service worker can also be woken without `onStartup` (an event wakes it), so do
// these once per worker start as well.
reconcilePending();
ping();

// For DevTools and `tools/capture-test.mjs`: a module worker's functions are not globals.
globalThis.reidm = { ping, launchReidm, status: () => status };
