// reIDM Integration — page-side scanner and picker.
//
// Runs in every frame at document_start, so it has to cost next to nothing on pages with no
// video. It does three things:
//   * takes download links on click, before the browser requests them (always, unless off);
//   * while "Look for video and audio" is on: reports media elements and media links it sees in
//     the DOM, and relays what the MAIN-world hook (inject.js, which the service worker
//     registers) finds in the page's own requests;
//   * shows a "Download with reIDM" button on a playing video, which opens a picker listing the
//     codecs and resolutions the player actually offers.
//
// It never talks to reIDM itself. A content script's fetch carries the *page's* origin, so it
// would be treated as a cross-origin page request; the service worker does the talking.
//
// The button and the panel live in a closed shadow root. On a site like YouTube the page's own
// CSS is aggressive and a plain element inherits enough of it to render wrong.

(() => {
  if (window.__reidmContentLoaded) return;
  window.__reidmContentLoaded = true;

  const D = globalThis.REIDM_DEFAULTS;
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

  // ── download links, taken before the browser asks for them ────────────
  //
  // A plain click on a link whose address names a file type on the capture list (or whose
  // `download` attribute does) is stopped here and handed to reIDM, so the browser never even
  // requests the file. Everything else — and anything reIDM declines — goes on to the browser,
  // where the service worker's download hook still has its say.
  //
  // The listener sits on `window` in the bubbling phase, so the page's own handlers run first:
  // a link the page handles itself (an SPA route, a script that builds the real URL) arrives
  // here with `defaultPrevented` set and is left alone.

  const KEYS = ['capture', 'interceptClicks', 'fileTypes', 'skipHosts', 'captureIncognito', 'sniffMedia'];
  const cap = Object.fromEntries(KEYS.map((k) => [k, D[k]]));
  let reidmUp = false;
  chrome.storage.sync
    .get(Object.fromEntries(KEYS.map((k) => [k, D[k]])))
    .then((s) => {
      Object.assign(cap, s);
      if (cap.sniffMedia) startMedia();
    })
    .catch(() => {});
  chrome.storage.local
    .get('reidm-status')
    .then((o) => (reidmUp = !!(o['reidm-status'] && o['reidm-status'].up)))
    .catch(() => {});
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === 'sync') {
      for (const [k, v] of Object.entries(changes)) if (KEYS.includes(k)) cap[k] = v.newValue ?? D[k];
      if ('sniffMedia' in changes) (cap.sniffMedia ? startMedia : stopMedia)();
    } else if (area === 'local' && changes['reidm-status']) {
      const s = changes['reidm-status'].newValue;
      reidmUp = !!(s && s.up);
    }
  });

  const extOf = (s) => {
    const m = /\.([a-z0-9]{1,8})$/i.exec(String(s || '').split(/[?#]/)[0]);
    return m ? m[1].toUpperCase() : '';
  };
  const listed = (s) => {
    const ext = extOf(s);
    if (!ext) return false;
    return String(cap.fileTypes || D.fileTypes).toUpperCase().split(/[\s,;|]+/).some((t) => t && (t.endsWith('*') ? ext.startsWith(t.slice(0, -1)) : t === ext));
  };
  const skippedHost = (href) => {
    let h = '';
    try {
      h = new URL(href).hostname.toLowerCase();
    } catch {
      return true;
    }
    return (cap.skipHosts || []).some((s) => {
      const e = String(s).trim().toLowerCase().replace(/^\*\./, '');
      return e && (h === e || h.endsWith(`.${e}`));
    });
  };
  const linkOf = (ev) => {
    for (const el of ev.composedPath ? ev.composedPath() : [ev.target]) {
      if (el && (el.tagName === 'A' || el.tagName === 'AREA') && el.href) return el;
    }
    return null;
  };
  /** The file name a link promises: its `download` attribute, else the last path segment. */
  const promisedName = (a) => {
    const d = a.getAttribute('download');
    if (d) return d;
    try {
      return decodeURIComponent(new URL(a.href).pathname.split('/').filter(Boolean).pop() || '');
    } catch {
      return '';
    }
  };
  /** Whether reIDM may act in this window at all. */
  const allowedHere = () => cap.capture && (!chrome.extension.inIncognitoContext || cap.captureIncognito);

  /** reIDM declined, or could not be reached: do what the click would have done. The click is
   *  replayed on the link rather than the address opened from the worker, because the replay
   *  carries the page as referrer, which download sites often check. Our own replay is
   *  untrusted, so the listener below lets it through. */
  function handBack(a, href) {
    chrome.runtime
      .sendMessage({ type: 'bypass', url: href })
      .catch(() => {})
      .finally(() => {
        const target = (a.getAttribute('target') || '').toLowerCase();
        if (target === '_blank' && !a.hasAttribute('download')) {
          // The click's user activation may have lapsed while reIDM was asked, and a script
          // `window.open` would then be blocked as a pop-up; the worker can open the tab.
          chrome.runtime.sendMessage({ type: 'open-tab', url: href }).catch(() => {});
        } else {
          a.click();
        }
      });
  }

  window.addEventListener(
    'click',
    (ev) => {
      if (!ev.isTrusted || ev.button !== 0) return;
      const a = linkOf(ev);
      if (!a) return;
      const href = a.href;
      // A magnet link is a torrent for reIDM — the browser has nothing to do with one but hand
      // it to some other program. Alt+click still leaves it to that program.
      if (/^magnet:/i.test(href)) {
        if (ev.altKey || ev.defaultPrevented || ev.ctrlKey || ev.metaKey || ev.shiftKey || !allowedHere()) return;
        ev.preventDefault();
        chrome.runtime
          .sendMessage({ type: 'capture-link', url: href })
          .then((r) => {
            if (!r || !r.ok) handBack(a, href);
          })
          .catch(() => handBack(a, href));
        return;
      }
      if (!isHttp(href)) return;
      // IDM's convention: Alt+click means "let the browser have this one".
      if (ev.altKey) {
        chrome.runtime.sendMessage({ type: 'bypass', url: href }).catch(() => {});
        return;
      }
      if (ev.defaultPrevented || ev.ctrlKey || ev.metaKey || ev.shiftKey) return;
      if (!allowedHere() || !cap.interceptClicks || !reidmUp || skippedHost(href)) return;
      const name = promisedName(a);
      if (!listed(href) && !listed(name)) return;

      ev.preventDefault();
      chrome.runtime
        .sendMessage({ type: 'capture-link', url: href, filename: a.getAttribute('download') || null })
        .then((r) => {
          if (!r || !r.ok) handBack(a, href);
        })
        .catch(() => handBack(a, href));
    },
    false,
  );

  // ── media, only while "Look for video and audio" is on ────────────────

  // Two independent sources feed the picker, and neither may clobber the other:
  //   * the page hook (inject.js) sees player responses and manifests the page fetched itself;
  //   * the worker parses manifests it saw through webRequest, which catches one the hook
  //     could not see (a player in a worker, a frame the hook does not run in).
  // They are kept apart and rendered as a union, because the hook's answer for a page is
  // often just the one <video> source, and letting it assign over the worker's list silently
  // downgrades a four-variant stream to a single progressive file.
  let hookFormats = [];
  let workerFormats = [];
  let pageMeta = { source: null, title: null, page: null };
  let mediaOn = false;
  let observer = null;
  /** Addresses already reported, so a busy page doesn't send the same ones over and over. */
  const sent = new Set();

  const humanBytes = (n) => {
    if (!n || n <= 0) return null;
    const u = ['B', 'KB', 'MB', 'GB'];
    let v = n, i = 0;
    while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
    return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${u[i]}`;
  };

  /** Messages from the MAIN-world hook. */
  function onHookMessage(e) {
    if (e.source !== window || !e.data || e.data.__reidm !== true) return;

    // The answer to a direct query for the current page's formats.
    if (e.data.kind === 'formats-answer') {
      applyHook(e.data.payload || {});
      return;
    }

    const items = (Array.isArray(e.data.items) ? e.data.items : []).filter((i) => i && isHttp(i.url));
    for (const it of items) if (it.kind === 'formats') applyHook(it);
    report(items.filter((i) => i.kind !== 'formats').map((i) => ({ url: i.url, kind: i.kind })));
  }

  function report(items) {
    const fresh = items.filter((i) => !sent.has(i.url));
    if (!fresh.length) return;
    for (const i of fresh) sent.add(i.url);
    if (sent.size > 2000) sent.clear();
    chrome.runtime.sendMessage({ type: 'media', items: fresh }).catch(() => {});
  }

  /** The page hook's view of this page. Replaces the previous hook view, not the worker's. On
   *  YouTube an empty list still counts: it means "this video, best quality". */
  function applyHook(p) {
    if (!p || !(p.source === 'youtube' || (p.formats && p.formats.length))) return;
    hookFormats = Array.isArray(p.formats) ? p.formats : [];
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

  /** Hook formats first (they carry the friendlier labels), then any the worker alone has. On
   *  YouTube only the hook's: the worker's would be the player's own traffic. */
  function combined() {
    const out = [];
    const seen = new Set();
    for (const f of pageMeta.source === 'youtube' ? hookFormats : [...hookFormats, ...workerFormats]) {
      const k = f.url || `${f.itag}|${f.quality}|${f.bitrate}|${f.codecs}`;
      if (seen.has(k)) continue;
      seen.add(k);
      out.push(f);
    }
    return out;
  }

  let lastSig = '';

  /** Publish the union. The signature guard keeps the DOM from being rebuilt on every report. */
  function refresh() {
    const fs = combined();
    if (!fs.length && pageMeta.source !== 'youtube') return;
    const sig = `${pageMeta.source}|${pageMeta.page}|${fs.length}|${fs.map((f) => f.url || f.itag || f.quality).join(',')}`;
    if (sig === lastSig) return;
    lastSig = sig;
    ensureUi();
    render(fs);
  }

  // ── DOM scan: only what changed ───────────────────────────────────────

  function scanRoot(root, items) {
    const push = (url, kind) => {
      const u = absolute(url);
      if (u && isHttp(u)) items.push({ url: u, kind });
    };
    const media = (el) => {
      const tag = el.tagName.toLowerCase();
      // A blob: URL means MSE; there is nothing useful in it and the hook has the real ones.
      if (el.src && !/^blob:/i.test(el.src)) push(el.src, tag === 'source' ? mediaKind(el) : tag);
      if (tag === 'video') noteVideo(el);
    };
    const link = (a) => {
      const href = a.getAttribute('href') || '';
      if (LINK_EXT.test(href)) push(href, 'link');
    };
    if (root.nodeType !== 1 && root !== document) return;
    if (root.matches && root.matches(MEDIA_SEL)) media(root);
    else if (root.matches && root.matches('a[href]')) link(root);
    if (!root.firstElementChild && root !== document) return;
    for (const el of root.querySelectorAll(MEDIA_SEL)) media(el);
    for (const a of root.querySelectorAll('a[href]')) link(a);
  }

  /** Nodes added or changed since the last scan; `document` means everything. */
  let dirty = new Set();
  let scheduled = false;
  const idle = window.requestIdleCallback || ((fn) => setTimeout(fn, 200));
  function schedule(root) {
    dirty.add(root);
    if (dirty.size > 500) dirty = new Set([document]);
    if (scheduled) return;
    scheduled = true;
    // A busy page (a feed, a chat) mutates all the time: look at most every two seconds, and
    // only when the browser is otherwise idle.
    setTimeout(() => idle(() => {
      scheduled = false;
      if (!mediaOn) return;
      const roots = dirty.has(document) ? [document] : [...dirty];
      dirty = new Set();
      const items = [];
      for (const r of roots) if (r === document || r.isConnected) scanRoot(r, items);
      report(items);
    }, { timeout: 4000 }), 2000);
  }

  function onMutations(records) {
    for (const r of records) {
      if (r.type === 'attributes') {
        if (r.target.matches && r.target.matches('video, audio, source, a[href]')) schedule(r.target);
        continue;
      }
      for (const n of r.addedNodes) if (n.nodeType === 1) schedule(n);
    }
  }

  function mediaKind(sourceEl) {
    const t = (sourceEl.getAttribute('type') || '').toLowerCase();
    if (t.includes('mpegurl') || t.includes('dash')) return 'stream';
    if (t.startsWith('video/')) return 'video';
    if (t.startsWith('audio/')) return 'audio';
    return 'media';
  }

  function startMedia() {
    if (mediaOn) return;
    mediaOn = true;
    window.addEventListener('message', onHookMessage);
    document.addEventListener('playing', onPlaying, true);
    const begin = () => {
      if (!mediaOn) return;
      schedule(document);
      observer = new MutationObserver(onMutations);
      observer.observe(document.documentElement || document, { childList: true, subtree: true, attributes: true, attributeFilter: ['src', 'href'] });
    };
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', begin, { once: true });
    else begin();
    if (window.navigation && typeof window.navigation.addEventListener === 'function') window.navigation.addEventListener('navigatesuccess', onNavigate);
    else navPoll = setInterval(onNavigate, 1500);
  }

  function stopMedia() {
    if (!mediaOn) return;
    mediaOn = false;
    window.removeEventListener('message', onHookMessage);
    document.removeEventListener('playing', onPlaying, true);
    if (observer) observer.disconnect();
    observer = null;
    if (window.navigation && typeof window.navigation.removeEventListener === 'function') window.navigation.removeEventListener('navigatesuccess', onNavigate);
    clearInterval(navPoll);
    removeUi();
  }

  // SPA navigation: a new video means a new list, and the old panel is stale.
  let navHref = location.href;
  let navPoll = null;
  function onNavigate() {
    if (location.href === navHref) return;
    navHref = location.href;
    hookFormats = [];
    workerFormats = [];
    pageMeta = { source: null, title: null, page: null };
    lastSig = '';
    sent.clear();
    if (panel) { panel.textContent = ''; panel.hidden = true; }
    schedule(document);
  }

  // ── the button and the picker ─────────────────────────────────────────
  //
  // Anchored to the video's own box rather than to a fixed corner, so it sits where the user
  // is already looking. Re-positioned when something says the box may have moved — a scroll, a
  // resize, the video starting, the pointer arriving on it — never on a timer: reading a box
  // forces layout, and a timer would do that every second in every frame with a video.

  let host = null, root = null, btn = null, panel = null, videoEl = null, sizeWatch = null;

  const MIN_BOX = { w: 120, h: 80 };
  /** Small frames are adverts and previews; they get no button. */
  const frameTooSmall = () => window !== window.top && (window.innerWidth < 200 || window.innerHeight < 150);

  function noteVideo(v) {
    if (frameTooSmall()) return;
    if (v === videoEl) return;
    const r = v.getBoundingClientRect();
    if (r.width < MIN_BOX.w || r.height < MIN_BOX.h) return;
    watchVideo(v);
    ensureUi();
  }

  function onPlaying(e) {
    if (e.target && e.target.tagName === 'VIDEO') {
      noteVideo(e.target);
      position();
    }
  }

  function watchVideo(v) {
    if (videoEl) videoEl.removeEventListener('mouseenter', position);
    videoEl = v;
    v.addEventListener('mouseenter', position, { passive: true });
    if (sizeWatch) {
      sizeWatch.disconnect();
      sizeWatch.observe(v);
    }
  }

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

  const BUTTON_TEXT = 'Download with reIDM';

  function ensureUi() {
    if (host || !document.documentElement || frameTooSmall()) return;
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
      const dot = document.createElement('span');
      dot.className = 'dot';
      const caption = document.createElement('b');
      caption.textContent = BUTTON_TEXT;
      btn.append(dot, caption);
      btn.addEventListener('click', (ev) => {
        ev.preventDefault();
        ev.stopPropagation();
        panel.hidden = !panel.hidden;
        if (!panel.hidden) {
          // The lists can change while the page plays; ask afresh when the user looks.
          queryFormats();
          refreshFromWorker();
        }
      });

      panel = document.createElement('div');
      panel.className = 'panel';
      panel.hidden = true;

      wrap.appendChild(btn);
      wrap.appendChild(panel);
      root.append(style, wrap);
      document.documentElement.appendChild(host);

      host.__wrap = wrap;
      window.addEventListener('scroll', position, { passive: true, capture: true });
      window.addEventListener('resize', position, { passive: true });
      sizeWatch = new ResizeObserver(() => position());
      if (videoEl) sizeWatch.observe(videoEl);
      if (document.body) sizeWatch.observe(document.body);
      position();
    } catch {
      /* a page that forbids decoration should still work without us */
    }
  }

  function removeUi() {
    if (!host) return;
    window.removeEventListener('scroll', position, { capture: true });
    window.removeEventListener('resize', position);
    if (sizeWatch) sizeWatch.disconnect();
    if (videoEl) videoEl.removeEventListener('mouseenter', position);
    if (host.parentNode) host.parentNode.removeChild(host);
    host = root = btn = panel = sizeWatch = videoEl = null;
    lastSig = '';
  }

  /** Keep the button in the video's top-right corner. */
  let positionQueued = false;
  function position() {
    if (positionQueued || !host) return;
    positionQueued = true;
    requestAnimationFrame(() => {
      positionQueued = false;
      if (!host || !host.__wrap) return;
      if (!videoEl || !videoEl.isConnected) {
        const v = document.querySelector('video');
        if (v) watchVideo(v);
      }
      if (!videoEl || !videoEl.isConnected) {
        host.__wrap.style.display = 'none';
        return;
      }
      const r = videoEl.getBoundingClientRect();
      if (r.width < MIN_BOX.w || r.height < MIN_BOX.h || r.bottom < 0 || r.top > window.innerHeight) {
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
    });
  }

  /** After a download is handed over: fold the panel and say so on the button for a moment.
   *  The button stays, ready for another quality. */
  function confirmSent(text) {
    if (!panel || !btn) return;
    panel.hidden = true;
    const b = btn.querySelector('b');
    b.textContent = text;
    setTimeout(() => {
      if (b.isConnected) b.textContent = BUTTON_TEXT;
    }, 2500);
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

  function choose(row, f) {
    row.style.opacity = '.5';
    chrome.runtime
      .sendMessage({
        type: 'download-format',
        // The page URL is what a manager needs to resolve a ciphered/protected format;
        // the direct URL is a hint for the ones that are already usable.
        page: pageMeta.page || location.href,
        title: pageMeta.title || document.title || null,
        source: pageMeta.source || null,
        url: (f && f.url) || null,
        format: f
          ? {
            itag: f.itag ?? null, quality: f.quality ?? null, kind: f.kind ?? null,
            videoCodec: f.videoCodec ?? null, audioCodec: f.audioCodec ?? null,
            mime: f.mime ?? null, bitrate: f.bitrate ?? null,
            height: f.height ?? null, audioOnly: !!f.audioOnly, hasAudio: !!f.hasAudio, drc: !!f.drc,
            // The whole file's size only when the format already has its sound.
            size: f.audioOnly || f.hasAudio ? f.size ?? null : null,
          }
          : null,
      })
      .then((r) => {
        row.style.opacity = '';
        if (r && r.cancelled) return;
        if (!r || !r.ok) {
          const n = document.createElement('div');
          n.className = 'note';
          n.textContent = `reIDM could not take it: ${(r && r.error) || 'unknown error'}`;
          panel.appendChild(n);
        } else {
          confirmSent('Sent to reIDM ✓');
        }
      })
      .catch(() => { row.style.opacity = ''; });
  }

  function render(fs) {
    if (!panel) return;

    panel.textContent = '';
    const hd = document.createElement('div');
    hd.className = 'hd';
    hd.textContent = pageMeta.title
      ? `${pageMeta.source || 'video'} — ${pageMeta.title}`
      : pageMeta.source || 'video';
    panel.appendChild(hd);

    // On YouTube every format goes through reIDM's yt-dlp, which finds its own address: the
    // page's are scrambled (`signatureCipher`) or missing. Elsewhere a format needs an address.
    const viaReidm = pageMeta.source === 'youtube';
    const anyUnusable = !viaReidm && fs.some((f) => !f.url);

    const rowFor = (label, metaText, sizeText, f, unusable) => {
      const row = document.createElement('div');
      row.className = unusable ? 'row no' : 'row';
      const q = document.createElement('span');
      q.className = 'q';
      q.textContent = label;
      const meta = document.createElement('span');
      meta.className = 'meta';
      meta.textContent = metaText;
      const sz = document.createElement('span');
      sz.className = 'sz';
      sz.textContent = sizeText || '';
      row.append(q, meta, sz);
      if (f && f.audioOnly) {
        const t = document.createElement('span');
        t.className = 'tag';
        t.textContent = 'audio';
        row.appendChild(t);
      }
      if (!unusable) {
        row.addEventListener('click', (ev) => {
          ev.preventDefault();
          ev.stopPropagation();
          choose(row, f);
        });
      }
      panel.appendChild(row);
    };

    // yt-dlp's own choice of the best, which needs no format list at all — and is all there is
    // when the page has not yet shown the player response for this video.
    if (viaReidm) rowFor('Best', 'best picture and sound, joined by reIDM', '', null, false);

    for (const f of sortFormats(fs)) {
      const bits = [];
      if (f.videoCodec) bits.push(f.videoCodec);
      if (f.audioCodec) bits.push(f.audioCodec);
      if (!bits.length && f.codecs) bits.push(f.codecs);
      if (f.drc) bits.push('stable volume');
      if (f.fps) bits.push(`${f.fps}fps`);
      if (f.bitrate) bits.push(`${Math.round(f.bitrate / 1000)} kbps`);
      if (f.mime) bits.push(f.mime.replace(/^video\/|^audio\//, ''));
      else if (f.kind) bits.push(String(f.kind).replace(/-/g, ' '));
      rowFor(f.quality || (f.audioOnly ? 'audio' : '?'), bits.join(' · ') || '—', humanBytes(f.size), f, !f.url && !viaReidm);
    }

    if (anyUnusable) {
      const n = document.createElement('div');
      n.className = 'note';
      n.textContent = 'Greyed-out formats have no address the page gives away.';
      panel.appendChild(n);
    }
    if (viaReidm) {
      const n = document.createElement('div');
      n.className = 'note';
      n.textContent = 'reIDM fetches YouTube videos with yt-dlp; the first one takes a moment while it gets it.';
      panel.appendChild(n);
    }
  }
})();
