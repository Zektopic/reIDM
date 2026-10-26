// reIDM Integration — MAIN-world hook.
//
// Runs in the *page's* JavaScript context, so it can see the requests and globals the page
// itself uses. `chrome.webRequest` already reports every media URL, but it cannot read response
// bodies, and the information a download manager actually needs to offer "1080p H.264" lives
// inside those bodies. So this file does two things:
//
//   * fetch(): report the request URL, and read manifest/player bodies without disturbing them.
//   * XMLHttpRequest: report the URL, and read the body when it is a manifest.
//
// The service worker registers it as a `document_start` content script in the page's world, and
// only while "Look for video and audio" is on; it then runs before any of the page's scripts.
// It reports by postMessage to the content script, which forwards to the service worker. It
// never blocks or modifies a request — the page must keep working exactly as before.
//
// Two body shapes matter for a video grabber:
//   * YouTube's player response (`streamingData.formats` / `.adaptiveFormats`) is where the
//     per-codec, per-resolution list comes from. YouTube is a single-page app: the
//     `ytInitialPlayerResponse` global describes the *first* video of the visit and is never
//     updated, while `/youtubei/v1/player` describes each later one (and sometimes one it
//     merely preloads). So every response is kept by its `videoDetails.videoId`, and only the
//     one for the video in the address bar is ever offered.
//   * HLS master playlists and DASH MPDs describe their variants in the body.

