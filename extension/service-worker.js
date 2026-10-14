// reIDM Integration — background service worker.
//
// Three jobs:
//   1. Intercept downloads the browser is about to start and hand them to reIDM.
//   2. Watch network requests for media (HLS/DASH manifests, progressive video/audio) so the
//      popup can offer them, including media inside cross-origin iframes that a content script
//      cannot see into.
//   3. Answer the popup and the context menus.
//
// Architecture notes that are not obvious:
//   * The download is *paused*, never cancelled, until reIDM has answered. Cancelling is
//     irreversible: if reIDM is not running, or declines the URL, the download is already gone
//     and the user has silently lost it. Pausing is reversible, so a refusal hands the download
//     back to Chrome exactly as it was. The official IDM extension does the same thing —
//     observed in `background.js` 6.43.1: `onCreated` calls `browser.downloads.pause()`, the
//     decision is resolved in `onDeterminingFilename`, and the reply is applied by
//     `kc()` = `browser.downloads.cancel()` when the result is 3 ("taken"), otherwise
//     `browser.downloads.resume()`.
//   * Pausing still loses the race for very small files: Chrome may finish a sub-megabyte
//     download before the pause lands. That is a browser limitation with no workaround; the
//     pause failing is handled below rather than assumed to succeed.
//   * Requests to 127.0.0.1 go out from *here*, never from a content script: a content script's
//     fetch carries the page's origin and would be subject to CORS, while this service worker's
//     POST carries `Origin: chrome-extension://<id>`, which reIDM accepts.
//   * `onCreated` also re-fires for downloads that already existed when the browser started, so
//     the handler filters on `state` before doing anything.

const DEFAULTS = {
  port: 61002,
  capture: true,
  sniffMedia: true,
  ignore: ['blob:', 'data:', 'chrome:', 'chrome-extension://', 'about:', 'file://'],
};

/** Settings are read before the listeners are registered, so `onCreated` never races them.
 *
 *  Listeners are registered *synchronously* at the top level, which is why this does not
 *  `await` its result. Chrome wakes a service worker because a listener for that event is
 *  declared in its top-level script; a listener added only after an `await` can miss the
 *  very event that started the worker — and in practice the worker may not start at all.
 *  So `settings` begins at the defaults and is corrected the moment storage answers; a
 *  first event handled against the defaults is the correct trade for a worker that runs. */
const settings = { ...DEFAULTS };

async function loadSettings() {
  try {
    Object.assign(settings, await chrome.storage.sync.get(DEFAULTS));
  } catch {
    /* keep the defaults */
  }
}
loadSettings();
chrome.storage.onChanged.addListener((changes, area) => {
  if (area !== 'sync') return;
  for (const [k, v] of Object.entries(changes)) settings[k] = v.newValue;
});

const api = (path) => `http://127.0.0.1:${settings.port}${path}`;

// ── talking to reIDM ────────────────────────────────────────────────────

async function ping() {
  try {
    const r = await fetch(api('/ext/ping'), { cache: 'no-store' });
    if (!r.ok) return null;
    return await r.json();
  } catch {
    return null;
  }
}

/** Queue one URL. Returns {ok, id?, outcome?, error?}. */
async function send(details) {
  try {
    const r = await fetch(api('/ext/add'), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(details),
    });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) return { ok: false, error: j.error || `HTTP ${r.status}` };
    return j;
  } catch (e) {
    return { ok: false, error: String(e && e.message ? e.message : e) };
  }
}

const isIgnored = (url) => !url || settings.ignore.some((p) => url.startsWith(p));
const isHttp = (url) => /^https?:/i.test(url || '');

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

// ── 1. download interception ────────────────────────────────────────────

chrome.downloads.onCreated.addListener((item) => {
  if (!settings.capture) return;
  // Started by an extension (possibly us) — leave it alone.
  if (item.byExtensionId) return;
  // `onCreated` also fires for downloads restored at browser start-up.
  if (item.state !== 'in_progress') return;
  if (!isHttp(item.url) || isIgnored(item.url)) return;
  capture(item);
});

/** In-flight hand-offs, so a service worker torn down mid-decision can be reconciled. */
const PENDING_KEY = 'reidm-pending-captures';

