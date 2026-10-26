// reIDM Integration — resource grabber.
//
// Lists everything a page offers for download, from every frame: links, images (including the
// largest `srcset` candidate, lazy-load attributes and CSS backgrounds), video and audio sources,
// subtitles, embedded objects, social-card media, and every resource the page actually loaded
// (`performance` entries — which is how script-built addresses are found). The service worker
// adds what it saw on the network, with types and sizes from the response headers.
//
// Nothing is downloaded until the user says so; then the selection goes to reIDM in one batch,
// with the page as referer and the browser's cookies for each address.

const $ = (id) => document.getElementById(id);
const params = new URLSearchParams(location.search);
const TAB_ID = Number(params.get('tab'));
/** `select=files`: opened as "Download all links on this page" — the links to files come
 *  ticked, so the user confirms one list instead of reIDM queueing every page link blind. */
const PRESELECT_FILES = params.get('select') === 'files';
const FILE_CATS = new Set(['video', 'audio', 'document', 'archive', 'program']);

const CATEGORIES = [
  ['all', 'All'],
  ['video', 'Video'],
  ['audio', 'Audio'],
  ['image', 'Images'],
  ['document', 'Documents'],
  ['archive', 'Archives'],
  ['program', 'Programs'],
  ['stream', 'Streams'],
  ['subtitle', 'Subtitles'],
  ['other', 'Other'],
  ['page', 'Pages'],
];

const EXT = {
  video: 'mp4 m4v webm mkv mov avi wmv flv 3gp 3g2 mpg mpeg ogv vob ts mts m2ts',
  audio: 'mp3 m4a aac flac wav ogg oga opus wma aif aiff mka',
  image: 'jpg jpeg png gif webp avif svg bmp ico tif tiff heic heif jxl',
  document: 'pdf doc docx xls xlsx ppt pptx pps odt ods odp rtf txt csv epub mobi azw3 djvu chm md',
  archive: 'zip rar 7z tar gz tgz bz2 tbz xz txz zst lz lzma cab iso img dmg',
  program: 'exe msi msix appx apk aab deb rpm pkg appimage jar bin run snap flatpakref',
  stream: 'm3u8 m3u mpd f4m ism',
  subtitle: 'srt vtt ass ssa sub ttml',
  page: 'html htm xhtml php asp aspx jsp cgi shtml',
};
const BY_EXT = new Map(Object.entries(EXT).flatMap(([cat, list]) => list.split(' ').map((e) => [e, cat])));

let items = []; // {url, name, ext, cat, kinds:Set, width, height, size, mime, text}
let selected = new Set(); // urls
let filterCat = 'all';
let sortKey = 'type';
let sortAsc = true;
let pageInfo = { url: '', title: '' };
let collected = false;

// ── collecting ───────────────────────────────────────────────────────────

