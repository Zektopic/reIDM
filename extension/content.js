// reIDM Integration — page-side scanner.
//
// Runs in every frame at document_start. It does two things:
//   * injects the MAIN-world hook (inject.js) that watches fetch/XHR, because the extension
//     world cannot see the page's own requests and `chrome.webRequest` cannot read bodies;
//   * reports media elements and media links it can see in the DOM.
//
// It never talks to reIDM itself. A content script's fetch carries the *page's* origin, so it
// would be treated as a cross-origin page request; the service worker does the talking.

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

  // Messages from the MAIN-world hook.
  window.addEventListener('message', (e) => {
    if (e.source !== window || !e.data || e.data.__reidm !== true) return;
    const items = (e.data.items || []).filter((i) => isHttp(i.url));
    if (items.length) chrome.runtime.sendMessage({ type: 'media', items }).catch(() => {});
  });

  // ── DOM scan ──────────────────────────────────────────────────────────
  const scan = () => {
    const items = [];
    const push = (url, kind) => {
      const u = absolute(url);
      if (u && isHttp(u)) items.push({ url: u, kind });
    };

    for (const el of document.querySelectorAll(MEDIA_SEL)) {
      const tag = el.tagName.toLowerCase();
      if (el.src) push(el.src, tag === 'source' ? mediaKind(el) : tag);
      const current = el.getAttribute && el.getAttribute('src');
      if (current) push(current, tag);
    }
    // Links that clearly point at a file worth downloading.
    for (const a of document.querySelectorAll('a[href]')) {
      const href = a.getAttribute('href') || '';
      if (LINK_EXT.test(href)) push(href, 'link');
    }
    if (items.length) chrome.runtime.sendMessage({ type: 'media', items }).catch(() => {});
  };

  function mediaKind(sourceEl) {
    const t = (sourceEl.getAttribute('type') || '').toLowerCase();
    if (t.includes('mpegurl') || t.includes('dash')) return 'stream';
    if (t.startsWith('video/')) return 'video';
    if (t.startsWith('audio/')) return 'audio';
    return 'media';
  }

  // The page may build its player long after document_start.
  let timer = null;
  const schedule = () => {
    if (timer) return;
    timer = setTimeout(() => {
      timer = null;
      scan();
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
})();
