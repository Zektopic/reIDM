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
//   * Every held download is written down (`storage.local`, one key each) with the identity of
//     the worker holding it. A worker that dies mid-decision releases Chrome's hold on the name
//     (checked in a real browser: Chrome then names the file), but the download stays paused.
//     The next worker to start resumes whatever another worker left behind.
//
// Other architecture notes:
//   * Requests to 127.0.0.1 go out from *here*, never from a content script: a content script's
//     fetch carries the page's origin and would be subject to CORS, while this service worker's
//     requests carry `Origin: chrome-extension://<id>`, which reIDM accepts.
//   * Every listener is registered synchronously at the top level. Chrome wakes a service worker
//     for an event only if the listener was declared in the first run of the script; a listener
//     added after an `await` can miss the very event that woke the worker.
//   * Idle cost matters on weak PCs: the network listeners are filtered in the browser process
//     to media addresses, nothing is written to storage unless it changed, and the page-side
//     hook is registered only while media detection is on.

import './defaults.js';

const DEFAULTS = globalThis.REIDM_DEFAULTS;

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
  // A removed key means "back to the default", not "undefined": a cleared sync store must not
  // switch capture off or lose the port.
  for (const [k, v] of Object.entries(changes)) settings[k] = v.newValue ?? DEFAULTS[k];
  if ('sniffMedia' in changes) applySniffing();
});

const VERSION = chrome.runtime.getManifest().version;
const NATIVE_HOST = 'com.zektopic.reidm';
/** This worker's identity, written next to every download it holds; see `reconcilePending`. */
const INSTANCE = crypto.randomUUID();
const api = (path, port = settings.port) => `http://127.0.0.1:${port}${path}`;
const isHttp = (url) => /^https?:/i.test(url || '');
const isIgnored = (url) => !url || (settings.ignore || []).some((p) => url.startsWith(p));
const errText = (e) => String((e && e.message) || e || 'unknown error');

/** How long a held download waits for an answer in the dialog before the browser gets it. */
const DIALOG_TIMEOUT_MS = 10 * 60 * 1000;
/** How long reIDM may take to accept a link (it checks the first byte within 12 s). */
const ADD_TIMEOUT_MS = 25000;
const BATCH_TIMEOUT_MS = 60000;
/** A POST that turned into a download is remembered this long. */
const POST_MEMORY_MS = 120000;
/** A "let the browser have it" mark lasts this long. */
const BYPASS_MS = 15000;

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

/** Files that run as programs. A page can start one without the user asking, and reIDM saves
 *  it without the browser's download protection, so these always get the dialog. */
const PROGRAM_EXT = new Set('EXE MSI MSIX MSP APPX APPXBUNDLE BAT CMD COM SCR PIF CPL HTA JS JSE VBS VBE WSF WSH PS1 PSM1 REG LNK JAR APK DMG PKG APP DEB RPM APPIMAGE RUN SH'.split(' '));
const isProgram = (name) => PROGRAM_EXT.has(extOf(name));

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
let status = { up: false, version: null };

/** Every write to `storage.local` wakes the `storage.onChanged` listener of every content
 *  script in every frame of every tab, so the status is written only when it changes. */
function publishStatus(next) {
  const same = ['up', 'version', 'media', 'dir'].every((k) => (status[k] ?? null) === (next[k] ?? null))
    && JSON.stringify(status.folders ?? null) === JSON.stringify(next.folders ?? null);
  status = next;
  if (same) return;
  stats.statusWrites++;
  chrome.storage.local.set({ 'reidm-status': status }).catch(() => {});
}

/** Ask reIDM how it is. `port` tests another port without adopting it (the options page). */
async function ping(port) {
  const other = port != null && Number(port) !== Number(settings.port);
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), 3000);
  try {
    const r = await fetch(api(`/ext/ping?v=${encodeURIComponent(VERSION)}`, other ? port : settings.port), { cache: 'no-store', signal: ctl.signal });
    if (!r.ok) return null;
    const info = await r.json();
    if (!other) publishStatus({ up: true, version: info.version || null, media: !!info.media, dir: info.default_dir || null, folders: info.folders || null });
    return info;
  } catch {
    if (!other) publishStatus({ up: false, version: null });
    return null;
  } finally {
    clearTimeout(t);
  }
}

/** POST to reIDM. Never throws: `{ok:false, unreachable?, timedOut?, error}` on any failure. */
async function post(path, body, timeoutMs = ADD_TIMEOUT_MS) {
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
    if (e && e.name === 'AbortError') return { ok: false, timedOut: true, error: 'reIDM did not answer in time' };
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
  // One id for this hand-off, the same on every attempt below: a reIDM that knows it (0.8 and
  // later) answers a repeat with its first answer instead of adding the link a second time.
  const body = { ...details, request_id: details.request_id || crypto.randomUUID() };
  let r = await post('/ext/add', body);
  if (!r.ok && r.unreachable && (await launchReidm())) r = await post('/ext/add', body);
  // A timeout leaves open whether reIDM took the link. Asking once more settles it instead of
  // guessing — guessing "no" would have both reIDM and the browser download it. reIDM answers
  // the repeated `request_id` with what it made of the first; an older reIDM still answers an
  // address it already has with "existing", without checking it again.
  if (!r.ok && r.timedOut) r = await post('/ext/add', body, BATCH_TIMEOUT_MS);
  return r;
}

/**
 * The download reIDM is waiting to give a new address, if any: `{id, name}`. The user armed it in
 * reIDM (*Refresh download address*, then "click the link again in the browser"), and for the
 * next three minutes reIDM tries every link it is handed against it. Asked afresh for each
 * capture, because the once-a-minute ping would miss most of that window. A reIDM older than
 * 0.8 doesn't say, and then nothing changes: no question, and reIDM decides as before.
 */
async function refreshArmed(url) {
  if (!isHttp(url)) return null;
  const a = (await ping())?.refresh_armed;
  const id = a && typeof a === 'object' ? a.id : a;
  if (!Number.isInteger(id)) return null;
  const name = a && typeof a.name === 'string' ? a.name.slice(0, 260) : null;
  return { id, name };
}

/** What `/ext/add` is told about an armed refresh: the user's answer to "Use this link to
 *  refresh …?". Without an answer ("Start all", or no dialog could open) nothing is said, and
 *  reIDM tries the link against the waiting download as it always has. */
function refreshFields(choice, armed) {
  if (!armed || !choice.refresh) return {};
  return choice.refresh === 'yes' ? { refresh_id: armed.id } : { no_refresh: true };
}

async function sendBatch(items, referer, later, storeId) {
  const withCookies = await Promise.all(
    items.map(async (it) => ({ ...it, referer: it.referer || referer, cookie: await cookieHeader(it.url, storeId) })),
  );
  const body = { items: withCookies, user_agent: navigator.userAgent, later: !!later };
  let r = await post('/ext/add-batch', body, BATCH_TIMEOUT_MS);
  if (!r.ok && r.unreachable && (await launchReidm())) r = await post('/ext/add-batch', body, BATCH_TIMEOUT_MS);
  return r;
}

// ── cookies, from the right profile ─────────────────────────────────────

/**
 * The cookie store a tab uses: Chrome's are "0" (the profile) and "1" (incognito), but the ids
 * are the browser's business, so they are looked up rather than assumed. A download has no
 * tab; for an incognito one, the store of any incognito tab is the incognito store.
 * Resolves to a store id, `undefined` for the default store, or `null` when an incognito store
 * should be used but cannot be found — then no cookies are sent, rather than the profile's.
 */
async function cookieStoreFor(tabId, incognito) {
  try {
    const stores = await chrome.cookies.getAllCookieStores();
    if (tabId != null && tabId >= 0) {
      const s = stores.find((x) => x.tabIds.includes(tabId));
      if (s) return s.id;
    }
    if (!incognito) return undefined;
    const tabs = await chrome.tabs.query({});
    const inc = new Set(tabs.filter((t) => t.incognito).map((t) => t.id));
    const s = stores.find((x) => x.tabIds.some((id) => inc.has(id)));
    if (s) return s.id;
  } catch {
    /* no cookie access */
  }
  return incognito ? null : undefined;
}

