// reIDM Integration — MAIN-world hook.
//
// Runs in the *page's* JavaScript context, so it can see the requests and globals the page
// itself uses. `chrome.webRequest` already reports every media URL, but it cannot read response
// bodies, and the information a download manager actually needs to offer "1080p H.264" lives
// inside those bodies. So this file does three things:
//
//   * fetch(): report the request URL, and read manifest/player bodies without disturbing them.
//   * XMLHttpRequest: report the URL, and read the body when it is a manifest.
//   * MediaSource.addSourceBuffer: a player using MSE usually means a stream is playing.
//
// It reports by postMessage to the content script, which forwards to the service worker. It
// never blocks or modifies a request — the page must keep working exactly as before.
//
// Two body shapes matter for a video grabber:
//   * YouTube's player response (`streamingData.formats` / `.adaptiveFormats`) is where the
//     per-codec, per-resolution list comes from.  Both the `ytInitialPlayerResponse` global and
//     `/youtubei/v1/player` carry it; SPA navigation updates the API, not the global, so both
//     are read.
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
  const LIKELY_MANIFEST = /(?:^|\/)(manifest|playlist|master|index)\.(m3u8|mpd)/i;
  const YT_PLAYER = /\/youtubei\/v1\/player|\/youtubei\/v1\/get_video_info/i;

  /** Manifest URL -> the variants parsed out of its body. Kept so an on-demand query can
   *  still answer with the variants after the body itself is long gone. */
  const seenManifests = new Map();

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
        // `qualityLabel` is the friendly one ("1080p60"); `quality` is the raw tag.
        quality: f.qualityLabel || f.quality || (hasAudio && !hasVideo ? 'audio' : null),
        bitrate: f.bitrate || null,
        size: f.contentLength ? Number(f.contentLength) : null,
        audioOnly: adaptive && hasAudio && !hasVideo,
        // A ciphered URL needs the player's signature logic, which an extension must not
        // reimplement. The picker still lists the format and says so.
        url: f.url || null,
        ciphered: !f.url && !!(f.signatureCipher || f.cipher),
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

  // ── the page's own fetches ─────────────────────────────────────────────

  const origFetch = window.fetch;
  if (typeof origFetch === 'function') {
    window.fetch = function (input, init) {
      let url = null;
      try {
        url = typeof input === 'string' ? input : input && input.url ? input.url : String(input);
      } catch {
        /* ignore */
      }
      const p = origFetch.apply(this, arguments);
      try {
        if (url && (MANIFEST.test(url) || LIKELY_MANIFEST.test(url))) {
          REPORT([{ url, kind: 'stream' }]);
          // Read a copy for the variant list; never disturb what the caller receives.
          p.then((r) => {
            r.clone().text().then((body) => {
              const f = formatsFromBody(body, url);
              if (f && f.formats.length) {
                const live = f.kind === 'hls' && !/#EXT-X-ENDLIST/.test(body);
                seenManifests.set(url, { kind: f.kind, formats: f.formats, live });
                REPORT([{
                  url,
                  kind: 'formats',
                  source: f.kind,
                  live,
                  formats: f.formats,
                }]);
              }
            }).catch(() => {});
          }).catch(() => {});
        } else if (url && YT_PLAYER.test(url)) {
          // The player response for the video currently being watched.
          p.then((r) => {
            r.clone().json().then((j) => {
              const fs = formatsFromYouTube(j);
              if (fs.length) REPORT([{ url, kind: 'formats', source: 'youtube', formats: fs, title: ytTitle() }]);
            }).catch(() => {});
          }).catch(() => {});
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
            if (MANIFEST.test(url) || LIKELY_MANIFEST.test(url) || /mpegurl|dash\+xml/i.test(ct)) {
              REPORT([{ url, kind: 'stream' }]);
              if (isText && typeof this.responseText === 'string') {
                const f = formatsFromBody(this.responseText, url);
                if (f && f.formats.length) {
                  seenManifests.set(url, { kind: f.kind, formats: f.formats });
                  REPORT([{ url, kind: 'formats', source: f.kind, formats: f.formats }]);
                }
              }
            } else if (YT_PLAYER.test(url) && isText && typeof this.responseText === 'string') {
              const fs = formatsFromYouTube(JSON.parse(this.responseText));
              if (fs.length) REPORT([{ url, kind: 'formats', source: 'youtube', formats: fs, title: ytTitle() }]);
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

  // MSE in use is a strong hint that a stream is playing even when no single URL looks like one.
  if (window.MediaSource && window.MediaSource.prototype && window.MediaSource.prototype.addSourceBuffer) {
    const add = window.MediaSource.prototype.addSourceBuffer;
    let told = false;
    window.MediaSource.prototype.addSourceBuffer = function (mime) {
      try {
        if (!told) {
          told = true;
          REPORT([{ url: location.href, kind: 'mse', mime: String(mime) }]);
        }
      } catch {
        /* ignore */
      }
      return add.apply(this, arguments);
    };
  }

  // ── on-demand: the formats for whatever is playing right now ───────────
  //
  // The content script asks for this when the button is clicked, rather than the hook pushing
  // continuously. The player response can change without a network request the hook sees
  // (SPA navigation, a different player instance), and the global is always the current one.

  function ytTitle() {
    try {
      const d = window.ytInitialPlayerResponse && window.ytInitialPlayerResponse.videoDetails;
      if (d && d.title) return d.title;
      const el = document.querySelector('h1.ytd-watch-metadata, #title h1, h1.title');
      const t = el && el.textContent && el.textContent.trim();
      return t || document.title.replace(/ - YouTube$/, '') || null;
    } catch {
      return null;
    }
  }

  /** Everything the picker can offer for the current page. */
  function currentFormats() {
    const out = { source: null, title: null, formats: [], page: location.href };

    // 1. YouTube: the player response holds the full per-codec list.
    const yt = window.ytInitialPlayerResponse || window.playerResponse;
    if (yt && yt.streamingData) {
      const fs = formatsFromYouTube(yt);
      if (fs.length) {
        return { source: 'youtube', title: ytTitle(), formats: fs, page: location.href };
      }
    }

    // 2. A manifest the page fetched earlier. Answering with only the <video> element would
    //    silently downgrade a four-variant stream to one progressive file, and the content
    //    script re-queries on a timer, so this list has to stay the better one.
    let best = null;
    for (const [url, m] of seenManifests) {
      if (!m.formats.length) continue;
      if (!best || m.formats.length > best.formats.length) best = { url, ...m };
    }
    if (best) {
      out.source = best.kind;
      out.title = document.title || null;
      out.formats = best.formats.slice();
    }

    // 3. A <video> element with a plain source. Kept alongside the manifest rather than
    //    instead of it: an unlabelled progressive file is still worth offering.
    const v = document.querySelector('video');
    if (v) {
      const src = v.currentSrc || v.src;
      if (src && /^https?:/i.test(src) && !/^blob:/i.test(src)) {
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
      payload = currentFormats();
    } catch (err) {
      payload = { source: null, formats: [], error: String(err && err.message) };
    }
    window.postMessage({ __reidm: true, kind: 'formats-answer', id: e.data.id, payload }, '*');
  });

  // A quick unsolicited report, so the button can appear without the user clicking first.
  const announce = () => {
    try {
      const f = currentFormats();
      if (f.formats.length) REPORT([{ url: location.href, kind: 'formats', ...f }]);
    } catch {
      /* ignore */
    }
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', announce, { once: true });
  } else {
    setTimeout(announce, 0);
  }
  window.addEventListener('load', announce, { once: true });
  // SPA navigations replace the video without a load event.
  let lastHref = location.href;
  setInterval(() => {
    if (location.href !== lastHref) {
      lastHref = location.href;
      announce();
    }
  }, 2000);
})();