/** Runs inside every frame of the page (isolated world). Keep it self-contained. */
function collectInFrame() {
  const out = [];
  const seen = new Set();
  const add = (raw, kind, extra) => {
    if (!raw || typeof raw !== 'string') return;
    let u;
    try {
      u = new URL(raw.trim(), document.baseURI);
    } catch {
      return;
    }
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return;
    u.hash = '';
    const key = `${kind}\u0000${u.href}`;
    if (seen.has(key)) return;
    seen.add(key);
    out.push(Object.assign({ url: u.href, kind }, extra || {}));
  };
  /** The largest candidate of a `srcset`: by width descriptor, else by density. */
  const largest = (srcset) => {
    let best = null;
    let score = -1;
    for (const part of String(srcset || '').split(/,\s+/)) {
      const [u, d] = part.trim().split(/\s+/);
      if (!u) continue;
      const n = d ? parseFloat(d) * (d.endsWith('w') ? 1 : 1000) : 1;
      if (n > score) {
        score = n;
        best = u;
      }
    }
    return best;
  };

  for (const a of document.querySelectorAll('a[href], area[href]')) {
    const text = (a.textContent || a.getAttribute('title') || a.getAttribute('aria-label') || '').replace(/\s+/g, ' ').trim();
    add(a.href, 'link', { text: text.slice(0, 140) || null, download: a.getAttribute('download') || null });
  }
  for (const img of document.images) {
    const dims = { width: img.naturalWidth || null, height: img.naturalHeight || null, text: img.alt || null };
    add(img.currentSrc || img.src, 'image', dims);
    if (img.srcset) add(largest(img.srcset), 'image', { text: img.alt || null });
    for (const at of ['data-src', 'data-original', 'data-lazy-src', 'data-lazy', 'data-full', 'data-large', 'data-hires', 'data-zoom-image']) {
      const v = img.getAttribute(at);
      if (v) add(v, 'image', { text: img.alt || null });
    }
    const lazySet = img.getAttribute('data-srcset');
    if (lazySet) add(largest(lazySet), 'image', { text: img.alt || null });
  }
  for (const s of document.querySelectorAll('picture source[srcset], source[data-srcset]')) {
    add(largest(s.getAttribute('srcset') || s.getAttribute('data-srcset')), 'image');
  }
  for (const m of document.querySelectorAll('video, audio')) {
    const kind = m.tagName === 'VIDEO' ? 'video' : 'audio';
    for (const v of [m.currentSrc, m.getAttribute('src')]) if (v && !v.startsWith('blob:')) add(v, kind);
    if (m.poster) add(m.poster, 'image', { text: 'video poster' });
    for (const s of m.querySelectorAll('source[src]')) add(s.getAttribute('src'), kind, { mime: s.getAttribute('type') || null });
    for (const t of m.querySelectorAll('track[src]')) add(t.getAttribute('src'), 'subtitle', { text: t.label || t.srclang || null });
  }
  for (const e of document.querySelectorAll('embed[src], object[data]')) add(e.getAttribute('src') || e.getAttribute('data'), 'object');
  for (const f of document.querySelectorAll('iframe[src], frame[src]')) add(f.getAttribute('src'), 'frame');
  for (const m of document.querySelectorAll('meta[property^="og:"], meta[name^="twitter:"]')) {
    const p = (m.getAttribute('property') || m.getAttribute('name') || '').toLowerCase();
    const v = m.getAttribute('content');
    if (/(:image|:video|:audio)(:url|:secure_url)?$/.test(p)) add(v, p.includes('video') ? 'video' : p.includes('audio') ? 'audio' : 'image', { text: 'shared-card media' });
  }
  for (const l of document.querySelectorAll('link[rel~="icon"], link[rel="apple-touch-icon"], link[rel="image_src"], link[rel="preload"][as="image"], link[rel="preload"][as="video"]')) {
    add(l.href, 'image');
  }
  // CSS backgrounds: bounded, because computed style on a huge page is not free.
  const els = document.querySelectorAll('body *');
  for (let i = 0; i < els.length && i < 5000; i++) {
    const bg = getComputedStyle(els[i]).backgroundImage;
    if (!bg || bg === 'none') continue;
    for (const m of bg.matchAll(/url\(\s*["']?([^"')]+)["']?\s*\)/g)) add(m[1], 'image', { css: true });
  }
  // Everything the page actually fetched, which is how addresses built by script show up.
  for (const e of performance.getEntriesByType('resource')) {
    add(e.name, 'resource', { initiator: e.initiatorType || null, size: e.decodedBodySize || null });
  }
  return { url: location.href, title: document.title, top: window === window.top, items: out };
}

function extOf(url) {
  try {
    const last = new URL(url).pathname.split('/').pop() || '';
    const m = /\.([a-z0-9]{1,10})$/i.exec(last);
    return m ? m[1].toLowerCase() : '';
  } catch {
    return '';
  }
}

function nameOf(url, fallback) {
  try {
    const u = new URL(url);
    const last = decodeURIComponent(u.pathname.split('/').filter(Boolean).pop() || '');
    return last || fallback || u.hostname;
  } catch {
    return url;
  }
}