/** The `Cookie` header the browser itself would send for `url`, so reIDM can fetch what the
 *  signed-in user can. Read only at the moment of a hand-off, for that one URL. */
async function cookieHeader(url, storeId) {
  if (!chrome.cookies || !isHttp(url) || storeId === null) return undefined;
  try {
    const list = await chrome.cookies.getAll(storeId ? { url, storeId } : { url });
    if (!list.length) return undefined;
    // Longer paths first, as browsers send them.
    list.sort((a, b) => b.path.length - a.path.length);
    return list.map((c) => `${c.name}=${c.value}`).join('; ');
  } catch (e) {
    console.debug('reIDM: could not read cookies', e);
    return undefined;
  }
}

/** Where an action from the popup, the grabber or a menu came from: the tab, whether it is
 *  incognito, and its cookie store. */
async function tabContext(tabOrId) {
  let tab = tabOrId && typeof tabOrId === 'object' ? tabOrId : null;
  if (!tab && Number.isInteger(tabOrId) && tabOrId >= 0) tab = await chrome.tabs.get(tabOrId).catch(() => null);
  const incognito = !!(tab && tab.incognito);
  return { tab, incognito, url: tab ? tab.url : undefined, storeId: await cookieStoreFor(tab ? tab.id : null, incognito) };
}

const INCOGNITO_OFF = 'reIDM is off in incognito windows. Turn on "Also in incognito windows" in the extension\'s options to use it here.';

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
      if (recentPosts.size > 200) prune(recentPosts, POST_MEMORY_MS);
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

/** Held downloads, one key each: `{url, paused, stage, instance, at}`. One key per download
 *  means two held at once can't overwrite each other's record. `storage.local` rather than
 *  `session`, so that a reload of the extension (which clears `session`) still finds them. */
const PENDING_PREFIX = 'reidm-pending:';

async function pendingSet(id, entry) {
  try {
    await chrome.storage.local.set({ [PENDING_PREFIX + id]: { ...entry, instance: INSTANCE, at: Date.now() } });
  } catch (e) {
    console.warn('reIDM: could not note a held download', e);
  }
}

async function pendingDelete(id) {
  await chrome.storage.local.remove(PENDING_PREFIX + id).catch(() => {});
}

/** Why a download should stay with the browser, or null to take it. */
function keepReason(item) {
  if (!settings.capture) return 'capture is off';
  if (item.byExtensionId) return 'started by an extension'; // including our own fall-backs
  if (item.state && item.state !== 'in_progress') return 'not a new download';
  if (item.danger && !['safe', 'accepted'].includes(item.danger)) return 'the browser flagged it as dangerous; its own warning applies';
  const url = item.finalUrl || item.url;
  if (!isHttp(item.url) || !isHttp(url)) return 'made by the page itself (blob/data), nothing to hand over';
  if (isIgnored(item.url) || isIgnored(url)) return 'ignored address';
  if (item.incognito && !settings.captureIncognito) return 'incognito';
  if (hostSkipped(item.url) || hostSkipped(url)) return 'site is on the never-capture list';
  if (bypassed(item.url, item.finalUrl)) return 'the user asked the browser to keep it';
  const posted = recentPosts.get(item.url) || recentPosts.get(item.finalUrl);
  if (posted && Date.now() - posted < POST_MEMORY_MS) return 'a form submission (POST) cannot be repeated by reIDM';
  const kb = Number(settings.minSizeKB) || 0;
  if (kb > 0 && item.totalBytes > 0 && item.totalBytes < kb * 1024) return 'smaller than the minimum size';
  if (!settings.captureAll && !typeListed(item.filename) && !typeListed(url)) return 'file type not on the list';
  return null;
}