(() => {
  if (window.__reidmInjected) return;
  window.__reidmInjected = true;

  const REPORT = (items) => {
    if (!items.length) return;
    window.postMessage({ __reidm: true, items }, '*');
  };

  const MEDIA = /\.(m3u8|mpd|mp4|m4v|webm|mkv|mov|flv|ts|mp3|m4a|flac|ogg|opus|wav|aac)(\?|#|$)/i;
  const MANIFEST = /\.(m3u8|mpd)(\?|#|$)/i;
  const YT_PLAYER = /\/youtubei\/v1\/player|\/youtubei\/v1\/get_video_info/i;

  /** Manifest URL -> {kind, formats, at}: the variants parsed out of its body, kept so an
   *  on-demand query can still answer after the body is gone. Bounded, and emptied when the
   *  page moves to another address: on a single-page video site that is another video. */
  const seenManifests = new Map();
  const MAX_MANIFESTS = 20;
  /** YouTube video id -> its player response. */
  const players = new Map();
  const MAX_PLAYERS = 10;

  function remember(map, key, value, max) {
    map.delete(key);
    map.set(key, value);
    while (map.size > max) map.delete(map.keys().next().value);
  }

  // ── format normalisation ───────────────────────────────────────────────

  /** "avc1.640028,mp4a.40.2" -> { video: "H.264", audio: "AAC" } (best effort only). */
  function codecNames(codecs) {
    const c = String(codecs || '').toLowerCase();
    const video = c.includes('av01') ? 'AV1'
      : c.includes('vp9') || c.includes('vp09') ? 'VP9'
      : c.includes('avc1') || c.includes('h264') ? 'H.264'
      : c.includes('hev1') || c.includes('hvc1') ? 'H.265'
      : null;
    const audio = c.includes('opus') ? 'Opus'
      : c.includes('mp4a') ? 'AAC'
      : null;
    return { video, audio };
  }

  /** YouTube's `streamingData` -> the flat list a picker can render. */
  function formatsFromYouTube(data) {
    const sd = data && data.streamingData;
    if (!sd) return [];
    const out = [];
    const push = (f, adaptive) => {
      if (!f || !f.mimeType) return;
      const [mime, codecs] = String(f.mimeType).split(';');
      const names = codecNames(codecs);
      const hasVideo = /video\//.test(mime);
      const hasAudio = /audio\//.test(mime) || /audioTrack/.test(f.mimeType);
      out.push({
        itag: f.itag,
        mime: mime,
        codecs: (codecs || '').replace(/codecs="?|"?/g, '').trim() || null,
        videoCodec: names.video,
        audioCodec: names.audio,
        // `qualityLabel` is the friendly one ("1080p60"). Audio has none, and its raw `quality`
        // ("tiny") means nothing to a person: its bitrate says more.
        quality: f.qualityLabel || (hasAudio && !hasVideo ? (f.bitrate ? `${Math.round(f.bitrate / 1000)}k` : 'audio') : f.quality || null),
        // YouTube lists some audio twice: as published, and loudness-normalised ("stable volume").
        drc: !!f.isDrc,
        bitrate: f.bitrate || null,
        size: f.contentLength ? Number(f.contentLength) : null,
        audioOnly: adaptive && hasAudio && !hasVideo,
        hasAudio,
        height: f.height || null,
        // A ciphered URL needs the player's signature logic, which an extension must not
        // reimplement. The picker still lists the format and says so.
        url: f.url || null,
        fps: f.fps || null,
      });
    };
    for (const f of sd.formats || []) push(f, false);
    for (const f of sd.adaptiveFormats || []) push(f, true);
    return out;
  }

  /** HLS master playlist -> one entry per `#EXT-X-STREAM-INF`. */
  function formatsFromHLS(body, baseUrl) {
    const out = [];
    const lines = body.split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i].trim();
      if (!line.startsWith('#EXT-X-STREAM-INF')) continue;
      const res = /RESOLUTION=(\d+)x(\d+)/i.exec(line);
      const bw = /BANDWIDTH=(\d+)/i.exec(line);
      const codecs = /CODECS="([^"]+)"/i.exec(line);
      const name = /NAME="([^"]+)"/i.exec(line);
      // The URI is on the following non-comment line.
      let uri = null;
      for (let k = i + 1; k < lines.length; k++) {
        const t = lines[k].trim();
        if (t && !t.startsWith('#')) { uri = t; break; }
      }
      const names = codecNames(codecs && codecs[1]);
      out.push({
        quality: name ? name[1] : res ? `${res[2]}p` : null,
        width: res ? Number(res[1]) : null,
        height: res ? Number(res[2]) : null,
        bitrate: bw ? Number(bw[1]) : null,
        videoCodec: names.video,
        audioCodec: names.audio,
        codecs: codecs ? codecs[1] : null,
        url: uri ? absolute(uri, baseUrl) : null,
        kind: 'hls-variant',
      });
    }
    return out;
  }

  /** DASH MPD -> one entry per `<Representation>`. */
  function formatsFromDASH(body, baseUrl) {
    const out = [];
    // A full XML parse is not needed and DOMParser on a hostile body is avoidable work;
    // the attributes a picker needs are flat and regular.
    const re = /<Representation\b([^>]*)>([\s\S]*?)<\/Representation>/gi;
    let m;
    while ((m = re.exec(body))) {
      const attrs = m[1];
      const inner = m[2];
      const attr = (n) => {
        const r = new RegExp(`${n}="([^"]*)"`, 'i').exec(attrs);
        return r ? r[1] : null;
      };
      const base = /<BaseURL>([^<]+)<\/BaseURL>/i.exec(inner);
      const names = codecNames(attr('codecs'));
      out.push({
        quality: attr('height') ? `${attr('height')}p` : attr('id'),
        width: attr('width') ? Number(attr('width')) : null,
        height: attr('height') ? Number(attr('height')) : null,
        bitrate: attr('bandwidth') ? Number(attr('bandwidth')) : null,
        mime: attr('mimeType'),
        codecs: attr('codecs'),
        videoCodec: names.video,
        audioCodec: names.audio,
        url: base ? absolute(base[1], baseUrl) : baseUrl.split('#')[0],
        kind: 'dash-representation',
      });
    }
    return out;
  }

  function absolute(u, base) {
    try {
      return new URL(u, base).href;
    } catch {
      return null;
    }
  }

  /** Work out whether a body is a manifest, and turn it into formats. */
  function formatsFromBody(body, url) {
    if (!body || body.length < 8) return null;
    const head = body.slice(0, 400);
    if (head.includes('#EXTM3U')) return { kind: 'hls', formats: formatsFromHLS(body, url) };
    if (head.includes('<MPD') || head.includes('<Representation')) {
      return { kind: 'dash', formats: formatsFromDASH(body, url) };
    }
    return null;
  }

  function noteManifestBody(url, body) {
    const f = formatsFromBody(body, url);
    if (!f || !f.formats.length) return;
    remember(seenManifests, url, { kind: f.kind, formats: f.formats, at: Date.now() }, MAX_MANIFESTS);
    REPORT([{ url, kind: 'formats', source: f.kind, formats: f.formats, title: document.title || null, page: location.href }]);
  }

  // ── YouTube ────────────────────────────────────────────────────────────

  /** The id of the video in the address bar: `watch?v=`, `/shorts/`, `/live/`, `/embed/`. */
  function currentVideoId() {
    try {
      const u = new URL(location.href);
      const v = u.searchParams.get('v');
      if (v) return v;
      const m = /^\/(?:shorts|live|embed)\/([\w-]{6,})/.exec(u.pathname);
      return m ? m[1] : null;
    } catch {
      return null;
    }
  }

  const isYouTubeHost = () => /(^|\.)(youtube\.com|youtube-nocookie\.com)$/i.test(location.hostname);
  const videoIdOf = (r) => (r && r.videoDetails && r.videoDetails.videoId) || null;

  function notePlayer(r) {
    const id = videoIdOf(r);
    if (!id || !r.streamingData) return;
    remember(players, id, r, MAX_PLAYERS);
    if (id === currentVideoId()) REPORT([{ url: location.href, kind: 'formats', ...youTubeAnswer(id, r) }]);
  }

  function youTubeAnswer(id, r) {
    return {
      source: 'youtube',
      title: (r && r.videoDetails && r.videoDetails.title) || document.title.replace(/ - YouTube$/, '') || null,
      formats: r ? formatsFromYouTube(r) : [],
      page: `https://www.youtube.com/watch?v=${encodeURIComponent(id)}`,
    };
  }

  /** The formats of the video being watched, or — when no response for it has been seen — the
   *  page alone, for which reIDM's yt-dlp picks the best quality. Never another video's. */
  function currentYouTube() {
    const id = currentVideoId();
    if (!id) return null;
    const global = window.ytInitialPlayerResponse;
    if (!isYouTubeHost() && !players.size && !videoIdOf(global)) return null;
    let live = null;
    try {
      const p = document.getElementById('movie_player');
      live = p && typeof p.getPlayerResponse === 'function' ? p.getPlayerResponse() : null;
    } catch {
      /* the player is not ready */
    }
    for (const r of [live, players.get(id), global]) {
      if (videoIdOf(r) === id && r.streamingData) return youTubeAnswer(id, r);
    }
    return youTubeAnswer(id, null);
  }

  // ── the page's own fetches ─────────────────────────────────────────────

  const origFetch = window.fetch;
  if (typeof origFetch === 'function') {
    window.fetch = function (input, init) {
      let url = null;
      try {
        // A relative address is resolved as the browser will resolve it; a Request or URL
        // object already carries an absolute one.
        url = new URL(typeof input === 'string' ? input : input && input.url ? input.url : String(input), document.baseURI).href;
      } catch {
        /* ignore */
      }
      const p = origFetch.apply(this, arguments);
      try {
        if (url && MANIFEST.test(url)) {
          REPORT([{ url, kind: 'stream' }]);
          // Read a copy for the variant list; never disturb what the caller receives.
          p.then((r) => r.clone().text().then((body) => noteManifestBody(url, body))).catch(() => {});
        } else if (url && YT_PLAYER.test(url)) {
          p.then((r) => r.clone().json().then(notePlayer)).catch(() => {});
        } else if (url && MEDIA.test(url)) {
          REPORT([{ url, kind: 'media' }]);
        }
      } catch {
        /* a streamed or already-consumed body */
      }
      return p;
    };
  }

  const OrigXHR = window.XMLHttpRequest;
  if (OrigXHR && OrigXHR.prototype && OrigXHR.prototype.open) {
    const open = OrigXHR.prototype.open;
    const send = OrigXHR.prototype.send;
    OrigXHR.prototype.open = function (method, url) {
      try {
        this.__reidmUrl = url ? new URL(url, document.baseURI).href : null;
      } catch {
        this.__reidmUrl = null;
      }
      return open.apply(this, arguments);
    };
    OrigXHR.prototype.send = function () {
      const url = this.__reidmUrl;
      if (url) {
        this.addEventListener('load', () => {
          try {
            const ct = this.getResponseHeader('content-type') || '';
            const isText = /mpegurl|dash\+xml|xml|json|text/i.test(ct);
            const text = isText && (this.responseType === '' || this.responseType === 'text') ? this.responseText : null;
            if (MANIFEST.test(url) || /mpegurl|dash\+xml/i.test(ct)) {
              REPORT([{ url, kind: 'stream' }]);
              if (text) noteManifestBody(url, text);
            } else if (YT_PLAYER.test(url) && text) {
              notePlayer(JSON.parse(text));
            } else if (MEDIA.test(url)) {
              REPORT([{ url, kind: 'media' }]);
            }
          } catch {
            /* cross-origin or unreadable response */
          }
        });
      }
      return send.apply(this, arguments);
    };
  }

  // ── on-demand: the formats for whatever is playing right now ───────────

  /** The first video on the page. `deep` also looks inside open shadow roots — a web-component
   *  player keeps its <video> where document.querySelector cannot see it — but that walk reads
   *  every element, so only the picker's on-demand query asks for it. */
  function findVideo(deep = false) {
    const v = document.querySelector('video');
    if (v || !deep) return v;
    return findVideoInShadows(document, 0);
  }

  function findVideoInShadows(root, depth) {
    if (depth >= 8) return null;
    for (const el of root.querySelectorAll('*')) {
      if (!el.shadowRoot) continue;
      const v = el.shadowRoot.querySelector('video');
      if (v) return v;
      const found = findVideoInShadows(el.shadowRoot, depth + 1);
      if (found) return found;
    }
    return null;
  }

  /** Everything the picker can offer for the current page. */
  function currentFormats(deep = false) {
    // 1. YouTube: the player response for this video holds the full per-codec list.
    const yt = currentYouTube();
    if (yt) return yt;

    const out = { source: null, title: null, formats: [], page: location.href };

    // 2. The manifest the page fetched most recently. Answering with only the <video> element
    //    would silently downgrade a four-variant stream to one progressive file.
    let best = null;
    for (const [url, m] of seenManifests) {
      if (m.formats.length && (!best || m.at >= best.at)) best = { url, ...m };
    }
    if (best) {
      out.source = best.kind;
      out.title = document.title || null;
      out.formats = best.formats.slice();
    }

    // 3. A <video> element with a plain source. Kept alongside the manifest rather than
    //    instead of it: an unlabelled progressive file is still worth offering.
    const v = findVideo(deep);
    if (v) {
      const src = v.currentSrc || v.src;
      if (src && /^https?:/i.test(src)) {
        out.source = out.source || 'video-element';
        if (!out.formats.some((f) => f.url === src)) {
          out.formats.push({ url: src, quality: v.videoHeight ? `${v.videoHeight}p` : null, kind: 'progressive' });
        }
      }
      for (const s of v.querySelectorAll('source[src]')) {
        const u = absolute(s.getAttribute('src'), document.baseURI);
        if (u && /^https?:/i.test(u) && !out.formats.some((f) => f.url === u)) {
          out.formats.push({ url: u, quality: null, kind: s.getAttribute('type') || 'source' });
        }
      }
      if (out.formats.length) out.title = out.title || document.title || null;
    }
    return out;
  }

  window.addEventListener('message', (e) => {
    if (e.source !== window || !e.data || e.data.__reidmQuery !== true) return;
    let payload;
    try {
      payload = currentFormats(true);
    } catch (err) {
      payload = { source: null, formats: [], error: String(err && err.message) };
    }
    window.postMessage({ __reidm: true, kind: 'formats-answer', id: e.data.id, payload }, '*');
  });

  // A quick unsolicited report, so the button can appear without the user clicking first.
  const announce = () => {
    try {
      const f = currentFormats();
      if (f.formats.length || f.source === 'youtube') REPORT([{ url: location.href, kind: 'formats', ...f }]);
    } catch {
      /* ignore */
    }
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', announce, { once: true });
  } else {
    setTimeout(announce, 0);
  }

  // A single-page site moves to another video without a load event. The Navigation API says
  // when (Chrome 102+); polling the address is only the fallback.
  let lastHref = location.href;
  const onNavigate = () => {
    if (location.href === lastHref) return;
    lastHref = location.href;
    seenManifests.clear();
    announce();
  };
  if (window.navigation && typeof window.navigation.addEventListener === 'function') {
    window.navigation.addEventListener('navigatesuccess', onNavigate);
  } else {
    setInterval(onNavigate, 2000);
  }
})();