async function pendingGet() {
  try {
    return (await chrome.storage.session.get(PENDING_KEY))[PENDING_KEY] || {};
  } catch {
    return {}; // no session storage; the reconcile step below simply does nothing
  }
}

/** Pause, ask reIDM, then release the download one way or the other. Never cancel blind. */
async function capture(item) {
  // The reversible half of the hand-off, and it must happen first.
  let paused = false;
  try {
    await chrome.downloads.pause(item.id);
    paused = true;
  } catch {
    // Refused, or the browser already finished a very small file. Carry on: if reIDM
    // takes the URL we still cancel, and if it does not, nothing has changed.
  }

  const p = await pendingGet();
  p[item.id] = { paused, at: Date.now() };
  try {
    await chrome.storage.session.set({ [PENDING_KEY]: p });
  } catch {
    /* best effort */
  }

  const r = await send({
    url: item.url,
    filename: item.filename ? item.filename.split(/[\\/]/).pop() : nameFromUrl(item.url),
    referer: item.referrer || undefined,
  });

  const q = await pendingGet();
  delete q[item.id];
  try {
    await chrome.storage.session.set({ [PENDING_KEY]: q });
  } catch {
    /* best effort */
  }

  if (r.ok) {
    // reIDM has it: now the browser's copy is redundant.
    chrome.downloads.cancel(item.id, () => void chrome.runtime.lastError);
    chrome.downloads.erase({ id: item.id }, () => void chrome.runtime.lastError);
    badge('✓', '#2ea053', 2500);
  } else if (paused) {
    // Put it back exactly as it was, tags and all.
    chrome.downloads.resume(item.id, () => void chrome.runtime.lastError);
    notify('reIDM could not take the download', r.error || 'unknown error');
  } else {
    notify('reIDM could not take the download', r.error || 'unknown error');
  }
}

/** A decision that never arrived must not leave a download stuck paused. */
async function reconcilePending() {
  const p = await pendingGet();
  let changed = false;
  for (const [id, e] of Object.entries(p)) {
    if (Date.now() - e.at < 30000) continue; // still in flight
    if (e.paused) chrome.downloads.resume(Number(id), () => void chrome.runtime.lastError);
    delete p[id];
    changed = true;
  }
  if (changed) {
    try {
      await chrome.storage.session.set({ [PENDING_KEY]: p });
    } catch {
      /* best effort */
    }
  }
}

function notify(title, message) {
  // A notification needs its own permission; the action badge is enough and needs nothing.
  console.warn(`reIDM: ${title}: ${message}`);
  badge('!', '#c02828', 4000);
}

let badgeTimer = null;
function badge(text, color, clearAfterMs) {
  chrome.action.setBadgeText({ text }).catch(() => {});
  chrome.action.setBadgeBackgroundColor({ color }).catch(() => {});
  if (badgeTimer) clearTimeout(badgeTimer);
  if (clearAfterMs) badgeTimer = setTimeout(() => chrome.action.setBadgeText({ text: '' }).catch(() => {}), clearAfterMs);
}

// ── 2. media sniffing ───────────────────────────────────────────────────

const MEDIA_EXT = /\.(m3u8|mpd|mp4|m4v|webm|mkv|mov|flv|ts|mp3|m4a|flac|ogg|opus|wav|aac)(\?|#|$)/i;
const MANIFEST_EXT = /\.(m3u8|mpd)(\?|#|$)/i;

/** tabId -> Map(url -> {url, kind, at, formats?}) */
const mediaByTab = new Map();

// ── surviving the worker being torn down ────────────────────────────────
//
// An MV3 service worker is stopped whenever Chrome feels like it, and every module-scope
// variable goes with it. Without this, the variants parsed out of a manifest disappear the
// moment the worker idles, and the popup would come back empty on a page that is still
// playing. `chrome.storage.session` is the right home: it lives as long as the browser
// session, which is exactly the lifetime of "a page that is still open".

const MEDIA_KEY = 'reidm-media-by-tab';
let persistTimer = null;

function persistMedia() {
  // Every network request would otherwise rewrite the whole map.
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
      if (!mediaByTab.has(Number(tab))) {
        mediaByTab.set(Number(tab), new Map(list.map((e) => [e.url, e])));
      }
    }
  } catch {
    /* nothing to restore */
  }
}