const callback = (fn) => new Promise((resolve) => fn(() => resolve(!chrome.runtime.lastError)));
const cancelAndErase = async (id) => {
  await callback((done) => chrome.downloads.cancel(id, done));
  await callback((done) => chrome.downloads.erase({ id }, done));
};

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
  let paused = false;
  try {
    await settingsReady;
    const why = keepReason(item);
    if (why) {
      console.debug(`reIDM: leaving download ${item.id} to the browser: ${why}`);
      return release();
    }

    // The reversible half first: stop the transfer while reIDM is asked.
    paused = await callback((done) => chrome.downloads.pause(item.id, done));
    await pendingSet(item.id, { url: item.url, paused, stage: 'asking' });

    const url = isHttp(item.finalUrl) ? item.finalUrl : item.url;
    const store = await cookieStoreFor(null, item.incognito);
    const details = {
      url,
      filename: baseName(item.filename) || nameFromUrl(url),
      referer: item.referrer || undefined,
      cookie: await cookieHeader(url, store),
      user_agent: navigator.userAgent,
      verify: true,
    };
    const armed = await refreshArmed(url);
    // The browser's copy stays paused, and its "Save as" unopened, while the user decides.
    const choice = await maybeAsk({
      url,
      from: item.referrer,
      filename: details.filename,
      size: item.totalBytes > 0 ? item.totalBytes : null,
      sizeNote: 'known once it starts',
      probe: false, // the browser is already fetching it; a probe would be one request more
      fallback: 'browser',
      program: isProgram(details.filename),
      refresh: armed,
    });
    if (choice.action === 'cancel') {
      // IDM's Cancel: nobody downloads it, neither reIDM nor the browser.
      await cancelAndErase(item.id);
      release();
      await pendingDelete(item.id);
      recordCapture({ url, filename: details.filename, outcome: 'cancelled' });
      return;
    }
    if (choice.action === 'release') {
      // Nobody answered: the browser downloads it, as if reIDM weren't there.
      if (paused) await callback((done) => chrome.downloads.resume(item.id, done));
      release();
      await pendingDelete(item.id);
      recordCapture({ url, filename: details.filename, error: choice.reason || 'no answer' });
      return;
    }
    details.filename = choice.filename || details.filename;
    if (choice.action === 'later') details.later = true;
    Object.assign(details, refreshFields(choice, armed));
    let r = await send(details);
    // A redirect's target can be bound to the request that produced it; the original address
    // is worth one more try before giving up.
    if (!r.ok && !r.unreachable && item.url !== url && isHttp(item.url)) {
      r = await send({ ...details, url: item.url, cookie: await cookieHeader(item.url, store) });
    }
    noteOffer(r, armed, choice);
    choice.report(r);
    if (r.ok) {
      // Noted before cancelling, so a worker that dies in between leaves a record saying the
      // browser's copy is redundant rather than one saying it should be resumed.
      await pendingSet(item.id, { url: item.url, paused, stage: 'taken' });
      // reIDM has it, and has already fetched its first byte: the browser's copy is redundant.
      await cancelAndErase(item.id);
      release();
      await pendingDelete(item.id);
      recordCapture({ url, filename: listedName(r, details.filename), outcome: r.outcome, id: r.id });
      flashBadge(r.outcome === 'already_complete' ? '=' : '✓', '#2ea053', 2500);
      announce(r, details.filename || nameFromUrl(url));
      return;
    }
    if (paused) await callback((done) => chrome.downloads.resume(item.id, done));
    release();
    await pendingDelete(item.id);
    recordCapture({ url, filename: details.filename, error: r.error || 'refused' });
    notify('the browser kept the download', r.error || 'reIDM refused it');
  } catch (e) {
    console.warn('reIDM: hand-off failed; the browser keeps the download', e);
    if (paused) chrome.downloads.resume(item.id, () => void chrome.runtime.lastError);
    release();
    pendingDelete(item.id);
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

/**
 * Downloads another worker was holding when it stopped. Chrome releases a dead worker's hold
 * on the file name but leaves the download paused, so it would wait forever. Whatever its age,
 * a record from another worker is an orphan: resumed for the browser, or — if reIDM had
 * already taken it — cancelled. The URL check guards against a download id reused after a
 * browser restart.
 */
async function reconcilePending() {
  let all = {};
  try {
    all = await chrome.storage.local.get(null);
  } catch {
    return;
  }
  for (const [key, e] of Object.entries(all)) {
    if (!key.startsWith(PENDING_PREFIX) || !e || e.instance === INSTANCE) continue;
    const id = Number(key.slice(PENDING_PREFIX.length));
    try {
      const [d] = await chrome.downloads.search({ id });
      if (d && d.url === e.url && d.state === 'in_progress') {
        if (e.stage === 'taken') await cancelAndErase(id);
        else if (d.paused) await callback((done) => chrome.downloads.resume(id, done));
        console.info(`reIDM: download ${id} was left by a stopped worker; ${e.stage === 'taken' ? 'reIDM has it' : 'the browser has it now'}`);
      }
    } catch (err) {
      console.warn('reIDM: could not settle a left-over download', err);
    }
    await chrome.storage.local.remove(key).catch(() => {});
  }
}

/** Recent hand-offs, for the popup's "what just happened". */
const CAPTURES_KEY = 'reidm-recent-captures';
async function recordCapture(entry) {
  try {
    const list = (await chrome.storage.session.get(CAPTURES_KEY))[CAPTURES_KEY] || [];
    list.unshift({ ...entry, at: Date.now() });
    await chrome.storage.session.set({ [CAPTURES_KEY]: list.slice(0, 20) });
  } catch (e) {
    console.debug('reIDM: could not record a hand-off', e);
  }
}

/** Record what came of a hand-off that has no browser copy behind it. */
function recordResult(url, filename, r) {
  if (r.cancelled) recordCapture({ url, filename, outcome: 'cancelled' });
  else if (r.ok) recordCapture({ url, filename, outcome: r.outcome, id: r.id });
  else recordCapture({ url, filename, error: r.error || 'refused', nothing: true });
}

function notify(title, message) {
  console.warn(`reIDM: ${title}: ${message}`);
  flashBadge('!', '#c02828', 4000);
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
  const offer = r.offered;
  const target = (offer && offer.name) || 'The download waiting for a new address';
  if (r.outcome === 'already_complete') toast('Already downloaded', `${what} is already in reIDM, finished.`, `reidm-${r.id}`);
  else if (r.outcome === 'existing') toast('reIDM carries on', `${what} was already in reIDM; it continues from where it stopped.`, `reidm-${r.id}`);
  else if (r.outcome === 'refreshed') toast('Address refreshed', `${target} continues from this link.`, `reidm-${r.id}`);
  else if (offer && offer.answer === 'yes') {
    // Asked to refresh, but reIDM found this link is another file and added it instead (a
    // reIDM that doesn't know `refresh_id` does that rather than refuse).
    toast('Added as a new download', `This link is not the same file as ${offer.name || 'the waiting download'}, so reIDM added it as ${what}. The waiting download still needs a new address.`, `reidm-${r.id}`);
  } else toast('Downloading in reIDM', what, `reidm-${r.id}`);
}

/** Note on a hand-off's result what the user was asked about an armed refresh, for `announce`
 *  and *Just now*. */
function noteOffer(r, armed, choice) {
  if (armed && r) r.offered = { ...armed, answer: choice.refresh || null };
  return r;
}

/** The name a hand-off is listed under in *Just now*: the waiting download's, if it took the link. */
const listedName = (r, filename) => (r && r.outcome === 'refreshed' && r.offered && r.offered.name) || filename;

// ── the toolbar badge ───────────────────────────────────────────────────
//
// The media count belongs to a tab: Chrome shows a tab's own badge while that tab is selected.
// The ✓, = and ! flashes go on the tab the user is looking at, and the count comes back after.

const BADGE_BLUE = '#0a60c8';

function showCount(tabId) {
  if (tabId == null || tabId < 0) return;
  const n = (mediaByTab.get(tabId) || new Map()).size;
  chrome.action.setBadgeBackgroundColor({ tabId, color: BADGE_BLUE }).catch(() => {});
  chrome.action.setBadgeText({ tabId, text: n ? String(n) : '' }).catch(() => {});
}

async function flashBadge(text, color, ms) {
  const [t] = await chrome.tabs.query({ active: true, lastFocusedWindow: true }).catch(() => []);
  const tabId = t ? t.id : undefined;
  const where = tabId != null ? { tabId } : {};
  chrome.action.setBadgeBackgroundColor({ ...where, color }).catch(() => {});
  chrome.action.setBadgeText({ ...where, text }).catch(() => {});
  setTimeout(() => (tabId != null ? showCount(tabId) : chrome.action.setBadgeText({ text: '' }).catch(() => {})), ms);
}

// ── the "Download file info" dialog ─────────────────────────────────────
//
// One window at a time. Requests queue up in it ("2 more waiting"), so a page that starts
// five downloads gets one dialog, not five windows stacked on top of each other that each take
// the focus. The window talks to this worker over a port named `confirm`.
//
// Every request ends in exactly one answer:
//   start / later — hand it to reIDM; `report(result)` shows the outcome in the dialog;
//   cancel        — the user pressed Cancel, Escape, or closed the window while it showed;
//   release       — nobody answered (the window never showed, or the request waited too long):
//                   the caller lets the browser have the download, as if reIDM weren't there.

const dialog = { windowId: null, creating: false, port: null, current: null, queue: [], seq: 0 };

const FALLBACK_TEXT = {
  browser: 'The browser downloads it instead.',
  none: 'Nothing was downloaded.',
};

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

/** `filename*=UTF-8''a%20b.zip` or `filename="a b.zip"` from a Content-Disposition header. */
function dispositionName(cd) {
  const star = /filename\*\s*=\s*"?(?:[\w!#$%&+^`{}~-]+'[^']*')?([^";]+)"?/i.exec(cd || '');
  if (star) {
    try {
      return decodeURIComponent(star[1].trim());
    } catch {
      /* fall through to the plain parameter */
    }
  }
  const plain = /filename\s*=\s*"([^"]*)"|filename\s*=\s*([^;]+)/i.exec(cd || '');
  return plain ? (plain[1] ?? plain[2]).trim() || null : null;
}

/** The size and the server's file name for a link nobody has requested yet. A HEAD request,
 *  which a one-time link does not count as its one download, with the browser's cookies as the
 *  download would carry. Never for incognito: that would use the normal profile's cookies. */
async function probeSize(url) {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), 8000);
  try {
    const r = await fetch(url, { method: 'HEAD', credentials: 'include', cache: 'no-store', signal: ctl.signal });
    const size = r.ok ? Number(r.headers.get('content-length')) || null : null;
    return { size, filename: r.ok ? dispositionName(r.headers.get('content-disposition')) : null };
  } catch {
    return { size: null, filename: null };
  } finally {
    clearTimeout(t);
  }
}

function dialogPost(m) {
  try {
    if (dialog.port) dialog.port.postMessage(m);
  } catch {
    /* the window is closing; `dialogGone` deals with it */
  }
}

function settle(q, choice) {
  if (q.answered) return;
  q.answered = true;
  clearTimeout(q.timer);
  q.resolve(choice);
}

/** Show the next request, or close the window when none is left. */
function advance() {
  dialog.current = null;
  const next = dialog.queue.shift();
  if (!next) {
    // Forget the window before it is gone, so a request arriving meanwhile opens a new one
    // instead of being shown in this one as it closes (and counted as cancelled).
    const id = dialog.windowId;
    Object.assign(dialog, { windowId: null, port: null });
    if (id != null) chrome.windows.remove(id).catch(() => {});
    return;
  }
  dialog.current = next;
  const r = next.req;
  const probing = !(r.size > 0) && r.probe !== false && !r.incognito && isHttp(r.url);
  dialogPost({
    type: 'info',
    id: next.id,
    url: r.url,
    from: hostOf(r.from) || null,
    filename: r.filename,
    size: r.size,
    sizeNote: r.sizeNote,
    dir: folderFor(r.filename),
    waiting: dialog.queue.length,
    fallback: FALLBACK_TEXT[r.fallback] || FALLBACK_TEXT.none,
    program: !!r.program,
    probing,
    // "Use this link to refresh …?" first; `ask` says whether *New download* then shows the
    // usual form or starts at once, as it would with the dialog off.
    refresh: r.refresh ? { name: r.refresh.name || `download #${r.refresh.id}` } : null,
    ask: !!settings.askBeforeDownload || !!r.program,
  });
  if (probing) {
    probeSize(r.url).then((p) => {
      if (dialog.current !== next) return;
      dialogPost({ type: 'size', id: next.id, size: p.size, note: 'checked when it starts', filename: r.nameFromServer ? p.filename : null });
    });
  }
}