function categorise(it) {
  const byExt = BY_EXT.get(it.ext);
  if (byExt && byExt !== 'page') return byExt;
  const mime = (it.mime || '').toLowerCase();
  if (/mpegurl|dash\+xml/.test(mime)) return 'stream';
  if (mime.startsWith('video/')) return 'video';
  if (mime.startsWith('audio/')) return 'audio';
  if (mime.startsWith('image/')) return 'image';
  if (it.kinds.has('video')) return 'video';
  if (it.kinds.has('audio')) return 'audio';
  if (it.kinds.has('image')) return 'image';
  if (it.kinds.has('subtitle')) return 'subtitle';
  if (it.kinds.has('stream')) return 'stream';
  if (byExt === 'page' || it.kinds.has('frame')) return 'page';
  // A plain link with no extension is almost always another page.
  if (!it.ext && it.kinds.size === 1 && it.kinds.has('link') && !it.download) return 'page';
  if (it.kinds.has('resource') && it.kinds.size === 1) {
    // Scripts, stylesheets and API calls the page made are not downloads.
    if (['script', 'link', 'css', 'xmlhttprequest', 'fetch', 'beacon', 'other'].includes(it.initiator) && !byExt) return 'page';
  }
  return 'other';
}

async function collect() {
  setMsg('Reading the page…');
  let frames = [];
  try {
    frames = await chrome.scripting.executeScript({ target: { tabId: TAB_ID, allFrames: true }, func: collectInFrame });
  } catch (e) {
    setMsg(`This page cannot be read (${e.message || e}). Browser pages and the Web Store are off limits to extensions.`, 'err');
  }
  const byUrl = new Map();
  const merge = (raw) => {
    let it = byUrl.get(raw.url);
    if (!it) {
      it = { url: raw.url, kinds: new Set(), ext: extOf(raw.url), width: null, height: null, size: null, mime: null, text: null, download: null, initiator: null };
      byUrl.set(raw.url, it);
    }
    it.kinds.add(raw.kind);
    for (const k of ['width', 'height', 'size', 'mime', 'text', 'download', 'initiator']) {
      if (raw[k] != null && raw[k] !== '' && it[k] == null) it[k] = raw[k];
    }
  };
  for (const f of frames) {
    const r = f && f.result;
    if (!r) continue;
    if (r.top) pageInfo = { url: r.url, title: r.title };
    r.items.forEach(merge);
  }
  // What the worker saw on the network for this tab: streams, media without a recognisable
  // address, and sizes from the response headers.
  try {
    const r = await chrome.runtime.sendMessage({ type: 'media-for-tab', tabId: TAB_ID });
    for (const m of (r && r.items) || []) {
      merge({ url: m.url, kind: m.kind === 'stream' ? 'stream' : m.kind === 'audio' ? 'audio' : 'video', mime: m.mime || null, size: m.size || null });
      for (const f of m.formats || []) if (f.url) merge({ url: f.url, kind: 'stream', text: [f.quality, f.videoCodec].filter(Boolean).join(' ') || null });
    }
  } catch {
    /* the worker is asleep or the tab is gone; the page scan is still useful */
  }
  items = [...byUrl.values()].map((it) => ({ ...it, name: nameOf(it.url, it.download), cat: categorise(it) }));
  for (const it of items) if (it.download) it.name = it.download;
  selected = new Set([...selected].filter((u) => byUrl.has(u)));
  if (PRESELECT_FILES && !collected) {
    for (const it of items) if (it.kinds.has('link') && FILE_CATS.has(it.cat)) selected.add(it.url);
  }
  collected = true;
  $('page').textContent = pageInfo.title ? `${pageInfo.title} — ${pageInfo.url}` : pageInfo.url || '';
  document.title = `reIDM — ${pageInfo.title || 'Resource grabber'}`;
  render();
  setMsg(`${items.length} addresses found`);
}

// ── showing ──────────────────────────────────────────────────────────────