/** Resolves once the stored map is back. Every reader must await this: a worker that has just
 *  been woken starts handling messages immediately, and reading `mediaByTab` before the
 *  restore lands would report an empty list for a page that is plainly playing video. */
const mediaReady = restoreMedia();

// ── manifest parsing, in the worker ─────────────────────────────────────
//
// The MAIN-world hook also parses manifests, but it can lose a race: a page that fetches its
// manifest from an inline <script> does so before the injected hook has finished loading, and
// the request is then invisible to it. `webRequest` sees the request regardless of what the
// page did, so the worker re-fetches the manifest itself (it has host permissions) and parses
// it here. That is the authoritative source; the hook's copy is a fallback for anything the
// worker cannot re-fetch (a POST-only or signed URL).

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
  if (tab.has(url)) return;
  tab.set(url, { url, kind, at: Date.now(), ...extra });
  // Keep the per-tab list from growing without bound on a long-running player.
  if (tab.size > 200) {
    const oldest = [...tab.entries()].sort((a, b) => a[1].at - b[1].at)[0];
    tab.delete(oldest[0]);
  }
  badge(String(tab.size), '#0a60c8', null);
  persistMedia();
}

chrome.webRequest.onBeforeRequest.addListener(
  (d) => {
    if (!settings.sniffMedia) return;
    if (MANIFEST_EXT.test(d.url)) {
      record(d.tabId, d.url, 'stream');
      // Fetch and parse it out-of-band; the picker can then offer real variants even if the
      // page fetched the manifest before our hook existed.
      formatsForManifest(d.url).then(async (formats) => {
        if (!formats.length) return;
        await mediaReady;
        const tab = mediaByTab.get(d.tabId);
        const entry = tab && tab.get(d.url);
        if (entry) {
          entry.formats = formats;
          persistMedia();
        }
      });
    } else if (MEDIA_EXT.test(d.url)) {
      record(d.tabId, d.url, 'media');
    }
  },
  { urls: ['http://*/*', 'https://*/*'] },
);

// Some players load manifests through fetch with a query string and no recognisable extension.
// The content script's MAIN-world hook reports those, since webRequest cannot see bodies.
chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  if (!msg || typeof msg !== 'object') return;
  const tabId = sender.tab ? sender.tab.id : msg.tabId;

  if (msg.type === 'media') {
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
    if (tabId !== undefined) badge(String((mediaByTab.get(tabId) || new Map()).size), '#0a60c8', null);
    reply?.({ ok: true });
    return true;
  }

  if (msg.type === 'formats-for-tab') {
    // Everything this tab has, including the variants the worker parsed out of a manifest the
    // page fetched before our hook existed. The content script merges this with the hook's view.
    const want = msg.tabId !== undefined ? msg.tabId : tabId;
    mediaReady.then(() => {
      const entries = [...((mediaByTab.get(want) || new Map()).values())].filter(
        (i) => i.formats && i.formats.length,
      );
      reply?.({ ok: true, entries });
    });
    return true; // async reply
  }

  if (msg.type === 'add') {
    send(msg.details).then((r) => reply?.(r));
    return true; // keep the channel open for the async reply
  }

  if (msg.type === 'download-format') {
    downloadFormat(msg).then((r) => reply?.(r));
    return true;
  }

  if (msg.type === 'ping') {
    ping().then((info) => reply?.({ ok: !!info, info }));
    return true;
  }

  if (msg.type === 'media-for-tab') {
    reply?.({ items: [...(mediaByTab.get(msg.tabId) || new Map()).values()] });
    return true;
  }

  if (msg.type === 'clear-tab') {
    mediaByTab.delete(msg.tabId);
    chrome.action.setBadgeText({ text: '' }).catch(() => {});
    reply?.({ ok: true });
    return true;
  }
  return false;
});

chrome.tabs.onRemoved.addListener((tabId) => mediaByTab.delete(tabId));
chrome.tabs.onUpdated.addListener((tabId, info) => {
  if (info.status === 'loading' && info.url) {
    // A new document in the tab: the previous page's media is no longer relevant.
    mediaByTab.delete(tabId);
    chrome.action.setBadgeText({ text: '' }).catch(() => {});
  }
});

// ── 2b. handing a chosen video format to reIDM ──────────────────────────