function openDialog() {
  if (dialog.windowId != null || dialog.creating) return;
  dialog.creating = true;
  const width = 560;
  const height = 400;
  (async () => {
    const opts = { url: chrome.runtime.getURL('confirm.html'), type: 'popup', width, height, focused: true };
    const w = await chrome.windows.getLastFocused().catch(() => null);
    if (w && w.width && w.left != null) {
      opts.left = Math.max(0, Math.round(w.left + (w.width - width) / 2));
      opts.top = Math.max(0, Math.round(w.top + (w.height - height) / 3));
    }
    try {
      const win = await chrome.windows.create(opts);
      dialog.windowId = win.id;
    } catch (e) {
      // No window can be opened (a kiosk, a policy): don't hold the downloads hostage to it.
      console.warn('reIDM: could not open the download dialog', e);
      for (const q of dialog.queue.splice(0)) settle(q, { action: 'start', filename: q.req.filename, report: () => {} });
    } finally {
      dialog.creating = false;
    }
  })();
}

/** The window closed, or its page went away. */
function dialogGone() {
  const shown = !!dialog.port;
  const cur = dialog.current;
  Object.assign(dialog, { windowId: null, port: null, current: null });
  // Closing the window while it shows a download is Cancel, as in IDM.
  if (cur && !cur.answered) settle(cur, { action: 'cancel', filename: cur.req.filename, report: () => {} });
  if (!dialog.queue.length) return;
  if (shown) {
    // The others are still waiting for their answer: show them again.
    openDialog();
  } else {
    // The window closed before it ever showed anything. Nobody saw these, so nobody declined
    // them: the browser has them, as it would without reIDM.
    for (const q of dialog.queue.splice(0)) settle(q, { action: 'release', reason: 'the download dialog closed before it showed', report: () => {} });
  }
}

chrome.windows.onRemoved.addListener((windowId) => {
  if (windowId === dialog.windowId) dialogGone();
});

function onDialogConnect(port) {
  const windowId = port.sender && port.sender.tab ? port.sender.tab.windowId : null;
  if (dialog.port || (windowId !== dialog.windowId && !(dialog.creating && dialog.windowId == null))) {
    // A window left over from a worker that stopped: what it showed is no longer waiting.
    port.postMessage({ type: 'stale' });
    return;
  }
  if (dialog.windowId == null) dialog.windowId = windowId;
  dialog.port = port;
  port.onMessage.addListener((m) => {
    if (dialog.port !== port) return; // a window that is closing
    const cur = dialog.current;
    if (m.type === 'hello') {
      if (!cur) advance();
    } else if (m.type === 'answer' && cur && m.id === cur.id && !cur.answered) {
      if (m.noAsk && m.action !== 'cancel') {
        settings.askBeforeDownload = false;
        chrome.storage.sync.set({ askBeforeDownload: false }).catch(() => {});
      }
      if (m.action === 'cancel') {
        settle(cur, { action: 'cancel', filename: cur.req.filename, report: () => {} });
        advance();
        return;
      }
      const action = m.action === 'later' ? 'later' : 'start';
      // On a request that offered a refresh, anything but *Refresh* is a new download.
      const refresh = cur.req.refresh ? (m.action === 'refresh' ? 'yes' : 'no') : undefined;
      settle(cur, {
        action,
        refresh,
        filename: m.filename || cur.req.filename,
        report: (r) => {
          if (dialog.current !== cur) return;
          if (r && r.ok) advance();
          else {
            const why = (r && r.error) || 'unknown error';
            dialogPost({ type: 'error', id: cur.id, text: `reIDM couldn't take it: ${why}. ${FALLBACK_TEXT[cur.req.fallback] || FALLBACK_TEXT.none}`, waiting: dialog.queue.length });
          }
        },
      });
    } else if (m.type === 'answer-all' && cur && !cur.answered) {
      // "Start all": every waiting download with the name it came with.
      for (const q of [cur, ...dialog.queue.splice(0)]) settle(q, { action: 'start', filename: q === cur ? m.filename || q.req.filename : q.req.filename, report: () => {} });
      advance();
    } else if (m.type === 'ack' && cur && cur.answered) {
      advance();
    }
  });
  port.onDisconnect.addListener(() => {
    if (dialog.port === port) dialogGone();
  });
}

chrome.runtime.onConnect.addListener((port) => {
  if (port.name === 'confirm') onDialogConnect(port);
});

/** Queue a request in the dialog. Resolves to `{action, filename, reason?, report(result)}`. */
function askUser(req) {
  return new Promise((resolve) => {
    const q = { id: ++dialog.seq, req, resolve, answered: false, timer: null };
    q.timer = setTimeout(() => {
      // The user is away. A held download goes back to the browser (whose server may drop a
      // connection paused for this long); anything else is simply not downloaded.
      const i = dialog.queue.indexOf(q);
      if (i >= 0) dialog.queue.splice(i, 1);
      settle(q, { action: 'release', reason: 'nobody answered the download dialog', report: () => {} });
      if (dialog.current === q) dialogPost({ type: 'error', id: q.id, text: `Nobody answered for ${DIALOG_TIMEOUT_MS / 60000} minutes. ${FALLBACK_TEXT[req.fallback] || FALLBACK_TEXT.none}`, waiting: dialog.queue.length });
      else if (dialog.current) dialogPost({ type: 'waiting', waiting: dialog.queue.length });
    }, DIALOG_TIMEOUT_MS);
    dialog.queue.push(q);
    if (dialog.port && !dialog.current) advance();
    else if (dialog.port) dialogPost({ type: 'waiting', waiting: dialog.queue.length });
    else openDialog();
  });
}

/**
 * Ask when the user wants to be asked — and always for a program a page started by itself, and
 * while reIDM waits for a new address (`refresh`): the user armed that a moment ago in reIDM,
 * and whether this link is the one they meant is theirs to say, not a guess.
 * `req`: { url, filename, from?, size?, sizeNote?, probe?: false, incognito?, nameFromServer?,
 *          fallback: 'browser' | 'none', program?, refresh?: {id, name} }.
 * Resolves to `{action, filename, refresh?: 'yes' | 'no', reason?, report(result)}`.
 */
async function maybeAsk(req) {
  await settingsReady;
  if (!settings.askBeforeDownload && !req.program && !req.refresh) return { action: 'start', filename: req.filename, report: () => {} };
  return askUser(req);
}

/** Run a hand-off after the dialog said yes; whatever happens, the dialog hears the outcome. */
async function deliver(choice, fn) {
  let r;
  try {
    r = await fn();
  } catch (e) {
    console.warn('reIDM: hand-off failed', e);
    r = { ok: false, error: errText(e) };
  }
  choice.report(r);
  return r;
}

// ── 2. links the content script took on click ───────────────────────────

/** A link the user clicked, which the content script stopped before the browser requested
 *  it. reIDM checks it first; `refuse_html` makes it decline a "file" link that actually
 *  answers with a web page (a login wall, a download page), which the browser should then open.
 *  Anything but `ok` makes the content script give the click back to the browser. */
