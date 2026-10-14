// reIDM Integration — page-side scanner and picker.
//
// Runs in every frame at document_start. It does three things:
//   * injects the MAIN-world hook (inject.js) that watches fetch/XHR, because the extension
//     world cannot see the page's own requests and `chrome.webRequest` cannot read bodies;
//   * reports media elements and media links it can see in the DOM;
//   * shows a "Download with reIDM" button next to a playing video, which opens a picker
//     listing the codecs and resolutions the player actually offers.
//
// It never talks to reIDM itself. A content script's fetch carries the *page's* origin, so it
// would be treated as a cross-origin page request; the service worker does the talking.
//
// The button and the panel live in a closed shadow root. On a site like YouTube the page's own
// CSS is aggressive and a plain element inherits enough of it to render wrong.

(() => {
  if (window.__reidmContentLoaded) return;
  window.__reidmContentLoaded = true;

  const MEDIA_SEL = 'video, audio, source';
  const LINK_EXT = /\.(m3u8|mpd|mp4|m4v|webm|mkv|mov|flv|mp3|m4a|flac|ogg|opus|wav|aac|zip|rar|7z|iso|dmg|exe|msi|deb|pdf|apk|img|bin|tar\.gz|tgz)(\?|#|$)/i;

  const isHttp = (u) => /^https?:/i.test(u || '');
  const absolute = (u) => {
    try {
      return new URL(u, document.baseURI).href;
    } catch {
      return null;
    }
  };

  // Two independent sources feed the picker, and neither may clobber the other:
  //   * the page hook (inject.js) sees player responses and manifests the page fetched itself;
  //   * the worker parses manifests it saw through webRequest, which is the only way to catch
  //     a manifest a page fetched from an inline script before the hook finished loading.
  // They are kept apart and rendered as a union, because the hook's answer for a page is
  // often just the one <video> source, and letting it assign over the worker's list silently
  // downgrades a four-variant stream to a single progressive file.
  let hookFormats = [];
  let workerFormats = [];
  let pageMeta = { source: null, title: null, page: null };

  // ── MAIN-world hook ───────────────────────────────────────────────────
  const inject = () => {
    try {
      const s = document.createElement('script');
      s.src = chrome.runtime.getURL('inject.js');
      s.async = false;
      (document.head || document.documentElement).appendChild(s);
      s.remove();
    } catch {
      /* A strict CSP can block this; the DOM scan still works. */
    }
  };
  if (document.documentElement) inject();
  else document.addEventListener('readystatechange', inject, { once: true });

  const humanBytes = (n) => {
    if (!n || n <= 0) return null;
    const u = ['B', 'KB', 'MB', 'GB'];
    let v = n, i = 0;
    while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
    return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${u[i]}`;
  };

  // Messages from the MAIN-world hook.
  window.addEventListener('message', (e) => {
    if (e.source !== window || !e.data || e.data.__reidm !== true) return;

    // The answer to a direct query for the current page's formats.
    if (e.data.kind === 'formats-answer') {
      applyHook(e.data.payload || {});
      return;
    }

    const items = (e.data.items || []).filter((i) => isHttp(i.url));
    if (!items.length) return;

    for (const it of items) {
      if (it.kind === 'formats' && it.formats && it.formats.length) applyHook(it);
    }

    const plain = items.filter((i) => i.kind !== 'formats');
    if (plain.length) chrome.runtime.sendMessage({ type: 'media', items: plain }).catch(() => {});
  });

  /** The page hook's view of this page. Replaces the previous hook view, not the worker's. */
  function applyHook(p) {
    if (!p || !p.formats || !p.formats.length) return;
    hookFormats = p.formats;
    pageMeta = {
      source: p.source || pageMeta.source,
      title: p.title || pageMeta.title || document.title || null,
      page: p.page || pageMeta.page || location.href,
    };
    refresh();
  }

  /** Ask the hook for the authoritative list for whatever is playing right now. */
  const queryFormats = () => {
    try {
      window.postMessage({ __reidmQuery: true, id: Date.now() }, '*');
    } catch {
      /* ignore */
    }
  };

  /** Merge formats the worker parsed itself. The page hook can miss a manifest that was
   *  fetched from an inline script before the hook finished loading, and the worker sees
   *  every request through webRequest regardless of when the page made it. */
  /** The worker's view: every manifest it could re-fetch and parse for this tab. */
  function applyWorker(entries) {
    const seen = new Set();
    const list = [];
    for (const e of entries || []) {
      for (const f of e.formats || []) {
        const k = f.url || `${e.url}#${f.quality}`;
        if (seen.has(k)) continue;
        seen.add(k);
        list.push(f);
      }
    }
    workerFormats = list;
    refresh();
  }

  const refreshFromWorker = () => {
    chrome.runtime
      .sendMessage({ type: 'formats-for-tab' })
      .then((r) => {
        if (r && r.ok) applyWorker(r.entries);
      })
      .catch(() => {});
  };

  /** Hook formats first (they carry the friendlier labels), then any the worker alone has. */
  function combined() {
    const out = [];
    const seen = new Set();
    for (const f of [...hookFormats, ...workerFormats]) {
      const k = f.url || `${f.quality}|${f.bitrate}|${f.codecs}`;
      if (seen.has(k)) continue;
      seen.add(k);
      out.push(f);
    }
    return out;
  }

  let lastSig = '';

  /** Publish the union. The signature guard keeps the DOM from being rebuilt on every tick. */
  function refresh() {
    const fs = combined();
    if (!fs.length) return;
    const sig = `${pageMeta.source}|${fs.length}|${fs.map((f) => f.url || f.quality).join(',')}`;
    if (sig === lastSig) return;
    lastSig = sig;
    ensureUi();
    render(fs);
  }

  // ── DOM scan ──────────────────────────────────────────────────────────
  const scan = () => {
    const items = [];
    const push = (url, kind) => {
      const u = absolute(url);
      if (u && isHttp(u)) items.push({ url: u, kind });
    };

    for (const el of document.querySelectorAll(MEDIA_SEL)) {
      const tag = el.tagName.toLowerCase();
      // A blob: URL means MSE; there is nothing useful in it and the hook has the real ones.
      if (el.src && !/^blob:/i.test(el.src)) push(el.src, tag === 'source' ? mediaKind(el) : tag);
      const current = el.getAttribute && el.getAttribute('src');
      if (current) push(current, tag);
    }
    for (const a of document.querySelectorAll('a[href]')) {
      const href = a.getAttribute('href') || '';
      if (LINK_EXT.test(href)) push(href, 'link');
    }
    if (items.length) chrome.runtime.sendMessage({ type: 'media', items }).catch(() => {});
    if (document.querySelector('video')) ensureUi();
  };

  function mediaKind(sourceEl) {
    const t = (sourceEl.getAttribute('type') || '').toLowerCase();
    if (t.includes('mpegurl') || t.includes('dash')) return 'stream';
    if (t.startsWith('video/')) return 'video';
    if (t.startsWith('audio/')) return 'audio';
    return 'media';
  }

  // ── the button and the picker ─────────────────────────────────────────
  //
  // Anchored to the video's own box rather than to a fixed corner, so it sits where the user
  // is already looking. Re-positioned on scroll and resize because the box moves.

  let host = null, root = null, btn = null, panel = null, videoEl = null;

  const CSS = `
    :host { all: initial; }
    .wrap { position: fixed; z-index: 2147483600; font: 12px/1.35 system-ui, "Segoe UI", Roboto, sans-serif; }
    .btn {
      display: flex; align-items: center; gap: 6px; cursor: pointer; user-select: none;
      background: rgba(18,18,20,.86); color: #fff; border: 1px solid rgba(255,255,255,.18);
      border-radius: 6px; padding: 5px 9px; box-shadow: 0 2px 10px rgba(0,0,0,.35);
      backdrop-filter: blur(4px);
    }
    .btn:hover { background: rgba(28,28,32,.95); }
    .btn b { font-weight: 600; }
    .dot { width: 7px; height: 7px; border-radius: 50%; background: #3ea6ff; flex: none; }
    .panel {
      margin-top: 6px; min-width: 320px; max-width: 400px; max-height: 340px; overflow: auto;
      background: #1b1b1f; color: #eaeaea; border: 1px solid rgba(255,255,255,.16);
      border-radius: 8px; box-shadow: 0 8px 28px rgba(0,0,0,.5); padding: 6px;
    }
    .panel[hidden] { display: none; }
    .hd { padding: 6px 8px 2px; color: #9a9aa5; font-size: 11px; text-transform: uppercase; letter-spacing: .04em; }
    .row {
      display: flex; align-items: center; gap: 8px; padding: 7px 8px; border-radius: 6px; cursor: pointer;
    }
    .row:hover { background: rgba(62,166,255,.16); }
    .row .q { font-weight: 600; min-width: 56px; }
    .row .meta { color: #9a9aa5; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .row .sz { color: #c8c8cf; white-space: nowrap; }
    .row.no { opacity: .55; cursor: not-allowed; }
    .row.no:hover { background: transparent; }
    .note { padding: 6px 8px; color: #9a9aa5; font-size: 11px; border-top: 1px solid rgba(255,255,255,.1); margin-top: 4px; }
    .tag { font-size: 10px; padding: 1px 5px; border-radius: 3px; background: rgba(255,255,255,.12); color: #cfcfd6; }
    .empty { padding: 10px; color: #9a9aa5; }
  `;

  function ensureUi() {
    if (host || !document.documentElement) return;
    try {
      host = document.createElement('div');
      host.style.cssText = 'all:initial;position:static';
      root = host.attachShadow({ mode: 'closed' });
      const style = document.createElement('style');
      style.textContent = CSS;
      const wrap = document.createElement('div');
      wrap.className = 'wrap';
      wrap.style.display = 'none';

      btn = document.createElement('div');
      btn.className = 'btn';
      btn.innerHTML = '<span class="dot"></span><b>Download with reIDM</b>';
      btn.addEventListener('click', (ev) => {
        ev.preventDefault();
        ev.stopPropagation();
        queryFormats();
        refreshFromWorker();
        panel.hidden = !panel.hidden;
      });

      panel = document.createElement('div');
      panel.className = 'panel';
      panel.hidden = true;

      wrap.appendChild(btn);
      wrap.appendChild(panel);
      root.append(style, wrap);
      document.documentElement.appendChild(host);

      host.__wrap = wrap;
      position();
      window.addEventListener('scroll', position, { passive: true, capture: true });
      window.addEventListener('resize', position, { passive: true });
      setInterval(position, 1000); // the player box moves without a scroll event too
    } catch {
      /* a page that forbids decoration should still work without us */
    }
  }

  /** Keep the button in the video's top-right corner. */
  function position() {
    if (!host || !host.__wrap) return;
    const v = document.querySelector('video');
    if (v) videoEl = v;
    if (!videoEl || !document.contains(videoEl)) {
      host.__wrap.style.display = 'none';
      return;
    }
    const r = videoEl.getBoundingClientRect();
    if (r.width < 120 || r.height < 80) {
      host.__wrap.style.display = 'none';
      return;
    }
    host.__wrap.style.display = '';
    // Right-aligned inside the video box; clamped so it cannot leave the viewport.
    const right = Math.max(8, window.innerWidth - r.right + 8);
    const top = Math.min(Math.max(8, r.top + 8), window.innerHeight - 40);
    host.__wrap.style.right = `${right}px`;
    host.__wrap.style.top = `${top}px`;
    host.__wrap.style.left = 'auto';
  }

  function close() {
    if (host && host.parentNode) host.parentNode.removeChild(host);
    host = root = btn = panel = null;
  }

  /** Put the formats in a sensible order: best picture first, audio-only last. */
  function sortFormats(fs) {
    return [...fs].sort((a, b) => {
      const ha = a.height || parseInt(a.quality, 10) || 0;
      const hb = b.height || parseInt(b.quality, 10) || 0;
      if (!!a.audioOnly !== !!b.audioOnly) return a.audioOnly ? 1 : -1;
      if (hb !== ha) return hb - ha;
      return (b.bitrate || 0) - (a.bitrate || 0);
    });
  }

  function render(fs) {
    if (!panel || !fs || !fs.length) return;

    panel.textContent = '';
    const hd = document.createElement('div');
    hd.className = 'hd';
    hd.textContent = pageMeta.title
      ? `${pageMeta.source || 'video'} — ${pageMeta.title}`
      : pageMeta.source || 'video';
    panel.appendChild(hd);

    const anyCiphered = fs.some((f) => f.ciphered);

    for (const f of sortFormats(fs)) {
      const row = document.createElement('div');
      const unusable = !f.url || f.ciphered;
      row.className = unusable ? 'row no' : 'row';

      const q = document.createElement('span');
      q.className = 'q';
      q.textContent = f.quality || (f.audioOnly ? 'audio' : '?');

      const meta = document.createElement('span');
      meta.className = 'meta';
      const bits = [];
      if (f.videoCodec) bits.push(f.videoCodec);
      if (f.audioCodec) bits.push(f.audioCodec);
      if (!bits.length && f.codecs) bits.push(f.codecs);
      if (f.fps) bits.push(`${f.fps}fps`);
      if (f.bitrate) bits.push(`${Math.round(f.bitrate / 1000)} kbps`);
      if (f.mime) bits.push(f.mime.replace(/^video\/|^audio\//, ''));
      else if (f.kind) bits.push(String(f.kind).replace(/-/g, ' '));
      meta.textContent = bits.join(' · ') || '—';

      const sz = document.createElement('span');
      sz.className = 'sz';
      sz.textContent = humanBytes(f.size) || '';

      row.append(q, meta, sz);

      if (f.audioOnly) {
        const t = document.createElement('span');
        t.className = 'tag';
        t.textContent = 'audio';
        row.appendChild(t);
      }

      if (!unusable) {
        row.addEventListener('click', (ev) => {
          ev.preventDefault();
          ev.stopPropagation();
          row.style.opacity = '.5';
          chrome.runtime
            .sendMessage({
              type: 'download-format',
              // The page URL is what a manager needs to resolve a ciphered/protected format;
              // the direct URL is a hint for the ones that are already usable.
              page: pageMeta.page || location.href,
              title: pageMeta.title || document.title || null,
              source: pageMeta.source || null,
              url: f.url || null,
              format: {
                itag: f.itag ?? null, quality: f.quality ?? null,
                videoCodec: f.videoCodec ?? null, audioCodec: f.audioCodec ?? null,
                mime: f.mime ?? null, bitrate: f.bitrate ?? null,
                height: f.height ?? null, audioOnly: !!f.audioOnly,
              },
            })
            .then((r) => {
              row.style.opacity = '';
              if (!r || !r.ok) {
                const n = document.createElement('div');
                n.className = 'note';
                n.textContent = `reIDM could not take it: ${(r && r.error) || 'unknown error'}`;
                panel.appendChild(n);
              } else {
                close();
              }
            })
            .catch(() => { row.style.opacity = ''; });
        });
      }
      panel.appendChild(row);
    }

    if (anyCiphered) {
      const n = document.createElement('div');
      n.className = 'note';
      n.textContent =
        'Some formats need the player’s signature and are shown greyed out. Those need a resolver (yt-dlp) on the reIDM side.';
      panel.appendChild(n);
    }
  }

  // The page may build its player long after document_start.
  let timer = null;
  const schedule = () => {
    if (timer) return;
    timer = setTimeout(() => {
      timer = null;
      scan();
      queryFormats();
      refreshFromWorker();
    }, 700);
  };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', schedule, { once: true });
  } else {
    schedule();
  }
  new MutationObserver(schedule).observe(document.documentElement || document, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['src', 'href', 'type'],
  });
  window.addEventListener('load', schedule, { once: true });

  // SPA navigation: a new video means a new list, and the old panel is stale.
  let navHref = location.href;
  setInterval(() => {
    if (location.href !== navHref) {
      navHref = location.href;
      hookFormats = [];
      workerFormats = [];
      pageMeta = { source: null, title: null, page: null };
      lastSig = '';
      if (panel) { panel.textContent = ''; panel.hidden = true; }
      schedule();
    }
  }, 1500);
})();