/** A safe-ish file name: "Title 1080p.mp4". reIDM sanitises it again on its side. */
function formatFilename(title, format, url) {
  const ext = (() => {
    const m = /^video\/(\w+)|^audio\/(\w+)/.exec(format.mime || '');
    if (m) return (m[1] || m[2]).replace('mpegurl', 'm3u8');
    const u = /\.([a-z0-9]{2,4})(?:\?|#|$)/i.exec(url || '');
    return u ? u[1] : 'mp4';
  })();
  const base = (title || 'video')
    .replace(/[/\\?%*:|"<>]/g, '_')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 120);
  const q = format.quality ? ` ${format.quality}` : '';
  const a = format.audioOnly ? ' (audio)' : '';
  return `${base}${q}${a}.${ext}`;
}

/**
 * The user picked a format.
 *
 * A format that already has a direct URL is just a download, so it goes through the same path
 * as any other capture. One that does not is a URL the player protects with a signature
 * (`signatureCipher`); an extension must not reimplement that, so the page URL and the format
 * descriptor go across instead and the manager resolves it. reIDM has no resolver yet, and
 * saying so plainly beats handing it a URL that will 403.
 */
async function downloadFormat(msg) {
  const f = msg.format || {};
  if (!msg.url) {
    return {
      ok: false,
      error:
        'this format needs the player’s signature to be resolved, which reIDM cannot do yet ' +
        '(it needs a yt-dlp-style resolver). Pick a format that is not greyed out.',
    };
  }
  const r = await send({
    url: msg.url,
    filename: formatFilename(msg.title, f, msg.url),
    referer: msg.page || undefined,
  });
  if (r.ok) {
    badge('✓', '#2ea053', 2500);
  } else {
    notify('reIDM could not take the video', r.error || 'unknown error');
  }
  return r;
}

// ── 3. context menus ────────────────────────────────────────────────────

const MENU = { link: 'reidm-link', page: 'reidm-page', media: 'reidm-media', selection: 'reidm-selection' };

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({ id: MENU.link, title: 'Download with reIDM', contexts: ['link'] });
    chrome.contextMenus.create({ id: MENU.media, title: 'Download this media with reIDM', contexts: ['video', 'audio', 'image'] });
    chrome.contextMenus.create({ id: MENU.page, title: 'Download all links on this page with reIDM', contexts: ['page'] });
    chrome.contextMenus.create({ id: MENU.selection, title: 'Download the selected link with reIDM', contexts: ['selection'] });
  });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const referer = tab && tab.url ? tab.url : undefined;

  if (info.menuItemId === MENU.link || info.menuItemId === MENU.media) {
    const url = info.linkUrl || info.srcUrl;
    if (!url) return;
    const r = await send({ url, referer, filename: nameFromUrl(url) || undefined });
    flash(r);
    return;
  }

  if (info.menuItemId === MENU.selection) {
    const url = (info.selectionText || '').trim();
    if (!isHttp(url)) return flash({ ok: false, error: 'the selection is not a link' });
    flash(await send({ url, referer }));
    return;
  }

  if (info.menuItemId === MENU.page) {
    // Collect the page's links in the content script, then queue them all.
    const [res] = await chrome.scripting
      .executeScript({
        target: { tabId: tab.id },
        func: () =>
          [...document.querySelectorAll('a[href]')]
            .map((a) => a.href)
            .filter((h) => /^https?:/i.test(h))
            .slice(0, 500),
      })
      .catch(() => []);
    const urls = (res && res.result) || [];
    if (!urls.length) return flash({ ok: false, error: 'no links found' });
    let queued = 0;
    for (const u of urls) {
      const r = await send({ url: u, referer });
      if (r.ok) queued++;
    }
    flash({ ok: true, queued });
  }
});

function flash(r) {
  if (r && r.ok) {
    badge(r.queued ? String(r.queued) : '✓', '#2ea053', 2500);
  } else {
    notify('reIDM could not take it', (r && r.error) || 'unknown error');
  }
}

chrome.runtime.onStartup.addListener(() => {
  ping().then((info) => badge(info ? '' : '?', info ? '#2ea053' : '#d98a1f', 3000));
  reconcilePending();
});

// A module service worker can also be woken without `onStartup` (an event wakes it), so
// reconcile once per worker start as well.
reconcilePending();