async function captureLink(msg, sender) {
  await settingsReady;
  const url = msg.url;
  if (/^magnet:/i.test(url || '')) return captureMagnet(msg, sender);
  if (!settings.capture || !isHttp(url) || isIgnored(url) || hostSkipped(url)) return { ok: false, error: 'not captured' };
  const incognito = !!(sender.tab && sender.tab.incognito);
  if (incognito && !settings.captureIncognito) return { ok: false, error: 'incognito' };
  let filename = msg.filename || nameFromUrl(url) || undefined;
  const referer = (sender.tab && sender.tab.url) || undefined;
  const armed = await refreshArmed(url);
  const choice = await maybeAsk({ url, from: referer, filename, incognito, nameFromServer: !msg.filename, fallback: 'browser', program: isProgram(filename), refresh: armed });
  // Cancelled: the click is used up and nothing downloads. `ok` so the page doesn't hand the
  // click back to the browser.
  if (choice.action === 'cancel') {
    recordCapture({ url, filename, outcome: 'cancelled' });
    return { ok: true, cancelled: true };
  }
  if (choice.action === 'release') return { ok: false, error: choice.reason };
  filename = choice.filename || filename;
  const store = await cookieStoreFor(sender.tab ? sender.tab.id : null, incognito);
  const r = await deliver(choice, async () => send({
    url,
    filename,
    referer,
    cookie: await cookieHeader(url, store),
    user_agent: navigator.userAgent,
    verify: true,
    refuse_html: true,
    later: choice.action === 'later',
    ...refreshFields(choice, armed),
  }).then((res) => noteOffer(res, armed, choice)));
  if (r.ok) {
    recordCapture({ url, filename: listedName(r, filename), outcome: r.outcome, id: r.id });
    flashBadge(r.outcome === 'already_complete' ? '=' : '✓', '#2ea053', 2500);
    announce(r, filename || nameFromUrl(url));
  } else {
    recordCapture({ url, filename, error: r.error || 'refused' });
  }
  return r;
}

/** A magnet link the user clicked: a torrent for reIDM. Its name is the link's `dn`; its size
 *  is known only once peers describe it. */
async function captureMagnet(msg, sender) {
  if (!settings.capture) return { ok: false, error: 'capture is off' };
  const incognito = !!(sender.tab && sender.tab.incognito);
  if (incognito && !settings.captureIncognito) return { ok: false, error: 'incognito' };
  let dn = null;
  try {
    dn = new URLSearchParams(msg.url.slice(msg.url.indexOf('?') + 1)).get('dn');
  } catch {
    /* no name, then */
  }
  const name = dn || 'torrent';
  const choice = await maybeAsk({ url: msg.url, filename: name, sizeNote: 'known once peers describe the torrent', probe: false, fallback: 'browser' });
  if (choice.action === 'cancel') {
    recordCapture({ url: msg.url, filename: name, outcome: 'cancelled' });
    return { ok: true, cancelled: true };
  }
  if (choice.action === 'release') return { ok: false, error: choice.reason };
  // The name is the torrent's own; an edited one is not sent, as reIDM names it from the torrent.
  const r = await deliver(choice, () => send({ url: msg.url, referer: (sender.tab && sender.tab.url) || undefined, later: choice.action === 'later' }));
  if (r.ok) {
    recordCapture({ url: msg.url, filename: name, outcome: r.outcome, id: r.id });
    flashBadge('✓', '#2ea053', 2500);
    announce(r, dn || 'the torrent');
  } else {
    recordCapture({ url: msg.url, filename: name, error: r.error || 'refused' });
  }
  return r;
}

/** A link the user handed over explicitly — a menu, the popup — so there is no browser copy:
 *  a refusal means nothing was downloaded, and says so. */
async function addLink(url, ctx, { referer, filename } = {}) {
  await settingsReady;
  if (!isHttp(url)) return { ok: false, error: 'that is not an http or https link' };
  if (ctx.incognito && !settings.captureIncognito) return { ok: false, error: INCOGNITO_OFF };
  const proposed = filename || nameFromUrl(url) || undefined;
  const armed = await refreshArmed(url);
  const choice = await maybeAsk({ url, from: referer, filename: proposed, incognito: ctx.incognito, nameFromServer: !filename, fallback: 'none', refresh: armed });
  if (choice.action === 'cancel' || choice.action === 'release') {
    const r = { ok: false, cancelled: true, error: 'cancelled' };
    recordResult(url, proposed, r);
    return r;
  }
  const name = choice.filename || proposed;
  const r = await deliver(choice, async () => send({
    url,
    referer,
    filename: name,
    cookie: await cookieHeader(url, ctx.storeId),
    user_agent: navigator.userAgent,
    later: choice.action === 'later',
    ...refreshFields(choice, armed),
  }).then((res) => noteOffer(res, armed, choice)));
  recordResult(url, listedName(r, name), r);
  return r;
}

// ── 3. media sniffing ───────────────────────────────────────────────────