const humanBytes = (n) => {
  if (!n || n <= 0) return '';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 && i ? v.toFixed(1) : Math.round(v)} ${u[i]}`;
};

function filterFn() {
  const raw = $('filter').value.trim();
  let test = () => true;
  if (raw.length > 2 && raw.startsWith('/') && raw.lastIndexOf('/') > 0) {
    try {
      const re = new RegExp(raw.slice(1, raw.lastIndexOf('/')), raw.slice(raw.lastIndexOf('/') + 1) || 'i');
      test = (it) => re.test(it.url) || re.test(it.name);
    } catch {
      test = () => false;
    }
  } else if (raw) {
    const q = raw.toLowerCase();
    test = (it) => it.url.toLowerCase().includes(q) || it.name.toLowerCase().includes(q) || (it.text || '').toLowerCase().includes(q);
  }
  const sameSite = $('sameSite').checked;
  // "Same site" by registrable domain, approximated without a public-suffix list: the last
  // two labels, or three under a two-letter country code with a short second level
  // (`example.co.uk`), so that `co.uk` doesn't count as one site.
  const host = (() => {
    try {
      const labels = new URL(pageInfo.url).hostname.split('.');
      const cc = labels.length > 2 && labels[labels.length - 1].length === 2 && labels[labels.length - 2].length <= 3;
      return labels.slice(cc ? -3 : -2).join('.');
    } catch {
      return '';
    }
  })();
  const showPages = $('showPages').checked;
  const minPx = Number($('minPx').value) || 0;
  return (it, ignoreCat = false) => {
    if (!ignoreCat && filterCat !== 'all' && it.cat !== filterCat) return false;
    if (it.cat === 'page' && !showPages && filterCat !== 'page') return false;
    if (it.cat === 'image' && minPx && it.width && it.height && Math.max(it.width, it.height) < minPx) return false;
    if (sameSite && host) {
      try {
        const h = new URL(it.url).hostname;
        if (h !== host && !h.endsWith(`.${host}`)) return false;
      } catch {
        return false;
      }
    }
    return test(it);
  };
}

function visible() {
  const ok = filterFn();
  const order = CATEGORIES.map((c) => c[0]);
  const key = {
    type: (it) => `${String(order.indexOf(it.cat)).padStart(2, '0')}${it.name.toLowerCase()}`,
    name: (it) => it.name.toLowerCase(),
    dim: (it) => (it.width || 0) * (it.height || 0),
    size: (it) => it.size || 0,
    src: (it) => [...it.kinds].sort().join(','),
  }[sortKey];
  return items
    .filter((it) => ok(it))
    .sort((a, b) => {
      const x = key(a);
      const y = key(b);
      const c = typeof x === 'number' ? x - y : String(x).localeCompare(String(y));
      return sortAsc ? c : -c;
    });
}

function renderChips() {
  const ok = filterFn();
  const counts = Object.fromEntries(CATEGORIES.map(([c]) => [c, 0]));
  for (const it of items) {
    if (!ok(it, true)) continue;
    counts.all += it.cat === 'page' && !$('showPages').checked ? 0 : 1;
    counts[it.cat] = (counts[it.cat] || 0) + 1;
  }
  const box = $('chips');
  box.textContent = '';
  for (const [c, label] of CATEGORIES) {
    if (c === 'page' && !$('showPages').checked && filterCat !== 'page') continue;
    const b = document.createElement('button');
    b.className = `chip${filterCat === c ? ' on' : ''}`;
    b.disabled = !counts[c] && c !== 'all';
    b.textContent = `${label} `;
    const n = document.createElement('span');
    n.className = 'n';
    n.textContent = String(counts[c] || 0);
    b.appendChild(n);
    b.addEventListener('click', () => {
      filterCat = c;
      render();
    });
    box.appendChild(b);
  }
}

function render() {
  renderChips();
  const rows = visible();
  const tbody = $('rows');
  tbody.textContent = '';
  const frag = document.createDocumentFragment();
  for (const it of rows) {
    const tr = document.createElement('tr');
    if (selected.has(it.url)) tr.className = 'sel';

    const tdc = document.createElement('td');
    tdc.className = 'check';
    const cb = document.createElement('input');
    cb.type = 'checkbox';
    cb.checked = selected.has(it.url);
    cb.addEventListener('change', () => {
      if (cb.checked) selected.add(it.url);
      else selected.delete(it.url);
      tr.className = cb.checked ? 'sel' : '';
      updateButtons();
    });
    tdc.appendChild(cb);

    const tdt = document.createElement('td');
    tdt.className = 'type';
    const tag = document.createElement('span');
    tag.className = `tag ${it.cat}`;
    tag.textContent = it.ext || it.cat;
    if (it.cat === 'image') {
      const img = document.createElement('img');
      img.className = 'thumb';
      img.loading = 'lazy';
      img.referrerPolicy = 'no-referrer';
      img.alt = '';
      // An address that does not load (a 404 favicon, a hot-link-protected image) shows its type.
      img.addEventListener('error', () => img.replaceWith(tag), { once: true });
      img.src = it.url;
      tdt.appendChild(img);
    } else {
      tdt.appendChild(tag);
    }

    const tdn = document.createElement('td');
    tdn.className = 'name';
    const fn = document.createElement('div');
    fn.className = 'fn';
    fn.textContent = it.name;
    fn.title = it.text ? `${it.name}\n${it.text}` : it.name;
    const url = document.createElement('div');
    url.className = 'url';
    url.textContent = it.url;
    url.title = it.url;
    tdn.append(fn, url);

    const tdd = document.createElement('td');
    tdd.className = 'dim';
    tdd.textContent = it.width && it.height ? `${it.width} × ${it.height}` : '';

    const tds = document.createElement('td');
    tds.className = 'size';
    tds.textContent = humanBytes(it.size);

    const tdsrc = document.createElement('td');
    tdsrc.className = 'src';
    tdsrc.textContent = [...it.kinds].map((k) => (k === 'resource' ? 'loaded' : k)).join(', ');

    tr.append(tdc, tdt, tdn, tdd, tds, tdsrc);
    tr.addEventListener('click', (ev) => {
      if (ev.target === cb) return;
      cb.checked = !cb.checked;
      cb.dispatchEvent(new Event('change'));
    });
    frag.appendChild(tr);
  }
  tbody.appendChild(frag);
  $('empty').hidden = rows.length > 0;
  const shownSelected = rows.filter((it) => selected.has(it.url)).length;
  $('all').checked = rows.length > 0 && shownSelected === rows.length;
  $('all').indeterminate = shownSelected > 0 && shownSelected < rows.length;
  for (const th of document.querySelectorAll('thead th[data-sort]')) {
    th.classList.toggle('sorted', th.dataset.sort === sortKey);
    th.classList.toggle('asc', th.dataset.sort === sortKey && sortAsc);
  }
  updateButtons();
}

function chosen() {
  return items.filter((it) => selected.has(it.url));
}

function updateButtons() {
  const n = chosen().length;
  $('download').disabled = !n;
  $('later').disabled = !n;
  $('copy').disabled = !n;
  $('export').disabled = !n;
  $('download').textContent = n ? `Download ${n} with reIDM` : 'Download with reIDM';
}

function setMsg(text, cls = '') {
  $('msg').textContent = text;
  $('msg').className = cls;
}

// ── acting ───────────────────────────────────────────────────────────────

async function queue(later) {
  const list = chosen();
  if (!list.length) return;
  setMsg(`Handing ${list.length} to reIDM…`);
  const r = await chrome.runtime.sendMessage({
    type: 'add-batch',
    tabId: TAB_ID,
    referer: pageInfo.url,
    later,
    items: list.map((it) => ({ url: it.url, filename: it.download || undefined })),
  });
  if (!r || !r.ok) return setMsg(`reIDM could not take them: ${(r && r.error) || 'no answer'}`, 'err');
  const res = r.results || [];
  const count = (k) => res.filter((x) => x.outcome === k).length;
  const failed = res.filter((x) => x.error);
  const parts = [];
  if (count('new')) parts.push(`${count('new')} ${later ? 'added for later' : 'queued'}`);
  if (count('existing')) parts.push(`${count('existing')} already in the list`);
  if (count('already_complete')) parts.push(`${count('already_complete')} already downloaded`);
  if (failed.length) parts.push(`${failed.length} refused (${failed[0].error})`);
  setMsg(parts.join(' · ') || 'Nothing was added', failed.length && !count('new') ? 'err' : 'ok');
}

/** Ask each server how big a file is: HEAD first, a one-byte range when HEAD is refused. */
async function sizeOf(url) {
  const fromHeaders = (h) => {
    const cr = /\/(\d+)\s*$/.exec(h.get('content-range') || '');
    if (cr) return Number(cr[1]);
    const cl = Number(h.get('content-length'));
    return cl > 0 ? cl : null;
  };
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), 12000);
  try {
    const r = await fetch(url, { method: 'HEAD', credentials: 'include', signal: ctl.signal });
    if (r.ok) {
      const n = fromHeaders(r.headers);
      if (n) return { size: n, mime: (r.headers.get('content-type') || '').split(';')[0] || null };
    }
    const g = await fetch(url, { headers: { Range: 'bytes=0-0' }, credentials: 'include', signal: ctl.signal });
    const n = g.status === 206 ? fromHeaders(g.headers) : null;
    g.body && g.body.cancel().catch(() => {});
    return { size: n, mime: (g.headers.get('content-type') || '').split(';')[0] || null };
  } catch {
    return { size: null, mime: null };
  } finally {
    clearTimeout(t);
  }
}

async function getSizes() {
  // Only addresses that name a file: asking a server about a plain page link is a request to
  // that page, with the user's cookies, and some such links do things (log out, unsubscribe).
  const want = visible().filter((it) => !it.size && it.cat !== 'page' && it.ext);
  if (!want.length) return setMsg('Every listed file already has a size.');
  $('sizes').disabled = true;
  let done = 0;
  const next = async () => {
    while (want.length) {
      const it = want.shift();
      const r = await sizeOf(it.url);
      if (r.size) it.size = r.size;
      if (r.mime && !it.mime) {
        it.mime = r.mime;
        it.cat = categorise(it);
      }
      done++;
      if (done % 5 === 0) setMsg(`Asking for sizes… ${done}`);
    }
  };
  await Promise.all(Array.from({ length: 6 }, next));
  $('sizes').disabled = false;
  render();
  setMsg(`Sizes checked for ${done} file(s)`, 'ok');
}

function download(name, text) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([text], { type: 'text/plain' }));
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 5000);
}

// ── wiring ───────────────────────────────────────────────────────────────

$('download').addEventListener('click', () => queue(false));
$('later').addEventListener('click', () => queue(true));
$('sizes').addEventListener('click', getSizes);
$('rescan').addEventListener('click', collect);
$('copy').addEventListener('click', async () => {
  await navigator.clipboard.writeText(chosen().map((it) => it.url).join('\n'));
  setMsg(`Copied ${chosen().length} address(es)`, 'ok');
});
$('export').addEventListener('click', () => {
  // IDM's .ef2, which IDM and reIDM both import; the page goes along as the referer.
  const ef2 = chosen()
    .map((it) => `<\r\n${it.url}\r\n${pageInfo.url ? `referer: ${pageInfo.url}\r\n` : ''}>\r\n`)
    .join('');
  download('reidm-grabbed.ef2', ef2);
  setMsg(`Exported ${chosen().length} address(es) as an IDM/reIDM .ef2 list`, 'ok');
});
$('all').addEventListener('change', (e) => {
  for (const it of visible()) {
    if (e.target.checked) selected.add(it.url);
    else selected.delete(it.url);
  }
  render();
});
for (const id of ['filter', 'sameSite', 'showPages', 'minPx']) $(id).addEventListener('input', render);
for (const th of document.querySelectorAll('thead th[data-sort]')) {
  th.addEventListener('click', () => {
    if (sortKey === th.dataset.sort) sortAsc = !sortAsc;
    else {
      sortKey = th.dataset.sort;
      sortAsc = sortKey === 'type' || sortKey === 'name';
    }
    render();
  });
}

(async () => {
  chrome.runtime.sendMessage({ type: 'ping' }).then((r) => {
    $('dot').style.background = r && r.ok ? 'var(--ok)' : 'var(--err)';
    $('statusText').textContent = r && r.ok ? `reIDM ${r.info.version} is running` : 'reIDM is not running — it will be started when you download';
  });
  if (!Number.isInteger(TAB_ID)) return setMsg('Open this from the reIDM popup or the page’s right-click menu.', 'err');
  await collect();
  if (PRESELECT_FILES) setMsg(selected.size ? `${selected.size} links to files are ticked. Check them, then press Download.` : 'No links to files on this page.');
})();
