// reIDM Integration — popup.
//
// Everything here goes through the service worker, which is the only context that may talk to
// 127.0.0.1 (see the note at the top of service-worker.js).

const $ = (id) => document.getElementById(id);
let media = [];
let tab = null;

const ask = (msg) =>
  new Promise((resolve) =>
    chrome.runtime.sendMessage(msg, (r) => {
      void chrome.runtime.lastError;
      resolve(r);
    }),
  );

function say(text, cls = '') {
  $('msg').textContent = text;
  $('msg').className = `msg ${cls}`;
}

function baseName(url) {
  try {
    const u = new URL(url);
    return decodeURIComponent(u.pathname.split('/').filter(Boolean).pop() || '') || u.hostname;
  } catch {
    return url;
  }
}

const humanBytes = (n) => {
  if (!n || n <= 0) return '';
  const u = ['B', 'KB', 'MB', 'GB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 && i ? v.toFixed(1) : Math.round(v)} ${u[i]}`;
};

const ago = (t) => {
  const s = Math.max(0, Math.round((Date.now() - t) / 1000));
  return s < 60 ? `${s}s ago` : s < 3600 ? `${Math.round(s / 60)} min ago` : `${Math.round(s / 3600)} h ago`;
};

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

// ── status ───────────────────────────────────────────────────────────────

async function refreshStatus() {
  const r = await ask({ type: 'ping' });
  const up = !!(r && r.ok);
  $('dot').style.background = up ? 'var(--ok)' : 'var(--err)';
  $('statusText').textContent = up ? `running · ${r.info.version}` : 'not running';
  $('start').hidden = up;
  return up;
}

$('start').addEventListener('click', async () => {
  $('start').disabled = true;
  say('Starting reIDM…');
  const r = await ask({ type: 'launch' });
  $('start').disabled = false;
  if (r && r.ok) {
    say('reIDM is running.', 'ok');
    refreshStatus();
  } else {
    say('Could not start reIDM from here. Open reIDM yourself, or reinstall it so the browser can start it.', 'err');
  }
});

// ── capture switches ─────────────────────────────────────────────────────

function hostOf(url) {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return '';
  }
}

async function loadToggles() {
  const s = await chrome.storage.sync.get({ capture: true, skipHosts: [] });
  $('capture').checked = s.capture;
  const host = tab ? hostOf(tab.url) : '';
  $('host').textContent = host || 'this site';
  $('siteSkip').disabled = !host;
  $('siteSkip').checked = !!host && (s.skipHosts || []).includes(host);
}

$('capture').addEventListener('change', (e) => chrome.storage.sync.set({ capture: e.target.checked }));
$('siteSkip').addEventListener('change', async (e) => {
  const host = tab ? hostOf(tab.url) : '';
  if (!host) return;
  const s = await chrome.storage.sync.get({ skipHosts: [] });
  const set = new Set(s.skipHosts || []);
  if (e.target.checked) set.add(host);
  else set.delete(host);
  await chrome.storage.sync.set({ skipHosts: [...set] });
  say(e.target.checked ? `Downloads from ${host} stay with the browser.` : `reIDM takes downloads from ${host} again.`);
});

// ── recent hand-offs ─────────────────────────────────────────────────────

async function renderRecent() {
  const r = await ask({ type: 'recent-captures' });
  const list = ((r && r.items) || []).slice(0, 4);
  const box = $('recent');
  box.textContent = '';
  if (!list.length) {
    box.appendChild(el('div', 'empty', 'Downloads you start in the browser will appear here as reIDM takes them.'));
    return;
  }
  for (const c of list) {
    const row = el('div', 'row');
    const meta = el('span', 'meta');
    meta.append(el('div', 'fn', c.filename || baseName(c.url)), el('div', 'sub', `${ago(c.at)} · ${c.url}`));
    const res = c.error
      ? el('span', 'res err', 'browser kept it')
      : el('span', 'res ok', c.outcome === 'already_complete' ? 'already have it' : c.outcome === 'existing' ? 'resumed' : c.outcome === 'refreshed' ? 'address refreshed' : 'in reIDM');
    if (c.error) res.title = c.error;
    row.append(meta, res);
    box.appendChild(row);
  }
}

// ── media on this page ───────────────────────────────────────────────────