const MEDIA_EXTS = ['m3u8', 'mpd', 'mp4', 'm4v', 'webm', 'mkv', 'mov', 'flv', 'mp3', 'm4a', 'flac', 'ogg', 'opus', 'wav', 'aac'];
const MEDIA_EXT = new RegExp(`\\.(${MEDIA_EXTS.join('|')})(\\?|#|$)`, 'i');
const MANIFEST_EXT = /\.(m3u8|mpd)(\?|#|$)/i;
/** Pieces of a stream, which are useless on their own and would bury the real entries. */
const SEGMENT = /\.(ts|m4s|cmfv|cmfa)(\?|#|$)|[/_-](seg|segment|chunk|frag|fragment)[-_]?\d+|\/range\/\d+-\d+/i;
const MANIFEST_MIME = /mpegurl|dash\+xml/i;
const MEDIA_MIME = /^(video|audio)\//i;
/** YouTube's own player traffic: its formats come through the quality picker instead. */
const SKIP_MEDIA_HOST = /(^|\.)googlevideo\.com$/i;

/** Counters for DevTools and the browser test: how often the network listeners run. */
const stats = { sniffed: 0, manifestFetches: 0, statusWrites: 0 };

/** tabId -> Map(url -> {url, kind, at, mime?, size?, formats?}) */
const mediaByTab = new Map();
/** tabId -> Set of manifest addresses (without their query) already re-fetched. */
const manifestsFetched = new Map();

// What a page may report, and how much of it. A page — any script on it, an advert included —
// can post messages to the content script, so nothing it says is stored unchecked.
const MAX_URL = 4096;
const MAX_ITEMS_PER_MESSAGE = 100;
const MAX_FORMATS = 64;
const MAX_PER_TAB = 200;
const REPORTED_KINDS = new Set(['media', 'video', 'audio', 'stream', 'link', 'source']);
const okUrl = (u) => typeof u === 'string' && u.length <= MAX_URL && isHttp(u);
const short = (s, n = 200) => (typeof s === 'string' ? s.slice(0, n) : null);
const num = (n) => (Number.isFinite(Number(n)) && n !== null && n !== '' ? Number(n) : null);

function cleanFormat(f) {
  if (!f || typeof f !== 'object') return null;
  const mime = /^(video|audio|application)\/[\w.+-]{1,60}$/i.test(f.mime || '') ? f.mime : null;
  return {
    itag: Number.isInteger(f.itag) ? f.itag : null,
    quality: short(f.quality, 40),
    width: num(f.width),
    height: num(f.height),
    bitrate: num(f.bitrate),
    fps: num(f.fps),
    size: num(f.size),
    mime,
    codecs: short(f.codecs, 100),
    videoCodec: short(f.videoCodec, 20),
    audioCodec: short(f.audioCodec, 20),
    audioOnly: !!f.audioOnly,
    hasAudio: !!f.hasAudio,
    drc: !!f.drc,
    kind: short(f.kind, 40),
    url: okUrl(f.url) ? f.url : null,
  };
}
const cleanFormats = (list) => (Array.isArray(list) ? list.slice(0, MAX_FORMATS).map(cleanFormat).filter(Boolean) : undefined);

// An MV3 service worker is stopped whenever Chrome likes, and module-scope state goes with it.
// Each tab's media is mirrored to session storage under its own key, which lives exactly as
// long as "a page that is still open" can, and only the tabs that changed are written.
const MEDIA_PREFIX = 'reidm-media:';
const dirtyTabs = new Set();
let persistTimer = null;

function persistMedia(tabId) {
  dirtyTabs.add(tabId);
  if (persistTimer) return;
  persistTimer = setTimeout(async () => {
    persistTimer = null;
    // Never write before the stored lists are merged in, or a worker just woken by a request
    // would replace a tab's full list with the one entry it has seen.
    await mediaReady;
    const set = {};
    const gone = [];
    for (const t of dirtyTabs) {
      const map = mediaByTab.get(t);
      if (map && map.size) set[MEDIA_PREFIX + t] = [...map.values()];
      else gone.push(MEDIA_PREFIX + t);
    }
    dirtyTabs.clear();
    try {
      if (Object.keys(set).length) await chrome.storage.session.set(set);
      if (gone.length) await chrome.storage.session.remove(gone);
    } catch (e) {
      console.warn('reIDM: could not store the media list', e);
    }
  }, 250);
}

/** Merge what an earlier worker stored with whatever this one has seen since it woke. */
async function restoreMedia() {
  try {
    const all = await chrome.storage.session.get(null);
    for (const [key, list] of Object.entries(all)) {
      if (!key.startsWith(MEDIA_PREFIX) || !Array.isArray(list)) continue;
      const tabId = Number(key.slice(MEDIA_PREFIX.length));
      if (!mediaByTab.has(tabId)) mediaByTab.set(tabId, new Map());
      const map = mediaByTab.get(tabId);
      for (const e of list) {
        const mine = map.get(e.url);
        if (!mine) map.set(e.url, e);
        else for (const [k, v] of Object.entries(e)) if (mine[k] == null && v != null) mine[k] = v;
      }
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
// The MAIN-world hook also parses manifests, but it can miss one — a player in a worker, a
// frame the hook can't see. `webRequest` sees the request regardless, so the worker re-fetches
// a manifest itself (it has host permissions) — once per address and tab, and never one it
// fetched itself: its own requests are seen by its own listeners too (tab −1), and re-fetching
// those fed a loop of thousands of requests a second on a live stream.

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
  stats.manifestFetches++;
  try {
    const r = await fetch(url, { credentials: 'include', cache: 'no-store' });
    if (!r.ok || Number(r.headers.get('content-length')) > 2 * 1024 * 1024) return [];
    const body = (await r.text()).slice(0, 2 * 1024 * 1024);
    if (body.includes('#EXTM3U')) return parseHLS(body, url);
    if (body.includes('<MPD') || body.includes('<Representation')) return parseDASH(body, url);
  } catch {
    /* a signed, POST-only or otherwise unfetchable URL; the page hook may still have it */
  }
  return [];
}

function record(tabId, url, kind, extra = {}) {
  if (tabId === undefined || tabId < 0 || !okUrl(url) || isIgnored(url)) return;
  if (!mediaByTab.has(tabId)) mediaByTab.set(tabId, new Map());
  const tab = mediaByTab.get(tabId);
  const prev = tab.get(url);
  if (prev) {
    // A later sighting may know more (a size, a MIME type, the variants); keep the first
    // sighting's time.
    let changed = false;
    for (const [k, v] of Object.entries(extra)) {
      if (v == null) continue;
      if (k === 'formats' ? v.length && JSON.stringify(v) !== JSON.stringify(prev.formats) : prev[k] == null) {
        prev[k] = v;
        changed = true;
      }
    }
    if (changed) persistMedia(tabId);
    return;
  }
  const entry = { url, kind, at: Date.now() };
  for (const [k, v] of Object.entries(extra)) if (v != null) entry[k] = v;
  tab.set(url, entry);
  // Keep the per-tab list from growing without bound on a long-running player.
  if (tab.size > MAX_PER_TAB) {
    const oldest = [...tab.entries()].sort((a, b) => a[1].at - b[1].at)[0];
    tab.delete(oldest[0]);
  }
  showCount(tabId);
  persistMedia(tabId);
}

function manifestKey(url) {
  try {
    const u = new URL(url);
    return u.origin + u.pathname; // tokens in the query change on every poll of a live stream
  } catch {
    return url;
  }
}

function noteManifest(tabId, url, extra) {
  if (tabId === undefined || tabId < 0) return;
  record(tabId, url, 'stream', extra);
  const key = manifestKey(url);
  if (!manifestsFetched.has(tabId)) manifestsFetched.set(tabId, new Set());
  const done = manifestsFetched.get(tabId);
  const entry = (mediaByTab.get(tabId) || new Map()).get(url);
  if (done.has(key) || (entry && entry.formats && entry.formats.length)) return;
  done.add(key);
  // Fetch and parse it out-of-band; the picker can then offer real variants even if the page
  // fetched the manifest where the hook could not see it.
  formatsForManifest(url).then(async (formats) => {
    if (!formats.length) return;
    await mediaReady;
    record(tabId, url, 'stream', { formats: cleanFormats(formats) });
  });
}

function onMediaRequest(d) {
  stats.sniffed++;
  if (d.tabId < 0 || SKIP_MEDIA_HOST.test(hostOf(d.url))) return;
  if (MANIFEST_EXT.test(d.url)) noteManifest(d.tabId, d.url);
  else if (MEDIA_EXT.test(d.url) && !SEGMENT.test(d.url)) record(d.tabId, d.url, 'media');
}

// Many players fetch media from addresses with no recognisable extension. The response's own
// Content-Type says what it is, and Content-Length/Content-Range how big.
function onMediaHeaders(d) {
  stats.sniffed++;
  if (d.tabId < 0 || SKIP_MEDIA_HOST.test(hostOf(d.url))) return;
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
}

// The filters run in the browser process: requests that can't be media — images, scripts,
// fonts, stylesheets, pings — never reach this worker, so it can go idle on a busy page.
const MEDIA_REQUESTS = {
  urls: MEDIA_EXTS.map((e) => `*://*/*.${e}*`),
  types: ['main_frame', 'sub_frame', 'media', 'xmlhttprequest', 'object', 'other'],
};
const MEDIA_RESPONSES = { urls: ['http://*/*', 'https://*/*'], types: ['media', 'xmlhttprequest'] };

/** The network listeners exist only while media detection is on. They don't need to be there
 *  at start-up to wake the worker: nothing depends on them but the media lists. */
function applySniffing() {
  const on = !!settings.sniffMedia;
  const has = chrome.webRequest.onBeforeRequest.hasListener(onMediaRequest);
  if (on && !has) {
    chrome.webRequest.onBeforeRequest.addListener(onMediaRequest, MEDIA_REQUESTS);
    chrome.webRequest.onHeadersReceived.addListener(onMediaHeaders, MEDIA_RESPONSES, ['responseHeaders']);
  } else if (!on && has) {
    chrome.webRequest.onBeforeRequest.removeListener(onMediaRequest);
    chrome.webRequest.onHeadersReceived.removeListener(onMediaHeaders);
  }
  syncPageHook();
}

/**
 * The MAIN-world hook (inject.js) is a content script registered here, not a `<script>` the
 * content script adds: registered for `document_start` in the page's world it runs before the
 * page's own scripts, a page's CSP can't block it, and it needs no web-accessible resource
 * that any site could probe to see that reIDM is installed. It is registered only while media
 * detection is on, so with it off no page has its `fetch` wrapped.
 */
const HOOK_ID = 'reidm-hook';
let hookSync = Promise.resolve();
function syncPageHook() {
  hookSync = hookSync.then(async () => {
    try {
      const want = !!settings.sniffMedia;
      const have = (await chrome.scripting.getRegisteredContentScripts({ ids: [HOOK_ID] })).length > 0;
      if (want && !have) {
        await chrome.scripting.registerContentScripts([
          { id: HOOK_ID, js: ['inject.js'], matches: ['<all_urls>'], allFrames: true, runAt: 'document_start', world: 'MAIN', persistAcrossSessions: true },
        ]);
      } else if (!want && have) {
        await chrome.scripting.unregisterContentScripts({ ids: [HOOK_ID] });
      }
    } catch (e) {
      console.warn('reIDM: could not register the page hook', e);
    }
  });
  return hookSync;
}

// Registered synchronously, as every listener must be; removed again below if the setting is off.
chrome.webRequest.onBeforeRequest.addListener(onMediaRequest, MEDIA_REQUESTS);
chrome.webRequest.onHeadersReceived.addListener(onMediaHeaders, MEDIA_RESPONSES, ['responseHeaders']);
settingsReady.then(applySniffing);

function forgetTab(tabId) {
  mediaByTab.delete(tabId);
  manifestsFetched.delete(tabId);
  persistMedia(tabId);
}

chrome.tabs.onRemoved.addListener(forgetTab);
chrome.tabs.onUpdated.addListener((tabId, info) => {
  if (info.status === 'loading' && info.url) {
    // A new document in the tab: the previous page's media is no longer relevant.
    forgetTab(tabId);
    showCount(tabId);
  }
});

// ── 3b. handing a chosen video format to reIDM ──────────────────────────

/** File extensions a format's name may end in; anything else a page claims becomes .mp4. */
const MEDIA_NAME_EXT = new Set(['mp4', 'm4v', 'webm', 'mkv', 'mov', 'mp3', 'm4a', 'aac', 'ogg', 'oga', 'opus', 'flac', 'wav', 'flv', 'ts']);

/** A safe-ish file name: "Title 1080p.mp4". reIDM sanitises it again on its side. */
function formatFilename(title, format, url) {
  const ext = (() => {
    const m = /^(?:video|audio)\/(?:x-)?([\w]+)/i.exec(format.mime || '');
    if (m && MEDIA_NAME_EXT.has(m[1].toLowerCase())) return m[1].toLowerCase();
    if (m && /^mpeg$/i.test(m[1])) return format.mime.startsWith('audio') ? 'mp3' : 'mp4';
    const u = /\.([a-z0-9]{2,4})(?:\?|#|$)/i.exec(url || '');
    return u && MEDIA_NAME_EXT.has(u[1].toLowerCase()) ? u[1].toLowerCase() : 'mp4';
  })();
  const base = (title || 'video').replace(/[/\\?%*:|"<>]/g, '_').replace(/\s+/g, ' ').trim().slice(0, 120);
  const q = format.quality ? ` ${String(format.quality).replace(/[/\\?%*:|"<>]/g, '_')}` : '';
  const a = format.audioOnly ? ' (audio)' : '';
  return `${base}${q}${a}.${ext}`;
}

/**
 * The yt-dlp format selector for a format picked in the list: that exact format when yt-dlp
 * offers it (its list is not always the page's), else the best at the same height — H.264 with
 * AAC first, which plays anywhere. A format without sound gets the best audio in its own
 * container joined to it (WebM with Opus, MP4 with AAC), so the file stays the type the dialog
 * named rather than becoming an MKV. "Stable volume" audio is yt-dlp's `<itag>-drc`, else the
 * plain one. With nothing picked (`null` from the popup's "best quality" row too), the best
 * there is. reIDM names the file after yt-dlp's result, whatever the extension sent.
 */
function ytdlpFormat(f) {
  f = f || {};
  const id = Number.isInteger(f.itag) ? String(f.itag) : null;
  if (f.audioOnly) return [id && f.drc ? `${id}-drc` : null, id, 'bestaudio[ext=m4a]', 'bestaudio'].filter(Boolean).join('/');
  const h = Number(f.height) || parseInt(f.quality, 10) || 0;
  const cap = h ? `[height<=${h}]` : '';
  const audio = /webm/i.test(f.mime || '') ? 'bestaudio[ext=webm]' : 'bestaudio[ext=m4a]';
  const alts = [];
  if (id) alts.push(...(f.hasAudio ? [id] : [`${id}+${audio}`, `${id}+bestaudio`]));
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

/** A video that only yt-dlp can resolve — a YouTube page, or an HLS stream elsewhere: hand
 *  reIDM the address and a format, and yt-dlp fetches and joins the pieces. */
async function downloadVideoPage(page, title, f, ctx) {
  await settingsReady;
  if (ctx.incognito && !settings.captureIncognito) return { ok: false, error: INCOGNITO_OFF };
  const can = await reidmResolves();
  if (!can.ok) return can;
  const proposed = f && (f.itag != null || f.quality) ? formatFilename(title, f, page) : `${String(title || 'video').replace(/[/\\?%*:|"<>]/g, '_').slice(0, 120)}.mp4`;
  const choice = await maybeAsk({
    url: page,
    from: ctx.url,
    filename: proposed,
    size: f && f.size > 0 ? f.size : null,
    sizeNote: 'known once the download starts',
    probe: false,
    fallback: 'none',
  });
  if (choice.action === 'cancel' || choice.action === 'release') {
    const r = { ok: true, cancelled: true };
    recordResult(page, proposed, r);
    return r;
  }
  const name = choice.filename || proposed;
  const r = await deliver(choice, async () => send({
    url: page,
    filename: name,
    later: choice.action === 'later',
    referer: ctx.url || page,
    // The signed-in user's cookies let yt-dlp fetch what they can watch (age checks, members).
    cookie: await cookieHeader(page, ctx.storeId),
    user_agent: navigator.userAgent,
    media: { format: f && f.ytdlp ? f.ytdlp : ytdlpFormat(f) },
  }));
  recordResult(page, name, r);
  if (r.ok) {
    flashBadge('✓', '#2ea053', 2500);
    announce(r, name);
  } else notify('reIDM could not take the video', r.error || 'unknown error');
  return r;
}

const isYouTube = (u) => /^https?:\/\/([a-z0-9-]+\.)*(youtube\.com|youtube-nocookie\.com|youtu\.be)\//i.test(u || '');
const isHlsFormat = (f, url) => f.kind === 'hls-variant' || /mpegurl/i.test(f.mime || '') || /\.m3u8(\?|#|$)/i.test(url || '');

/**
 * The user picked a format in the picker. On YouTube it goes through reIDM's yt-dlp; an HLS
 * variant elsewhere too (on its own it is a playlist file, not a video); anything else with an
 * address is just a download. What the page told the picker is checked here: a script on the
 * page could have posted a "format" pointing at another site.
 */
async function downloadFormat(msg, sender) {
  const f = cleanFormat(msg.format) || {};
  const frameUrl = sender.url || (sender.tab && sender.tab.url) || '';
  const ctx = await tabContext(sender.tab);
  const title = short(msg.title, 200);
  if (msg.source === 'youtube') {
    // Only a YouTube frame may ask for a YouTube download, and only of a YouTube page.
    if (!isYouTube(frameUrl) || !isYouTube(msg.page)) return { ok: false, error: 'that page is not a YouTube video' };
    return downloadVideoPage(msg.page, title, f, ctx);
  }
  const url = okUrl(msg.url) ? msg.url : null;
  if (!url) return { ok: false, error: 'this format has no address to download from' };
  if (isHlsFormat(f, url)) {
    return downloadVideoPage(url, title, { ...f, ytdlp: 'best' }, ctx);
  }
  await settingsReady;
  if (ctx.incognito && !settings.captureIncognito) return { ok: false, error: INCOGNITO_OFF };
  const proposed = formatFilename(title, f, url);
  const armed = await refreshArmed(url);
  const choice = await maybeAsk({ url, from: frameUrl, filename: proposed, size: f.size > 0 ? f.size : null, incognito: ctx.incognito, fallback: 'none', refresh: armed });
  if (choice.action === 'cancel' || choice.action === 'release') {
    const r = { ok: true, cancelled: true };
    recordResult(url, proposed, r);
    return r;
  }
  const name = choice.filename || proposed;
  const r = await deliver(choice, async () => send({
    url,
    filename: name,
    referer: frameUrl || undefined,
    cookie: await cookieHeader(url, ctx.storeId),
    user_agent: navigator.userAgent,
    later: choice.action === 'later',
    ...refreshFields(choice, armed),
  }).then((res) => noteOffer(res, armed, choice)));
  recordResult(url, listedName(r, name), r);
  if (r.ok) {
    flashBadge('✓', '#2ea053', 2500);
    announce(r, name);
  } else notify('reIDM could not take the video', r.error || 'unknown error');
  return r;
}

/** The popup's "Download selected": handled here rather than in the popup, because the popup
 *  closes as soon as the dialog takes the focus, and its loop would die with it. All the
 *  dialogs queue up in the one window. */
async function queueMedia(items, tabId) {
  const ctx = await tabContext(tabId);
  const list = (Array.isArray(items) ? items : []).slice(0, MAX_ITEMS_PER_MESSAGE).filter((it) => it && okUrl(it.url));
  const results = await Promise.all(list.map((it) => (it.video
    ? downloadVideoPage(it.url, short(it.title), null, ctx)
    : addLink(it.url, ctx, { referer: ctx.url, filename: short(it.filename) || undefined }))));
  const done = results.filter((r) => r.ok && !r.cancelled).length;
  const failed = results.filter((r) => !r.ok && !r.cancelled);
  if (done) flashBadge('✓', '#2ea053', 2500);
  if (done && results.length > 1) toast('Downloading in reIDM', `${done} download${done === 1 ? '' : 's'} added`);
  else if (done === 1) announce(results.find((r) => r.ok && !r.cancelled), list.length === 1 ? nameFromUrl(list[0].url) : null);
  if (failed.length) notify('reIDM could not take it', failed[0].error || 'unknown error');
  return { ok: !failed.length, done, cancelled: results.filter((r) => r.cancelled).length, errors: failed.map((r) => r.error || 'failed'), results };
}

// ── 4. messages ─────────────────────────────────────────────────────────

/** What a content script — and so, in the worst case, a compromised page — may ask for.
 *  Everything else is for the extension's own pages: the popup, the grabber, the options. */
const FROM_PAGES = new Set(['media', 'formats-for-tab', 'capture-link', 'bypass', 'open-tab', 'download-format']);

/** Answer with the handler's result, or with its error: the sender always hears back. */
const answer = (reply, p) => {
  Promise.resolve(p)
    .then(reply, (e) => {
      console.warn('reIDM: message handler failed', e);
      reply({ ok: false, error: errText(e) });
    });
  return true;
};

chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  if (!msg || typeof msg !== 'object') return false;
  const fromExtension = sender.id === chrome.runtime.id && (sender.url || '').startsWith(chrome.runtime.getURL(''));
  if (!fromExtension && !FROM_PAGES.has(msg.type)) {
    reply({ ok: false, error: 'not allowed from a web page' });
    return false;
  }
  // A content script speaks for its own tab only.
  const tabId = fromExtension ? msg.tabId : sender.tab ? sender.tab.id : undefined;

  switch (msg.type) {
    case 'media': {
      for (const m of (Array.isArray(msg.items) ? msg.items : []).slice(0, MAX_ITEMS_PER_MESSAGE)) {
        if (!m || !REPORTED_KINDS.has(m.kind)) continue;
        record(tabId, m.url, m.kind, { formats: cleanFormats(m.formats) });
      }
      reply({ ok: true });
      return false;
    }
    case 'formats-for-tab':
      return answer(reply, mediaReady.then(() => ({
        ok: true,
        entries: [...(mediaByTab.get(tabId) || new Map()).values()].filter((i) => i.formats && i.formats.length),
      })));
    case 'media-for-tab':
      return answer(reply, mediaReady.then(() => ({ items: [...(mediaByTab.get(tabId) || new Map()).values()] })));
    case 'clear-tab':
      forgetTab(tabId);
      showCount(tabId);
      reply({ ok: true });
      return false;
    case 'capture-link':
      return answer(reply, captureLink(msg, sender));
    case 'bypass':
      // The content script is about to let the browser have this URL.
      if (okUrl(msg.url)) bypass.set(msg.url, Date.now() + BYPASS_MS);
      prune(bypass, 60000);
      reply({ ok: true });
      return false;
    case 'open-tab': {
      // A `target=_blank` link reIDM declined: open it as the click would have. The tab is the
      // worker's to open because the page's user activation may have lapsed by now.
      if (!okUrl(msg.url)) {
        reply({ ok: false });
        return false;
      }
      bypass.set(msg.url, Date.now() + BYPASS_MS);
      const t = sender.tab;
      chrome.tabs.create({ url: msg.url, windowId: t ? t.windowId : undefined, index: t ? t.index + 1 : undefined, openerTabId: t ? t.id : undefined }, () =>
        reply({ ok: !chrome.runtime.lastError }),
      );
      return true;
    }
    case 'queue-media':
      return answer(reply, queueMedia(msg.items, tabId));
    case 'add-batch':
      return answer(reply, (async () => {
        await settingsReady;
        const ctx = await tabContext(tabId);
        if (ctx.incognito && !settings.captureIncognito) return { ok: false, error: INCOGNITO_OFF };
        const items = (Array.isArray(msg.items) ? msg.items : []).filter((it) => it && okUrl(it.url)).map((it) => ({ url: it.url, filename: short(it.filename) || undefined }));
        const r = await sendBatch(items, ctx.url || msg.referer, msg.later, ctx.storeId);
        const n = ((r && r.results) || []).filter((x) => x.outcome === 'new').length;
        if (r && r.ok && n) toast(msg.later ? 'Added to reIDM for later' : 'Downloading in reIDM', `${n} download${n === 1 ? '' : 's'} added`);
        return r;
      })());
    case 'download-format':
      return answer(reply, downloadFormat(msg, sender));
    case 'download-video':
      // The video on a YouTube page, in the best quality.
      return answer(reply, tabContext(tabId).then((ctx) => downloadVideoPage(msg.page, short(msg.title), null, ctx)));
    case 'ping':
      return answer(reply, ping(msg.port).then((info) => ({ ok: !!info, info })));
    case 'launch':
      return answer(reply, launchReidm().then((ok) => ({ ok })));
    case 'recent-captures':
      return answer(reply, chrome.storage.session.get(CAPTURES_KEY).then((o) => ({ items: o[CAPTURES_KEY] || [] }), () => ({ items: [] })));
    case 'open-grabber':
      return answer(reply, openGrabber(tabId, msg.select).then(() => ({ ok: true })));
    case 'wake':
      // A dialog whose worker stopped: starting this one has already settled what it held.
      return answer(reply, reconciled.then(() => ({ ok: true })));
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
    chrome.contextMenus.create({ id: MENU.page, title: 'Download all links on this page with reIDM…', contexts: ['page'] });
    chrome.contextMenus.create({ id: MENU.grab, title: 'Grab resources from this page with reIDM…', contexts: ['page', 'frame'] });
    chrome.contextMenus.create({ id: MENU.selection, title: 'Download the selected link with reIDM', contexts: ['selection'] });
  });
  ping();
});

/** The grabber, for a tab. `select: 'files'` ticks the links to files, so "all links" is one
 *  look and one press rather than hundreds of pages queued blind. */
async function openGrabber(tabId, select) {
  if (!Number.isInteger(tabId)) return;
  const q = new URLSearchParams({ tab: String(tabId) });
  if (select === 'files') q.set('select', 'files');
  await chrome.tabs.create({ url: chrome.runtime.getURL(`grabber.html?${q}`) });
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const ctx = await tabContext(tab);
  const referer = ctx.url;

  if (info.menuItemId === MENU.link || info.menuItemId === MENU.media || info.menuItemId === MENU.selection) {
    const url = info.menuItemId === MENU.selection ? (info.selectionText || '').trim() : info.linkUrl || info.srcUrl;
    if (!url) return;
    const r = await addLink(url, ctx, { referer });
    if (r.cancelled) return;
    if (r.ok) {
      flashBadge(r.outcome === 'already_complete' ? '=' : '✓', '#2ea053', 2500);
      announce(r, nameFromUrl(url));
    } else notify('reIDM could not take it', r.error || 'unknown error');
    return;
  }

  if (info.menuItemId === MENU.grab || info.menuItemId === MENU.page) {
    if (tab) openGrabber(tab.id, info.menuItemId === MENU.page ? 'files' : undefined);
  }
});

// ── keeping in touch ────────────────────────────────────────────────────

chrome.alarms.create('reidm-ping', { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((a) => {
  if (a.name === 'reidm-ping') ping();
});

chrome.runtime.onStartup.addListener(() => {
  ping().then((info) => {
    chrome.action.setBadgeBackgroundColor({ color: info ? '#2ea053' : '#d98a1f' }).catch(() => {});
    chrome.action.setBadgeText({ text: info ? '' : '?' }).catch(() => {});
    setTimeout(() => chrome.action.setBadgeText({ text: '' }).catch(() => {}), 3000);
  });
});

// Once per worker start — a module service worker is woken by events without `onStartup`: settle
// whatever a stopped worker left held, and see whether reIDM is there.
const reconciled = reconcilePending();
ping();

// For DevTools and `tools/capture-test.mjs`: a module worker's functions are not globals.
globalThis.reidm = { ping, launchReidm, send, status: () => status, stats, ytdlpFormat, cookieStoreFor, syncPageHook: () => syncPageHook() };