function renderMedia() {
  const box = $('mediaList');
  box.textContent = '';
  $('downloadSelected').disabled = !media.length;
  if (!media.length) {
    box.appendChild(el('div', 'empty', 'No video or audio found yet. Play something, or grab every resource on the page.'));
    return;
  }
  media.forEach((it, i) => {
    const row = el('label', 'row');
    const cb = el('input');
    cb.type = 'checkbox';
    cb.dataset.i = String(i);
    cb.checked = i === 0;
    const kind = (it.kind || 'media').toLowerCase();
    const meta = el('span', 'meta');
    const bits = [it.mime, humanBytes(it.size), it.formats && it.formats.length ? `${it.formats.length} qualities` : null].filter(Boolean);
    if (it.ytdlp) bits.push('best quality, through yt-dlp');
    meta.append(el('div', 'fn', it.title || baseName(it.url)), el('div', 'sub', bits.length ? `${bits.join(' · ')} — ${it.url}` : it.url));
    meta.lastChild.title = it.url;
    row.append(cb, el('span', `kind ${kind}`, kind), meta);
    box.appendChild(row);
  });
}

async function queue(urls) {
  let ok = 0;
  const errors = [];
  for (const url of urls) {
    const video = media.find((m) => m.url === url && m.ytdlp);
    const r = video
      ? await ask({ type: 'download-video', page: url, title: video.title, incognito: !!(tab && tab.incognito) })
      : await ask({ type: 'add', details: { url, filename: baseName(url) || undefined, referer: tab ? tab.url : undefined } });
    if (r && r.ok) ok++;
    else errors.push(`${baseName(url)}: ${(r && r.error) || 'failed'}`);
  }
  if (ok && !errors.length) say(`Queued ${ok} download${ok === 1 ? '' : 's'}.`, 'ok');
  else if (ok) say(`Queued ${ok}; ${errors.length} failed — ${errors[0]}`);
  else say(errors[0] || 'nothing was queued', 'err');
}

$('downloadSelected').addEventListener('click', () => {
  const urls = [...document.querySelectorAll('#mediaList input:checked')].map((c) => media[Number(c.dataset.i)].url);
  if (!urls.length) return say('Select at least one item.', 'err');
  queue(urls);
});

$('grab').addEventListener('click', async () => {
  if (!tab) return;
  await ask({ type: 'open-grabber', tabId: tab.id });
  window.close();
});

$('clear').addEventListener('click', async () => {
  if (tab) await ask({ type: 'clear-tab', tabId: tab.id });
  media = [];
  renderMedia();
  say('Cleared the list for this page.');
});

$('openOptions').addEventListener('click', () => chrome.runtime.openOptionsPage());

$('addUrl').addEventListener('click', addTyped);
$('url').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') addTyped();
});

function addTyped() {
  const v = $('url').value.trim();
  if (!/^https?:\/\//i.test(v)) return say('Enter an http or https link.', 'err');
  $('url').value = '';
  queue([v]);
}

const isYouTubeVideo = (u) => /^https?:\/\/(www\.|m\.|music\.)?youtube\.com\/(watch\?|shorts\/|live\/)|^https?:\/\/youtu\.be\//i.test(u || '');

// ── startup ──────────────────────────────────────────────────────────────

(async () => {
  [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  loadToggles();
  renderRecent();
  const up = await refreshStatus();
  if (!up) say('reIDM is not running. Downloads stay with the browser until it is — or press Start reIDM.');
  if (tab) {
    const r = await ask({ type: 'media-for-tab', tabId: tab.id });
    media = ((r && r.items) || []).filter((m) => m.kind !== 'link');
    // Streams first (they carry the qualities), then newest first.
    media.sort((a, b) => (b.kind === 'stream') - (a.kind === 'stream') || (b.at || 0) - (a.at || 0));
    // On a YouTube video what the page loads is the player's own traffic, not files: offer the
    // video itself instead, which reIDM fetches through yt-dlp.
    if (isYouTubeVideo(tab.url)) {
      media = media.filter((m) => !['formats', 'mse', 'media', 'stream'].includes(m.kind));
      media.unshift({ url: tab.url, kind: 'video', title: (tab.title || '').replace(/ - YouTube$/, '') || null, ytdlp: true, at: Date.now() });
    }
  }
  renderMedia();
})();
