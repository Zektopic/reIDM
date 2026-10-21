#!/usr/bin/env node
// End-to-end test of browser download capture, in a real Chromium, with no dependencies.
//
// Why not Playwright: Playwright manages downloads itself (CDP `Browser.setDownloadBehavior`),
// and with that in force Chrome decides download paths without asking extensions — the very
// hook reIDM's extension relies on would never run, and a test would pass or fail for reasons
// that have nothing to do with a real browser. This drives Chromium over plain CDP instead,
// leaves downloads entirely to the browser's own settings, and uses *trusted* input events
// (`Input.dispatchMouseEvent`), because the extension deliberately ignores synthetic clicks.
//
// What it checks, against a local test site and a real reIDM daemon:
//   1. a file link is taken on click, after the "Download file info" dialog shows its name and
//      size and the user presses Start download: reIDM has it, the browser never requested it,
//      and a notification says so;
//   1b. Cancel in that dialog, for a download the browser started: nobody downloads it;
//   2. a download only recognisable from its response headers is taken from the browser:
//      reIDM has it and the browser kept nothing — not even a 2 KB file;
//   3. cookies reach reIDM, so a download behind a login works;
//   4. a form POST that returns a file stays with the browser;
//   5. Alt+click leaves the download to the browser;
//   6. a "file" link that is really a web page opens in the browser instead of being saved;
//   7. with reIDM stopped, the extension starts it through the native host and still takes over;
//   8. with reIDM stopped and no native host, the browser simply downloads the file;
//   9. the resource grabber lists what the page offers, hides noise, and hands the selection over;
//  10. a magnet link goes to reIDM as a torrent, and the browser does nothing with it;
//  11. the dialog: Enter on Cancel cancels; two downloads at once share one window and are both
//      written down while held; the popup's "Download selected" survives the popup closing; the
//      "best quality" video row; a refusal says the browser has it; a window closed at once, or
//      a worker stopped while two downloads are held, never leaves a download stuck;
//  12. with the dialog off, a program still asks; a page cannot use privileged messages, spoof
//      a YouTube download or store junk; an unchanged status is not rewritten; cookie stores;
//  13. finding media: a live stream is not re-fetched in a loop, per-tab badges, relative and
//      SPA manifests, YouTube SPA navigation, images never wake the worker, and the page hook
//      exists only while media detection is on;
//  14. while reIDM waits for a new address, a captured link asks "Use this link to refresh …?":
//      Refresh moves the waiting download to the link, New download adds the link as its own
//      (at once with the dialog off). Skipped, saying so, against a service that doesn't report
//      `refresh_armed` in /ext/ping (before 0.8);
//  15. a hand-off that timed out is asked again with the same `request_id`.
//
// REIDM_PROMPT=1 turns on the browser's "Ask where to save each file" and runs checks 1–2 and
// the dialog checks that end with reIDM taking the download;
// REIDM_ONLY_GRABBER=1 runs check 9 only; KEEP=1 keeps the temporary profile and logs.
//
// Usage (needs a display; use xvfb-run on a headless machine):
//   cargo build -p reidm -p reidm-host
//   xvfb-run -a node tools/capture-test.mjs --chrome /path/to/chrome
//
// On Windows, browsers find native hosts through the registry, and the service registers itself
// for Chrome, Edge, Brave and Vivaldi when it starts. The test notes those registrations first and
// puts them back at the end; checks 7 and 8 add and remove the one under
// HKCU\Software\<--host-key>\NativeMessagingHosts (default Google\Chrome, which Chrome for
// Testing reads too). Stop any running reIDM first: the test needs its ports.
//
// Exits non-zero if any check fails.

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, all) => (a.startsWith('--') ? [...acc, [a.slice(2), all[i + 1]]] : acc), []),
);
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const WIN = process.platform === 'win32';
const EXE = WIN ? '.exe' : '';
/** The newest Chromium that Playwright has installed (`npx playwright install chromium`). */
function playwrightChromium() {
  const root = path.join(os.homedir(), '.cache/ms-playwright');
  const builds = (fs.existsSync(root) ? fs.readdirSync(root) : [])
    .filter((d) => /^chromium-\d+$/.test(d))
    .sort((a, b) => Number(b.split('-')[1]) - Number(a.split('-')[1]));
  for (const b of builds) {
    const exe = path.join(root, b, 'chrome-linux64', 'chrome');
    if (fs.existsSync(exe)) return exe;
  }
  return null;
}
const CHROME = args.chrome || process.env.CHROME_PATH || playwrightChromium();
if (!CHROME) {
  console.error('No browser: pass --chrome PATH, set CHROME_PATH, or run `npx playwright install chromium`.');
  process.exit(2);
}
const BIN = path.resolve(args.bin || path.join(ROOT, 'target/debug'));
const EXT = path.resolve(args.extension || path.join(ROOT, 'extension'));
const EXT_ID = 'pgeonmdaagbiccimgojpafdjcipcfnlc';
const hostKey = (base) => `HKCU\\Software\\${base}\\NativeMessagingHosts\\com.zektopic.reidm`;
const HOST_KEY = hostKey(args['host-key'] || 'Google\\Chrome');
const REG_KEYS = ['Google\\Chrome', 'Microsoft\\Edge', 'BraveSoftware\\Brave-Browser', 'Vivaldi'].map(hostKey);
const regValue = (key) => /REG_SZ\s+(.*)/.exec(spawnSync('reg', ['query', key, '/ve'], { encoding: 'utf8' }).stdout || '')?.[1].trim();
const savedReg = WIN ? new Map(REG_KEYS.map((k) => [k, regValue(k)])) : new Map();
const SITE_PORT = Number(args['site-port'] || 18871);
const CDP_PORT = Number(args['cdp-port'] || 9333);
const API = 'http://127.0.0.1:61002';

const WORK = fs.mkdtempSync(path.join(os.tmpdir(), 'reidm-capture-'));
const DATA = path.join(WORK, 'data'); // the daemon's data folder
const REIDM_DL = path.join(WORK, 'reidm-downloads');
const BROWSER_DL = path.join(WORK, 'browser-downloads');
const PROFILE = path.join(WORK, 'profile');
for (const d of [DATA, REIDM_DL, BROWSER_DL, path.join(PROFILE, 'Default')]) fs.mkdirSync(d, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const results = [];
function check(name, ok, detail = '') {
  results.push({ name, ok });
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? ` — ${detail}` : ''}`);
}

// ── the test site ───────────────────────────────────────────────────────

const body = (n, seed) => Buffer.from(Array.from({ length: n }, (_, i) => (i * 7 + seed) % 251));
const FILES = {
  '/files/app.zip': body(3 << 20, 1),
  '/files/alt.zip': body(64 << 10, 2),
  '/dl/report': body(2 << 20, 3),
  '/dl/tiny': body(2048, 4),
  '/private/secret.bin': body(1 << 20, 5),
  '/dl/launched': body(512 << 10, 6),
  '/dl/fallback': body(300 << 10, 7),
  '/dl/cancelled': body(100 << 10, 8),
  '/dl/enter': body(90 << 10, 11),
  '/dl/two-a': body(120 << 10, 12),
  '/dl/two-b': body(130 << 10, 13),
  '/dl/refuse': body(150 << 10, 14),
  '/dl/closed': body(110 << 10, 15),
  '/files/q1.zip': body(70 << 10, 16),
  '/files/q2.zip': body(80 << 10, 17),
  '/files/setup.exe': body(60 << 10, 18),
  // Refreshing an address (M14): two downloads waiting for one, a mirror of the first, and two
  // other files that are not either.
  '/files/rfa.zip': body(140 << 10, 23),
  '/files/rfb.zip': body(150 << 10, 24),
  '/dl/other': body(160 << 10, 25),
  '/files/other.zip': body(170 << 10, 26),
};
FILES['/mirror/rfa.zip'] = FILES['/files/rfa.zip'];
/** Served a little at a time, so a download the extension pauses is really still unfinished. */
const SLOW = { '/slow/a': body(600 << 10, 21), '/slow/b': body(700 << 10, 22) };
const log = [];

const PAGE = `<!doctype html><html><head><title>capture test</title>
<style>a,button{display:block;margin:6px;font:16px sans-serif}</style></head><body>
<a id="file" href="/files/app.zip">app.zip (file link)</a>
<a id="report" href="/dl/report?id=7">report (attachment, no extension)</a>
<a id="tiny" href="/dl/tiny?id=8">tiny (2 KB attachment)</a>
<a id="private" href="/private/secret.bin">secret.bin (needs the login cookie)</a>
<a id="alt" href="/files/alt.zip">alt.zip (Alt+click)</a>
<a id="fake" href="/files/fake.zip">fake.zip (really a web page)</a>
<a id="launched" href="/dl/launched?id=9">launched (reIDM stopped, host registered)</a>
<a id="fallback" href="/dl/fallback?id=10">fallback (reIDM stopped, no host)</a>
<a id="cancelled" href="/dl/cancelled?id=11">cancelled (Cancel in the dialog)</a>
<a id="enter" href="/dl/enter?id=12">enter (Enter on Cancel)</a>
<a id="refuse" href="/dl/refuse?id=13">refuse (reIDM is refused, the browser is not)</a>
<a id="closed" href="/dl/closed?id=14">closed (dialog closed at once)</a>
<a id="setup" href="/files/setup.exe">setup.exe (a program)</a>
<a id="mirror" href="/mirror/rfa.zip">rfa.zip from a mirror (refresh)</a>
<a id="other" href="/dl/other?id=30">other (not the download being refreshed)</a>
<a id="other2" href="/files/other.zip">other.zip (not the download being refreshed either)</a>
<a id="magnet" href="magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Test%20Torrent.bin">a magnet link</a>
<form method="post" action="/post/export"><input type="hidden" name="q" value="1">
<button id="post" type="submit">POST export</button></form>
</body></html>`;

// A live HLS stream whose player polls its playlists every second (H4).
const LIVE = `<!doctype html><title>live</title><script>
setInterval(() => fetch('/hls/master.m3u8').then(r => r.text()).then(() => fetch('/hls/media.m3u8')), 1000);
</script>`;
const MASTER = '#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS="avc1.4d401e,mp4a.40.2"\nmedia.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720\nmedia.m3u8?hd\n';
const MEDIA_PLAYLIST = '#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nseg1.ts\n';
// A single-page video site: a relative manifest fetch, then another video after pushState (M21, M22).
const hlsBody = (n) => '#EXTM3U\n' + Array.from({ length: n }, (_, i) => `#EXT-X-STREAM-INF:BANDWIDTH=${(i + 1) * 500000},RESOLUTION=${(i + 1) * 320}x${(i + 1) * 180}\nv${i}.m3u8\n`).join('');
const SPA = `<!doctype html><title>spa</title><script>fetch('hls/a.m3u8');
window.next = () => { history.pushState({}, '', '/spa/two'); return fetch('hls/b.m3u8').then(r => r.text()); };</script>`;
// YouTube's shape, on the test site: the global describes the first video, the player API the
// next one (H6).
const ytPlayer = (id, title, itag) => ({ videoDetails: { videoId: id, title }, streamingData: { formats: [{ itag, mimeType: 'video/mp4; codecs="avc1.42001E, mp4a.40.2"', qualityLabel: '360p', height: 360, url: `https://example.invalid/${id}` }] } });
const YT = `<!doctype html><title>Video A - YouTube</title><script>
window.ytInitialPlayerResponse = ${JSON.stringify(ytPlayer('AAAAAAAAAAA', 'Video A', 18))};
window.toB = () => { history.pushState({}, '', '/yt?v=BBBBBBBBBBB'); document.title = 'Video B - YouTube'; return fetch('/youtubei/v1/player', { method: 'POST' }).then(r => r.json()); };
window.toC = () => { history.pushState({}, '', '/yt?v=CCCCCCCCCCC'); document.title = 'Video C - YouTube'; };
</script>`;
// Lots of images and scripts, none of them media (M5).
const BUSY = `<!doctype html><title>busy</title>${Array.from({ length: 60 }, (_, i) => `<img src="/img/icon.svg?${i}" width=4>`).join('')}`;

const svg = (w, h) => `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><rect width="100%" height="100%" fill="#3b8cff"/></svg>`;
const GALLERY = `<!doctype html><html><head><title>gallery</title>
<meta property="og:image" content="/img/card.svg"></head><body>
<img id="big" src="/img/big.svg" alt="big picture">
<img src="/img/icon.svg" alt="icon">
<img srcset="/img/a-480.svg 480w, /img/a-1080.svg 1080w" src="/img/a-480.svg" sizes="200px" alt="responsive">
<img data-src="/img/lazy.svg" src="/img/icon.svg" alt="lazy">
<div style="width:200px;height:100px;background-image:url('/img/bg.svg')"></div>
<video src="/media/clip.mp4" poster="/img/poster.svg"><track src="/media/subs.vtt" srclang="en"></video>
<a href="/files/doc.pdf">the manual</a>
<a href="/files/app.zip">the app</a>
<a href="/about">about us</a>
<script>fetch('/api/data.json').catch(() => {});</script>
</body></html>`;
const SVGS = { '/img/big.svg': svg(800, 600), '/img/icon.svg': svg(16, 16), '/img/a-480.svg': svg(480, 320), '/img/a-1080.svg': svg(1080, 720),
  '/img/lazy.svg': svg(640, 480), '/img/bg.svg': svg(400, 200), '/img/poster.svg': svg(1280, 720), '/img/card.svg': svg(1200, 630) };

const site = http.createServer((req, res) => {
  const u = new URL(req.url, `http://127.0.0.1:${SITE_PORT}`);
  if (u.pathname === '/gallery') {
    res.writeHead(200, { 'Content-Type': 'text/html' });
    return res.end(GALLERY);
  }
  if (SVGS[u.pathname]) {
    res.writeHead(200, { 'Content-Type': 'image/svg+xml' });
    return res.end(SVGS[u.pathname]);
  }
  if (['/media/clip.mp4', '/media/subs.vtt', '/files/doc.pdf', '/api/data.json', '/about'].includes(u.pathname)) {
    const type = { '.mp4': 'video/mp4', '.vtt': 'text/vtt', '.pdf': 'application/pdf', '.json': 'application/json' }[path.extname(u.pathname)] || 'text/html';
    const data = body(u.pathname === '/media/clip.mp4' ? 300 << 10 : 4096, 9);
    res.writeHead(200, { 'Content-Type': type, 'Content-Length': data.length, 'Accept-Ranges': 'bytes' });
    return res.end(data);
  }
  log.push({ method: req.method, path: u.pathname, range: req.headers.range, cookie: req.headers.cookie || '', mode: req.headers['sec-fetch-mode'] || '', site: req.headers['sec-fetch-site'] || '' });
  const page = { '/live': LIVE, '/spa/one': SPA, '/yt': YT, '/busy': BUSY, '/watch': '<!doctype html><title>a video page</title>' }[u.pathname];
  if (page) {
    res.writeHead(200, { 'Content-Type': 'text/html' });
    return res.end(page);
  }
  if (u.pathname === '/hls/master.m3u8' || u.pathname === '/hls/media.m3u8') {
    res.writeHead(200, { 'Content-Type': 'application/vnd.apple.mpegurl' });
    return res.end(u.pathname === '/hls/master.m3u8' ? MASTER : MEDIA_PLAYLIST);
  }
  if (u.pathname === '/spa/hls/a.m3u8' || u.pathname === '/spa/hls/b.m3u8') {
    res.writeHead(200, { 'Content-Type': 'application/vnd.apple.mpegurl' });
    return res.end(hlsBody(u.pathname.endsWith('a.m3u8') ? 3 : 1));
  }
  if (u.pathname === '/youtubei/v1/player') {
    req.resume();
    res.writeHead(200, { 'Content-Type': 'application/json' });
    return res.end(JSON.stringify(ytPlayer('BBBBBBBBBBB', 'Video B', 22)));
  }
  if (SLOW[u.pathname]) {
    const data = SLOW[u.pathname];
    res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Disposition': `attachment; filename="${path.basename(u.pathname)}.bin"`, 'Content-Length': data.length });
    let at = 0;
    const tick = () => {
      if (res.destroyed) return;
      res.write(data.subarray(at, at + (20 << 10)));
      at += 20 << 10;
      if (at < data.length) setTimeout(tick, 150);
      else res.end();
    };
    return tick();
  }
  if (u.pathname === '/dl/refuse' && req.headers.range) {
    // reIDM checks with a ranged request; the browser's own request has none.
    res.writeHead(403);
    return res.end('no');
  }
  if (u.pathname === '/') {
    res.writeHead(200, { 'Content-Type': 'text/html', 'Set-Cookie': 'session=ok; Path=/; HttpOnly' });
    return res.end(PAGE);
  }
  if (u.pathname === '/files/fake.zip') {
    res.writeHead(200, { 'Content-Type': 'text/html' });
    return res.end('<!doctype html><title>Please sign in</title><h1 id="signin">Please sign in</h1>');
  }
  if (u.pathname === '/post/export' && req.method === 'POST') {
    req.resume();
    res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Disposition': 'attachment; filename="export.csv"', 'Content-Length': 11 });
    return res.end('a,b\n1,2\n3,4');
  }
  const data = FILES[u.pathname];
  if (!data) {
    res.writeHead(404);
    return res.end();
  }
  if (u.pathname.startsWith('/private/') && !/(^|;\s*)session=ok(;|$)/.test(req.headers.cookie || '')) {
    res.writeHead(403);
    return res.end('login required');
  }
  const headers = { 'Accept-Ranges': 'bytes', ETag: '"v1"' };
  if (u.pathname.startsWith('/dl/')) {
    headers['Content-Type'] = 'application/octet-stream';
    headers['Content-Disposition'] = `attachment; filename="${path.basename(u.pathname)}.bin"`;
  } else {
    headers['Content-Type'] = 'application/zip';
  }
  const m = /bytes=(\d+)-(\d*)/.exec(req.headers.range || '');
  if (m) {
    const a = Number(m[1]);
    const b = m[2] ? Math.min(Number(m[2]), data.length - 1) : data.length - 1;
    res.writeHead(206, { ...headers, 'Content-Range': `bytes ${a}-${b}/${data.length}`, 'Content-Length': b - a + 1 });
    return res.end(data.subarray(a, b + 1));
  }
  res.writeHead(200, { ...headers, 'Content-Length': data.length });
  res.end(data);
});

// ── reIDM ───────────────────────────────────────────────────────────────

const env = { ...process.env, REIDM_DATA_DIR: DATA, RUST_LOG: 'reidm=info,reidm_core=info' };
let daemon = null;
async function startDaemon() {
  daemon = spawn(path.join(BIN, `reidm${EXE}`), ['daemon', '--no-idm-port', '--dir', REIDM_DL], { env, stdio: ['ignore', 'ignore', 'inherit'] });
  for (let i = 0; i < 100; i++) {
    if (await apiUp()) return;
    await sleep(100);
  }
  throw new Error('daemon did not start');
}
async function stopDaemon() {
  if (daemon) daemon.kill('SIGTERM');
  daemon = null;
  // Wait for both ports, not just the API's: the service closes its HTTP port first and its
  // extension socket only once the list is flushed, and the next run needs both. A daemon the
  // native host started is not our child; stop whatever still holds a port after a grace period.
  for (let i = 0; i < 100; i++) {
    const held = [];
    for (const port of [61001, 61002]) if (await portOpen(port)) held.push(port);
    if (!held.length) return;
    if (i >= 20) {
      for (const port of held) {
        if (WIN) {
          const re = new RegExp(`127\\.0\\.0\\.1:${port}\\s.*LISTENING`);
          const pids = new Set(spawnSync('netstat', ['-ano', '-p', 'TCP'], { encoding: 'utf8' }).stdout.split('\n')
            .filter((l) => re.test(l)).map((l) => l.trim().split(/\s+/).pop()));
          for (const pid of pids) spawnSync('taskkill', ['/F', '/PID', pid]);
        } else {
          spawnSync('fuser', ['-k', '-TERM', `${port}/tcp`]);
        }
      }
    }
    await sleep(100);
  }
  throw new Error('reIDM still holds its ports 10 s after being asked to stop');
}
function portOpen(port) {
  return new Promise((resolve) => {
    const c = net.connect({ host: '127.0.0.1', port }, () => {
      c.destroy();
      resolve(true);
    });
    c.on('error', () => resolve(false));
  });
}
async function apiUp() {
  try {
    await fetch(`${API}/ext/ping`);
    return true;
  } catch {
    return false;
  }
}
function token() {
  return fs.readFileSync(path.join(DATA, 'api-token'), 'utf8').trim();
}
async function reidmItems() {
  const r = await fetch(`${API}/api/downloads`, { headers: { 'X-Reidm-Token': token() } });
  return r.json();
}
const itemFor = (items, p) => items.find((i) => new URL(i.request.url).pathname === p);

// ── Chromium over CDP ───────────────────────────────────────────────────

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.id = 0;
    this.pending = new Map();
    this.handlers = [];
    ws.onmessage = (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id && this.pending.has(m.id)) {
        const { resolve, reject } = this.pending.get(m.id);
        this.pending.delete(m.id);
        if (m.error) reject(new Error(`${m.error.message} ${m.error.data || ''}`));
        else resolve(m.result);
      } else if (m.method) {
        for (const h of this.handlers) h(m);
      }
    };
  }
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((resolve, reject) => {
      ws.onopen = resolve;
      ws.onerror = reject;
    });
    return new Cdp(ws);
  }
  /** A reply that never comes (a session whose target went away) fails the call after 30 s
   *  instead of hanging the whole run. */
  send(method, params = {}, sessionId) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params, sessionId }));
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`no answer to ${method} in 30 s${params.expression ? `: ${params.expression.slice(0, 120)}` : ''} (session ${sessionId === worker ? 'worker' : sessionId === page ? 'page' : sessionId || 'browser'})`));
      }, 30000);
      this.pending.set(id, {
        resolve: (v) => (clearTimeout(t), resolve(v)),
        reject: (e) => (clearTimeout(t), reject(e)),
      });
    });
  }
}

let chrome = null;
let cdp = null;
let page = null; // session id
let worker = null; // session id
const contexts = new Map(); // session id -> execution contexts

async function launchChrome() {
  fs.writeFileSync(
    path.join(PROFILE, 'Default', 'Preferences'),
    JSON.stringify({
      download: { default_directory: BROWSER_DL, prompt_for_download: !!process.env.REIDM_PROMPT, directory_upgrade: true },
      savefile: { default_directory: BROWSER_DL },
      // A page that starts several downloads at once (the dialog-queue checks) would otherwise
      // meet Chrome's "download multiple files?" question.
      profile: { content_settings: { exceptions: { automatic_downloads: { [`http://127.0.0.1:${SITE_PORT},*`]: { setting: 1 } } } } },
    }),
  );
  chrome = spawn(
    CHROME,
    [
      `--user-data-dir=${PROFILE}`,
      `--remote-debugging-port=${CDP_PORT}`,
      `--disable-extensions-except=${EXT}`,
      `--load-extension=${EXT}`,
      // Chrome 137+ ignores --load-extension without this.
      '--disable-features=DisableLoadExtensionCommandLineSwitch',
      '--no-first-run',
      '--no-default-browser-check',
      '--password-store=basic',
      'about:blank',
    ],
    { env, stdio: 'ignore' },
  );
  // A browser that can't start is an error like any other, so the cleanup below still runs.
  let failed = null;
  chrome.once('error', (e) => (failed = e));
  let version;
  for (let i = 0; i < 100 && !version; i++) {
    if (failed) throw failed;
    try {
      version = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`)).json();
    } catch {
      await sleep(100);
    }
  }
  if (!version) throw new Error('Chromium did not start');
  cdp = await Cdp.connect(version.webSocketDebuggerUrl);
  // The execution contexts of every attached page, so a check can run code in the extension's
  // content script (its isolated world) rather than the page's.
  cdp.handlers.push((m) => {
    if (m.sessionId && m.sessionId === worker) {
      if (m.method === 'Runtime.exceptionThrown') console.log(`worker: ${m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text}`);
      else if (m.method === 'Runtime.consoleAPICalled' && ['error', 'warning'].includes(m.params.type)) console.log(`worker ${m.params.type}: ${m.params.args.map((a) => a.value ?? a.description).join(' ')}`);
    }
    if (m.method !== 'Runtime.executionContextCreated') return;
    const list = contexts.get(m.sessionId) || [];
    list.push(m.params.context);
    contexts.set(m.sessionId, list);
  });
  console.log(`browser: ${version.Browser}`);
}

async function attachWorker() {
  for (let i = 0; i < 100; i++) {
    const { targetInfos } = await cdp.send('Target.getTargets');
    const sw = targetInfos.find((t) => t.type === 'service_worker' && t.url.startsWith(`chrome-extension://${EXT_ID}/`));
    if (sw) {
      const { sessionId } = await cdp.send('Target.attachToTarget', { targetId: sw.targetId, flatten: true });
      worker = sessionId;
      // The worker's own errors, so a failure here says what went wrong in it.
      await cdp.send('Runtime.enable', {}, worker).catch(() => {});
      return sw.url;
    }
    await sleep(100);
  }
  return null;
}

async function inWorker(expression) {
  // The worker may have been stopped and restarted since we attached, and a worker just
  // started may not have finished running its script (`reidm` not defined yet).
  for (let attempt = 0; ; attempt++) {
    try {
      const r = await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, worker);
      if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
      return r.result.value;
    } catch (e) {
      if (attempt >= 4) throw e;
      await sleep(300);
      await attachWorker();
    }
  }
}

let pageTarget = null;
async function openPage(url) {
  const { targetId } = await cdp.send('Target.createTarget', { url });
  pageTarget = targetId;
  const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
  page = sessionId;
  await cdp.send('Page.enable', {}, page);
  await cdp.send('Runtime.enable', {}, page);
  await sleep(1500);
}

async function evalPage(expression) {
  const r = await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, page);
  return r.result.value;
}

/** Press a button in the dialog. Asynchronously: the press may close the window, and an
 *  evaluation whose page goes away never answers. */
const press = (d, id) => d.run(`setTimeout(() => document.getElementById(${JSON.stringify(id)}).click(), 0), true`);

/** Run code as the extension's content script in the current page — what a compromised page
 *  could at worst get to say to the service worker. */
async function inContentScript(expression) {
  const ctx = (contexts.get(page) || []).filter((c) => c.auxData && c.auxData.type === 'isolated' && c.name === 'reIDM Integration').pop();
  if (!ctx) return { error: 'no content script context' };
  const r = await cdp.send('Runtime.evaluate', { expression, contextId: ctx.id, awaitPromise: true, returnByValue: true }, page);
  return r.result.value;
}

/** Ask the page hook for the page's formats, as the content script does. */
const hookAnswer = () => evalPage(`new Promise((resolve) => {
  const id = Math.random();
  addEventListener('message', function on(e) {
    if (e.data && e.data.kind === 'formats-answer' && e.data.id === id) { removeEventListener('message', on); resolve(e.data.payload); }
  });
  postMessage({ __reidmQuery: true, id }, '*');
  setTimeout(() => resolve(null), 3000);
})`);

const confirmTargets = async () => (await cdp.send('Target.getTargets')).targetInfos.filter((x) => x.type === 'page' && x.url.startsWith(`chrome-extension://${EXT_ID}/confirm.html`));
const browserDownloads = () => inWorker('chrome.downloads.search({}).then(d => d.map(x => ({ url: x.url, state: x.state, paused: x.paused, filename: x.filename })))');
/** What the dialog shows: the download's name, the waiting count, its message, which buttons. */
const dialogView = (d) => d.run(`({ url: document.getElementById('url').textContent, name: document.getElementById('name').value,
  waiting: document.getElementById('waiting').hidden ? 0 : parseInt(document.getElementById('waiting').textContent, 10),
  err: document.getElementById('err').textContent, asking: !document.querySelector('.ask').hidden && !document.getElementById('start').disabled,
  close: !document.getElementById('close').hidden, program: !document.getElementById('program').hidden })`);
const dialogShowing = (d, re) => waitFor('the dialog shows it', async () => {
  const v = await dialogView(d).catch(() => null);
  return v && v.asking && re.test(v.url) ? v : null;
}, 15000);
const recent = () => inWorker(`chrome.storage.session.get('reidm-recent-captures').then(o => o['reidm-recent-captures'] || [])`);
async function deleteItem(it) {
  if (it) await fetch(`${API}/api/downloads/${it.id}`, { method: 'DELETE', headers: { 'X-Reidm-Token': token() } });
}

/** A real, trusted click in the middle of the element. `modifiers`: 1 Alt, 2 Ctrl, 4 Meta, 8 Shift. */
async function click(selector, modifiers = 0) {
  const box = await evalPage(`(() => { const e = document.querySelector(${JSON.stringify(selector)}); if (!e) return null; const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
  if (!box) throw new Error(`no ${selector} on ${await evalPage('location.href')}`);
  for (const type of ['mousePressed', 'mouseReleased']) {
    await cdp.send('Input.dispatchMouseEvent', { type, x: box.x, y: box.y, button: 'left', clickCount: 1, modifiers }, page);
  }
}

/** Optional: `REIDM_SHOT_CMD="python3 shot.py"` saves a screenshot of the whole display, to see
 *  whether the browser's own download UI ever appears. */
function shot(name) {
  const cmd = process.env.REIDM_SHOT_CMD;
  if (!cmd) return;
  const out = path.join(process.env.REIDM_SHOT_DIR || WORK, `${name}.png`);
  const [exe, ...rest] = cmd.split(' ');
  spawnSync(exe, [...rest, out], { env });
}

const browserFiles = () => fs.readdirSync(BROWSER_DL).filter((f) => !f.startsWith('.'));
const browserRequested = (p) => log.some((l) => l.path === p && l.mode === 'navigate');

async function waitFor(what, fn, ms = 20000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await fn();
    if (v) return v;
    await sleep(200);
  }
  return null;
}

// ── the checks ──────────────────────────────────────────────────────────

async function grabberChecks() {
  await openPage(`http://127.0.0.1:${SITE_PORT}/gallery`);
  await sleep(1000);
  const tabId = await inWorker(`chrome.tabs.query({}).then(t => t.find(x => x.url.endsWith('/gallery')).id)`);
  const { targetId } = await cdp.send('Target.createTarget', { url: `chrome-extension://${EXT_ID}/grabber.html?tab=${tabId}` });
  const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
  await cdp.send('Runtime.enable', {}, sessionId);
  const inGrabber = async (expression) => (await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, sessionId)).result.value;
  const rows = await waitFor('grabber rows', async () => {
    const r = await inGrabber(`[...document.querySelectorAll('#rows .url')].map(e => e.textContent)`);
    return r && r.length ? r : null;
  });
  const shown = new Set((rows || []).map((u) => new URL(u).pathname));
  for (const p of ['/img/big.svg', '/img/a-1080.svg', '/img/lazy.svg', '/img/bg.svg', '/img/poster.svg', '/img/card.svg', '/media/clip.mp4', '/media/subs.vtt', '/files/doc.pdf', '/files/app.zip']) {
    check(`grabber lists ${p}`, shown.has(p));
  }
  check('grabber hides a 16 px icon by default', !shown.has('/img/icon.svg'));
  check('grabber hides links to pages by default', !shown.has('/about'));
  check('grabber hides the page’s own API calls', !shown.has('/api/data.json'));
  shot('grabber');
  // Pick the documents and archives, and hand them over.
  await inGrabber(`(() => {
    for (const tr of document.querySelectorAll('#rows tr')) {
      const u = tr.querySelector('.url').textContent;
      if (/doc\.pdf$|clip\.mp4$/.test(u)) tr.querySelector('input').click();
    }
    document.getElementById('download').click();
  })()`);
  const msg = await waitFor('grabber result', async () => {
    const m = await inGrabber(`document.getElementById('msg').textContent`);
    return /queued|already|refused/.test(m || '') ? m : null;
  });
  check('grabber hands the selection to reIDM', /2 queued/.test(msg || ''), msg || 'no answer');
  const got = await reidmItems();
  check('reIDM has the grabbed files', !!itemFor(got, '/files/doc.pdf') && !!itemFor(got, '/media/clip.mp4'));
  check('grabbed files carry the page as referer', (itemFor(got, '/files/doc.pdf') || { request: {} }).request.referer?.endsWith('/gallery'));
  await cdp.send('Target.closeTarget', { targetId });

  // M13: "Download all links on this page" opens the grabber with the links to files ticked.
  const all = await cdp.send('Target.createTarget', { url: `chrome-extension://${EXT_ID}/grabber.html?tab=${tabId}&select=files` });
  const allS = (await cdp.send('Target.attachToTarget', { targetId: all.targetId, flatten: true })).sessionId;
  await cdp.send('Runtime.enable', {}, allS);
  const ticked = await waitFor('ticked rows', async () => {
    const r = (await cdp.send('Runtime.evaluate', { expression: `[...document.querySelectorAll('#rows tr.sel .url')].map(e => new URL(e.textContent).pathname)`, returnByValue: true }, allS)).result.value;
    return r && r.length ? r : null;
  });
  check('"all links": the links to files are ticked, pages and images are not', !!ticked && ticked.includes('/files/doc.pdf') && ticked.includes('/files/app.zip') && !ticked.includes('/about') && !ticked.some((u) => u.startsWith('/img/')), JSON.stringify(ticked));
  await cdp.send('Target.closeTarget', { targetId: all.targetId });
}

/** The "Download file info" dialog the extension opens, once it is up: a session in it. */
async function dialog() {
  const t = await waitFor('the download dialog', async () => {
    const { targetInfos } = await cdp.send('Target.getTargets');
    return targetInfos.find((x) => x.type === 'page' && x.url.startsWith(`chrome-extension://${EXT_ID}/confirm.html`));
  }, 10000);
  if (!t) return null;
  const { sessionId } = await cdp.send('Target.attachToTarget', { targetId: t.targetId, flatten: true });
  await cdp.send('Runtime.enable', {}, sessionId);
  const run = async (expression) => (await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, sessionId)).result.value;
  return { run, targetId: t.targetId, session: sessionId };
}


const noDialog = () => waitFor('no dialog window', async () => ((await confirmTargets()).length === 0 ? true : null), 8000);
const onRoot = () => `http://127.0.0.1:${SITE_PORT}`;
/** Downloads the browser made in these checks, removed so the later "the browser kept
 *  nothing" checks still mean what they say. */
async function forgetBrowserDownloads(names) {
  await inWorker('chrome.downloads.erase({}).then(() => true)');
  for (const f of browserFiles()) if (names.some((n) => f.startsWith(n))) fs.rmSync(path.join(BROWSER_DL, f), { force: true });
}
/** An extension page to send messages from, as the popup or the grabber would. */
async function extensionPage(file) {
  const { targetId } = await cdp.send('Target.createTarget', { url: `chrome-extension://${EXT_ID}/${file}` });
  const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
  await sleep(800);
  const run = async (expression) => (await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, sessionId)).result.value;
  return { run, targetId, close: () => cdp.send('Target.closeTarget', { targetId }).catch(() => {}) };
}

/** The dialog, with "Ask before each download" on. */
async function dialogChecks() {
  // M1, M20: Enter on a focused Cancel cancels; "Just now" says so.
  await noDialog();
  await click('#enter');
  let d = await dialog();
  check('Enter on Cancel: the dialog opens', !!d);
  if (d) {
    await dialogShowing(d, /\/dl\/enter/);
    await d.run(`document.getElementById('cancel').focus()`);
    // Not awaited: the key may close the window, and then no answer comes.
    cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, text: '\r' }, d.session).catch(() => {});
    cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 }, d.session).catch(() => {});
    await sleep(2500);
    check('Enter on Cancel: reIDM did not take it', !itemFor(await reidmItems(), '/dl/enter'));
    check('Enter on Cancel: the browser kept nothing', !(await browserDownloads()).some((x) => x.url.includes('/dl/enter')));
    const r = (await recent())[0];
    check('Cancel: "Just now" says cancelled, not "browser kept it"', !!r && r.outcome === 'cancelled' && /enter/.test(r.url), JSON.stringify(r));
  }

  // M3, H3.3: two downloads at once share one window, and both are written down while held.
  await noDialog();
  await evalPage(`for (const p of ['/dl/two-a?id=20', '/dl/two-b?id=21']) { const f = document.createElement('iframe'); f.style.display = 'none'; f.src = p; document.body.appendChild(f); }`);
  d = await dialog();
  const both = d && (await waitFor('two waiting', async () => {
    const v = await dialogView(d).catch(() => null);
    return v && v.asking && v.waiting === 1 ? v : null;
  }, 15000));
  check('two downloads at once: one dialog window says "1 more waiting"', !!both && (await confirmTargets()).length === 1, JSON.stringify(both));
  const held = await inWorker(`chrome.storage.local.get(null).then(o => Object.keys(o).filter(k => k.startsWith('reidm-pending:')).length)`);
  check('two downloads at once: both are written down while held', held === 2, `${held} held`);
  if (d) {
    for (let i = 0; i < 2; i++) {
      const v = await dialogShowing(d, /\/dl\/two-/);
      if (v) await press(d, 'start');
      await sleep(400);
    }
  }
  const twoDone = await waitFor('both in reIDM', async () => {
    const items = await reidmItems();
    return itemFor(items, '/dl/two-a') && itemFor(items, '/dl/two-b') ? true : null;
  });
  check('two downloads at once: reIDM has both', !!twoDone);
  check('two downloads at once: the dialog closed after the last', !!(await noDialog()));

  // H2: the popup's "Download selected" for two links goes on after the popup has closed.
  const siteTab = await inWorker(`chrome.tabs.query({}).then(t => t.find(x => x.url === '${onRoot()}/').id)`);
  const pop = await extensionPage('popup.html');
  await pop.run(`chrome.runtime.sendMessage({ type: 'queue-media', tabId: ${siteTab}, items: [{ url: '${onRoot()}/files/q1.zip' }, { url: '${onRoot()}/files/q2.zip' }] }); true`);
  await pop.close();
  d = await dialog();
  if (d) {
    for (let i = 0; i < 2; i++) {
      const v = await dialogShowing(d, /\/files\/q[12]\.zip/);
      if (v) await press(d, 'start');
      await sleep(600);
    }
  }
  const queued = await waitFor('both popup items in reIDM', async () => {
    const items = await reidmItems();
    return itemFor(items, '/files/q1.zip') && itemFor(items, '/files/q2.zip') ? true : null;
  });
  check('popup closed at once: both selected items still reach reIDM', !!queued);
  await noDialog();

  // H1: the popup's YouTube row ("best quality", no format) with the dialog on.
  const opt = await extensionPage('options.html');
  await opt.run(`chrome.runtime.sendMessage({ type: 'download-video', tabId: ${siteTab}, page: '${onRoot()}/watch?v=best1', title: 'A video' }); true`);
  d = await dialog();
  const vv = d && (await dialogShowing(d, /watch\?v=best1/));
  check('best-quality video row: the dialog opens with the title as its name', !!vv && vv.name === 'A video.mp4', JSON.stringify(vv));
  if (d) await press(d, 'later');
  const vid = await waitFor('the video in reIDM', async () => (await reidmItems()).find((i) => i.request.url.endsWith('/watch?v=best1')));
  check('best-quality video row: reIDM gets it with yt-dlp’s best format', !!vid && /^bestvideo/.test((vid.request.media || {}).format || ''), vid && JSON.stringify(vid.request.media));
  await deleteItem(vid);
  await noDialog();

  await refreshChecks();
  if (!process.env.REIDM_PROMPT) await fallbackDialogChecks();
  await opt.close();
}

/** The dialog when the browser ends up downloading: reIDM refuses, the dialog vanishes, the
 *  extension's worker stops. Not with "Ask where to save" on, which would stop at Save As. */
async function fallbackDialogChecks() {
  // M2: reIDM refuses; the browser downloads it, and the dialog says so with one Close button.
  await click('#refuse');
  let d = await dialog();
  if (d) {
    await dialogShowing(d, /\/dl\/refuse/);
    await press(d, 'start');
  }
  const shown = d && (await waitFor('the refusal', async () => {
    const v = await dialogView(d).catch(() => null);
    return v && v.close && /couldn't take it/.test(v.err) ? v : null;
  }));
  check('reIDM refuses: the dialog says the browser downloads it instead, with only Close', !!shown && /browser downloads it instead/.test(shown.err) && !shown.asking, JSON.stringify(shown));
  const refused = await waitFor('refuse.bin in the browser folder', async () => browserFiles().find((f) => f === 'refuse.bin'));
  check('reIDM refuses: the browser downloaded it, intact', !!refused && Buffer.compare(fs.readFileSync(path.join(BROWSER_DL, refused)), FILES['/dl/refuse']) === 0);
  if (d) await press(d, 'close');
  check('reIDM refuses: Close closes the dialog', !!(await noDialog()));

  // H3.1: the dialog window is closed the moment it appears, before its page can connect.
  await cdp.send('Target.setDiscoverTargets', { discover: true });
  let closed = false;
  const closer = (m) => {
    const t = m.params && m.params.targetInfo;
    if (!closed && t && /^Target\.target(Created|InfoChanged)$/.test(m.method) && t.url.includes('/confirm.html')) {
      closed = true;
      cdp.send('Target.closeTarget', { targetId: t.targetId }).catch(() => {});
    }
  };
  cdp.handlers.push(closer);
  await click('#closed');
  await waitFor('the dialog closed', async () => closed || null, 10000);
  await sleep(4000);
  cdp.handlers.splice(cdp.handlers.indexOf(closer), 1);
  await cdp.send('Target.setDiscoverTargets', { discover: false });
  const left = (await browserDownloads()).filter((x) => x.url.includes('/dl/closed'));
  const file = browserFiles().find((f) => f === 'closed.bin');
  check('dialog closed at once: the download is not left stuck', !left.some((x) => x.state === 'in_progress' && x.paused), `${file ? 'the browser has it' : left.length ? JSON.stringify(left) : 'cancelled'}`);

  // H3.2, H3.5: the worker stops while two downloads are held in its dialog.
  await noDialog();
  await evalPage(`for (const p of ['/slow/a', '/slow/b']) { const f = document.createElement('iframe'); f.style.display = 'none'; f.src = p; document.body.appendChild(f); }`);
  d = await dialog();
  if (d) await waitFor('two waiting', async () => ((await dialogView(d).catch(() => null)) || {}).waiting === 1 || null, 15000);
  if (d) {
    await cdp.send('ServiceWorker.enable', {}, d.session);
    await cdp.send('ServiceWorker.stopAllWorkers', {}, d.session);
  }
  const gone = d && (await waitFor('the dialog notices', async () => {
    const v = await dialogView(d).catch(() => null);
    return v && v.close && /no longer waiting/.test(v.err) ? v : null;
  }, 15000));
  check('worker stopped while held: the dialog says the download is no longer waiting', !!gone, JSON.stringify(gone));
  const slow = await waitFor('both slow files in the browser folder', async () => {
    const f = browserFiles();
    return f.includes('a.bin') && f.includes('b.bin') ? f : null;
  }, 40000);
  check('worker stopped while held: the browser downloads both, intact',
    !!slow && Buffer.compare(fs.readFileSync(path.join(BROWSER_DL, 'a.bin')), SLOW['/slow/a']) === 0 && Buffer.compare(fs.readFileSync(path.join(BROWSER_DL, 'b.bin')), SLOW['/slow/b']) === 0,
    JSON.stringify(await browserDownloads()));
  if (d) await press(d, 'close').catch(() => {});
  await attachWorker();
  await forgetBrowserDownloads(['refuse', 'closed', 'a.bin', 'b.bin']);
}

/** What the dialog shows when it offers to refresh a waiting download's address. */
const offerView = (d) => d.run(`({ offer: !document.getElementById('offer').hidden && !document.getElementById('offerRefresh').disabled,
  name: document.getElementById('offerName').textContent, title: document.title, url: document.getElementById('url').textContent })`);
const offerShowing = (d, re) => waitFor('the refresh offer', async () => {
  const v = await offerView(d).catch(() => null);
  return v && v.offer && re.test(v.url) ? v : null;
}, 15000);

/** A download waiting in reIDM for a new address, as *Refresh download address* leaves it:
 *  added (checked, so its size is known) for later, then armed. Null when the service doesn't
 *  tell the extension about it (before 0.8), and then the checks are skipped. */
async function armRefresh(p) {
  let it = itemFor(await reidmItems(), p);
  if (!it) {
    await fetch(`${API}/ext/add`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ url: `${onRoot()}${p}`, verify: true, later: true }) });
    it = itemFor(await reidmItems(), p);
  }
  if (!it) return null;
  await fetch(`${API}/api/downloads/${it.id}/refresh-capture`, { method: 'POST', headers: { 'X-Reidm-Token': token(), 'Content-Type': 'application/json' }, body: '{}' });
  const info = await (await fetch(`${API}/ext/ping`)).json();
  if (!('refresh_armed' in info)) {
    console.log('skipped: service lacks refresh_armed');
    await disarmRefresh();
    return null;
  }
  return it;
}
const disarmRefresh = () => fetch(`${API}/api/refresh-capture/cancel`, { method: 'POST', headers: { 'X-Reidm-Token': token() } }).catch(() => {});

/** M14: while reIDM waits for a new address, a captured link first asks "Use this link to
 *  refresh …?". With the dialog on. */
async function refreshChecks() {
  await noDialog();
  const a = await armRefresh('/files/rfa.zip');
  if (!a) return;
  try {
    // Refresh: a click on a mirror of the same file becomes the waiting download's address.
    await click('#mirror');
    let d = await dialog();
    const v = d && (await offerShowing(d, /\/mirror\/rfa\.zip/));
    check('refresh armed: the dialog asks "Use this link to refresh rfa.zip?"', !!v && v.name === 'rfa.zip' && /^Refresh download address/.test(v.title), JSON.stringify(v));
    if (d) await press(d, 'offerRefresh');
    const moved = await waitFor('the waiting download on its new address', async () => {
      const i = (await reidmItems()).find((x) => x.id === a.id);
      return i && i.request.url.endsWith('/mirror/rfa.zip') ? i : null;
    });
    check('Refresh: the waiting download carries on from the new address, with no new download', !!moved && (await reidmItems()).filter((x) => /rfa\.zip$/.test(x.request.url)).length === 1);
    const r = (await recent())[0];
    check('Refresh: "Just now" says the address was refreshed', !!r && r.outcome === 'refreshed' && r.filename === 'rfa.zip', JSON.stringify(r));
    await noDialog();

    // New download: a download the browser started, which is another file, is added as its own;
    // the waiting download is left as it was.
    const b = await armRefresh('/files/rfb.zip');
    await click('#other');
    d = await dialog();
    const w = d && (await offerShowing(d, /\/dl\/other/));
    check('refresh armed, another file: the dialog offers the refresh first', !!w && w.name === 'rfb.zip', JSON.stringify(w));
    if (d) await press(d, 'offerNew');
    const form = d && (await dialogShowing(d, /\/dl\/other/));
    check('New download: the usual form follows, with the file’s own name', !!form && form.name === 'other.bin', JSON.stringify(form));
    if (d) await press(d, 'start');
    const added = await waitFor('other in reIDM', async () => itemFor(await reidmItems(), '/dl/other'));
    const still = (await reidmItems()).find((x) => x.id === b.id);
    check('New download: reIDM adds it, and the waiting download keeps its address', !!added && !!still && still.request.url.endsWith('/files/rfb.zip'), JSON.stringify(still && still.request.url));
    check('New download: the browser kept nothing', !(await browserDownloads()).some((x) => x.url.includes('/dl/other')));
    await deleteItem(added);
    await noDialog();
  } finally {
    await disarmRefresh();
  }
}

/** With the dialog off. */
async function quietChecks() {
  // M15: a program a page starts still asks first.
  await click('#setup');
  const d = await dialog();
  const v = d && (await dialogShowing(d, /setup\.exe/));
  check('a program asks even with the dialog off, and warns', !!v && v.program, JSON.stringify(v));
  if (d) await press(d, 'cancel');
  await noDialog();

  // M14 with the dialog off: the refresh is still offered (the user just armed it), and *New
  // download* then starts at once, with no form.
  const b = await armRefresh('/files/rfb.zip');
  if (b) {
    try {
      await click('#other2');
      const od = await dialog();
      const ov = od && (await offerShowing(od, /\/files\/other\.zip/));
      check('refresh armed, dialog off: the refresh is still offered', !!ov && ov.name === 'rfb.zip', JSON.stringify(ov));
      if (od) await press(od, 'offerNew');
      const added = await waitFor('other.zip in reIDM', async () => itemFor(await reidmItems(), '/files/other.zip'));
      check('refresh armed, dialog off: New download starts it at once', !!added && !!(await noDialog()));
      await deleteItem(added);
    } finally {
      await disarmRefresh();
    }
  }

  // H1 with the dialog off: straight to reIDM, best format.
  const opt = await extensionPage('options.html');
  await opt.run(`chrome.runtime.sendMessage({ type: 'download-video', page: '${onRoot()}/watch?v=best2', title: 'B' }).then(r => r.ok)`);
  const vid = await waitFor('the video in reIDM', async () => (await reidmItems()).find((i) => i.request.url.endsWith('/watch?v=best2')));
  check('best-quality video row, no dialog: reIDM gets yt-dlp’s best format', !!vid && /^bestvideo/.test((vid.request.media || {}).format || ''));
  await deleteItem(vid);

  // L1, M17: what a compromised page could make the content script say.
  const add = await inContentScript(`chrome.runtime.sendMessage({ type: 'add', details: { url: '${onRoot()}/files/q1.zip' } })`);
  check('a page cannot use the popup’s messages', !!add && add.ok === false, JSON.stringify(add));
  const spoof = await inContentScript(`chrome.runtime.sendMessage({ type: 'download-format', source: 'youtube', page: 'https://www.youtube.com/watch?v=spoofed', format: {} })`);
  check('a page that is not YouTube cannot ask for a YouTube download', !!spoof && spoof.ok === false && !(await reidmItems()).some((i) => i.request.url.includes('spoofed')), JSON.stringify(spoof));

  // M4: junk a page posts is not stored.
  await evalPage(`postMessage({ __reidm: true, items: [
    { url: 'javascript:alert(1)', kind: 'media' }, { url: '${onRoot()}/x.mp4', kind: 'mse' },
    { url: '${onRoot()}/' + 'a'.repeat(5000) + '.mp4', kind: 'media' }, { url: '${onRoot()}/real.mp4', kind: 'media' }] }, '*')`);
  await sleep(2500);
  const listed = await inWorker(`chrome.tabs.query({}).then(t => t.find(x => x.url === '${onRoot()}/').id).then(id => new Promise(r => chrome.storage.session.get('reidm-media:' + id).then(o => r((o['reidm-media:' + id] || []).map(e => e.url)))))`);
  check('page reports are checked: only the valid one is kept', listed.some((u) => u.endsWith('/real.mp4')) && !listed.some((u) => /javascript:|x\.mp4|aaaa/.test(u)), JSON.stringify(listed).slice(0, 300));

  // M6: an unchanged status is not written again (each write wakes every content script).
  const before = await inWorker('reidm.stats.statusWrites');
  await inWorker('reidm.ping().then(() => reidm.ping()).then(() => reidm.ping()).then(() => true)');
  check('an unchanged reIDM status is not written to storage again', (await inWorker('reidm.stats.statusWrites')) === before);

  // H5: the cookie store comes from the tab; incognito with no incognito tab means no cookies.
  const stores = await inWorker(`chrome.tabs.query({}).then(t => t.find(x => x.url === '${onRoot()}/').id).then(async id => [await reidm.cookieStoreFor(id, false), await reidm.cookieStoreFor(null, true)])`);
  check('cookies come from the tab’s own store, never the wrong profile’s', stores[0] === '0' && stores[1] === null, JSON.stringify(stores));
  await opt.close();
}

/** M19: a hand-off carries a `request_id`, and the retry after a timeout carries the same one,
 *  so reIDM can tell a repeat from a second download. Against a stand-in for reIDM whose first
 *  answer comes too late; it takes the extension's 25 s timeout to run. */
async function requestIdChecks() {
  const bodies = [];
  const stand = http.createServer((req, res) => {
    let b = '';
    req.on('data', (c) => (b += c));
    req.on('end', () => {
      if (!req.url.startsWith('/ext/add')) {
        res.writeHead(404);
        return res.end();
      }
      try {
        bodies.push(JSON.parse(b));
      } catch {
        bodies.push(null);
      }
      if (bodies.length === 1) return; // never answered: the extension gives up on it and asks again
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ ok: true, id: 1, outcome: 'existing' }));
    });
  });
  await new Promise((r) => stand.listen(0, '127.0.0.1', r));
  try {
    await inWorker(`chrome.storage.sync.set({ port: ${stand.address().port} }).then(() => new Promise(r => setTimeout(r, 300))).then(() => true)`);
    await inWorker(`(globalThis.__sent = null, reidm.send({ url: '${onRoot()}/files/m19.zip' }).then(r => (globalThis.__sent = r)), true)`);
    const r = await waitFor('the retried hand-off', () => inWorker('globalThis.__sent'), 45000);
    const ids = bodies.map((x) => x && x.request_id);
    check('a timed-out hand-off is asked again with the same request id', !!r && r.ok && ids.length === 2 && !!ids[0] && ids[0] === ids[1], JSON.stringify({ ids, r }));
  } finally {
    await inWorker('chrome.storage.sync.set({ port: 61002 }).then(() => new Promise(r => setTimeout(r, 300))).then(() => true)');
    stand.closeAllConnections();
    stand.close();
  }
}

/** Finding media, cheaply. */
async function mediaChecks() {
  // H4: a live stream polled every second is fetched by the extension at most once per address.
  const from = log.length;
  await openPage(`${onRoot()}/live`);
  const liveTarget = pageTarget;
  await sleep(8000);
  const mine = log.slice(from).filter((l) => l.path.startsWith('/hls/') && l.site === 'none').length;
  const theirs = log.slice(from).filter((l) => l.path.startsWith('/hls/') && l.site !== 'none').length;
  check('live stream: the extension adds at most one request per playlist', mine <= 2, `${mine} by the extension, ${theirs} by the page`);
  const liveTab = await inWorker(`chrome.tabs.query({}).then(t => t.find(x => x.url.endsWith('/live')).id)`);
  const entries = await inWorker(`reidm.ping().then(() => new Promise(r => chrome.storage.session.get('reidm-media:${liveTab}').then(o => r(o['reidm-media:${liveTab}'] || []))))`);
  const master = entries.find((e) => e.url.endsWith('/hls/master.m3u8'));
  check('live stream: its two qualities are listed', !!master && (master.formats || []).length === 2, JSON.stringify(master).slice(0, 200));

  // M11: the media count is the tab's own.
  const badges = await inWorker(`chrome.tabs.query({}).then(t => t.find(x => x.url === '${onRoot()}/').id).then(async root => {
    const count = async (id) => ((await chrome.storage.session.get('reidm-media:' + id))['reidm-media:' + id] || []).length;
    return { live: [await chrome.action.getBadgeText({ tabId: ${liveTab} }), await count(${liveTab})], root: [await chrome.action.getBadgeText({ tabId: root }), await count(root)] };
  })`);
  check('each tab’s badge shows its own media count', badges.live[0] === String(badges.live[1]) && badges.root[0] === String(badges.root[1]) && badges.live[0] !== badges.root[0], JSON.stringify(badges));
  // A playing stream keeps the network busy; the checks below count the worker's work.
  await cdp.send('Target.closeTarget', { targetId: liveTarget });

  // M22, M21: a relative manifest fetch is understood; after SPA navigation the new video's.
  await openPage(`${onRoot()}/spa/one`);
  const a = await hookAnswer();
  check('relative manifest fetch: the picker gets its three qualities', !!a && a.formats.length === 3 && a.formats.every((f) => /^http:\/\/127\.0\.0\.1:\d+\/spa\/hls\/v\d\.m3u8$/.test(f.url)), JSON.stringify(a && a.formats.map((f) => f.url)));
  // The content script resets its picker on the Navigation API's event, which must reach its
  // isolated world when the page itself calls pushState.
  await inContentScript(`window.navigation.addEventListener('navigatesuccess', () => (window.__navSeen = location.pathname)); true`);
  await evalPage('next()');
  await sleep(300);
  check('the content script sees the page’s SPA navigation', (await inContentScript('window.__navSeen')) === '/spa/two');
  const b = await hookAnswer();
  check('SPA navigation: the picker offers the new video, not the old one', !!b && b.formats.length === 1, JSON.stringify(b && b.formats.map((f) => f.url)));

  // H6: YouTube's global describes the first video only.
  await openPage(`${onRoot()}/yt?v=AAAAAAAAAAA`);
  const ya = await hookAnswer();
  check('YouTube: the first video’s formats and title', !!ya && ya.source === 'youtube' && ya.title === 'Video A' && ya.formats[0].itag === 18, JSON.stringify(ya));
  await evalPage('toB()');
  await sleep(300);
  const yb = await hookAnswer();
  check('YouTube after SPA navigation: the new video’s formats, title and page', !!yb && yb.title === 'Video B' && yb.formats.length === 1 && yb.formats[0].itag === 22 && /v=BBBBBBBBBBB$/.test(yb.page), JSON.stringify(yb));
  await evalPage('toC()');
  const yc = await hookAnswer();
  check('YouTube, a video not seen yet: no other video’s formats, "best" only', !!yc && yc.source === 'youtube' && yc.formats.length === 0 && yc.title === 'Video C' && /v=CCCCCCCCCCC$/.test(yc.page), JSON.stringify(yc));

  // M5: images never reach the network listeners.
  const sniffed = await inWorker('reidm.stats.sniffed');
  await openPage(`${onRoot()}/busy`);
  check('a page full of images costs the worker nothing', (await inWorker('reidm.stats.sniffed')) === sniffed, `${(await inWorker('reidm.stats.sniffed')) - sniffed} listener calls`);

  // M8, M9: with "Look for video and audio" off no page is hooked; inject.js is not reachable.
  check('the page hook runs before the page’s scripts', !(await evalPage('String(window.fetch).includes("[native code]")')));
  const probe = await evalPage(`fetch('chrome-extension://${EXT_ID}/inject.js').then(() => 'reachable', () => 'blocked')`);
  check('a page cannot detect reIDM by fetching its files', probe === 'blocked', probe);
  await inWorker('chrome.storage.sync.set({ sniffMedia: false }).then(() => new Promise(r => setTimeout(r, 300))).then(() => reidm.syncPageHook()).then(() => true)');
  await openPage(`${onRoot()}/busy`);
  check('media detection off: pages are not hooked', !!(await evalPage('String(window.fetch).includes("[native code]")')));
  await inWorker('chrome.storage.sync.set({ sniffMedia: true }).then(() => new Promise(r => setTimeout(r, 300))).then(() => reidm.syncPageHook()).then(() => true)');
  await openPage(`${onRoot()}/busy`);
  check('media detection back on: pages are hooked again', !(await evalPage('String(window.fetch).includes("[native code]")')));
}

async function main() {
  await new Promise((r) => site.listen(SITE_PORT, '127.0.0.1', r));
  await startDaemon();
  await launchChrome();
  const sw = await attachWorker();
  check('the extension service worker runs', !!sw, sw || 'no worker');
  check('the extension has the fixed ID', !!sw && sw.includes(EXT_ID));
  // Let the worker publish reIDM's status for the content scripts.
  await inWorker('reidm.ping().then(i => !!i)');
  // The service counts a check-in only from the extension itself (not a web page pinging it), by
  // what Chrome says about where the request came from: make sure the real extension still counts.
  const want = JSON.parse(fs.readFileSync(path.join(ROOT, 'extension', 'manifest.json'), 'utf8')).version;
  const seen = await waitFor('the service to see the extension', async () => {
    const r = await fetch(`${API}/api/status`, { headers: { 'X-Reidm-Token': token() } });
    const ext = (await r.json()).extension;
    return ext?.version === want ? ext : null;
  }, 10000);
  check('the service reports the extension as connected', !!seen, seen ? `version ${seen.version}` : `expected ${want}`);
  if (process.env.REIDM_ONLY_GRABBER) return grabberChecks();

  await openPage(`http://127.0.0.1:${SITE_PORT}/`);

  // 1. a file link, taken on click — after the dialog
  await click('#file');
  const dlg = await dialog();
  check('file link: the "Download file info" dialog opens', !!dlg);
  if (dlg) {
    const shown = await waitFor('the size in the dialog', async () => {
      const v = await dlg.run(`({ name: document.getElementById('name').value, size: document.getElementById('size').textContent })`);
      return v && !/checking/.test(v.size) ? v : null;
    });
    check('file link: the dialog shows the name and the size', shown && shown.name === 'app.zip' && shown.size === '3.00 MB', JSON.stringify(shown));
    if (process.env.REIDM_SHOT_DIR) {
      const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' }, dlg.session);
      fs.writeFileSync(path.join(process.env.REIDM_SHOT_DIR, 'download-dialog.png'), Buffer.from(data, 'base64'));
    }
    await press(dlg, 'start');
  }
  let it = await waitFor('app.zip in reIDM', async () => itemFor(await reidmItems(), '/files/app.zip'));
  check('file link: reIDM has it', !!it);
  it = await waitFor('app.zip done', async () => {
    const i = itemFor(await reidmItems(), '/files/app.zip');
    return i && i.status.state === 'Done' && i;
  });
  check('file link: download completes', !!it, it ? '' : JSON.stringify(itemFor(await reidmItems(), '/files/app.zip')?.status));
  check('file link: the browser never requested it', !browserRequested('/files/app.zip'));
  const shown = await inWorker('chrome.notifications.getAll().then(n => Object.keys(n))');
  check('file link: a notification says reIDM is downloading it', shown.some((k) => k.startsWith('reidm-')), JSON.stringify(shown));
  if (it) check('file link: bytes are right', Buffer.compare(fs.readFileSync(it.path), FILES['/files/app.zip']) === 0);

  // 1b. Cancel, for a download the browser started (known only from its headers)
  await click('#cancelled');
  const cdlg = await dialog();
  check('Cancel: the dialog opens for a download the browser started', !!cdlg);
  if (cdlg) await press(cdlg, 'cancel');
  await sleep(2500);
  check('Cancel: reIDM did not take it', !itemFor(await reidmItems(), '/dl/cancelled'));
  check('Cancel: the browser kept no download either', !(await inWorker('chrome.downloads.search({}).then(d => d.length)')) && !browserFiles().some((f) => f.startsWith('cancelled')));
  await dialogChecks();
  // The rest of the checks are about the hand-off itself: no dialog.
  await inWorker('chrome.storage.sync.set({ askBeforeDownload: false })');
  await sleep(300);
  await quietChecks();

  // 2. downloads only recognisable from their headers: taken from the browser
  for (const [sel, p] of [['#report', '/dl/report'], ['#tiny', '/dl/tiny']]) {
    await click(sel);
    for (const ms of [150, 400, 900]) {
      await sleep(ms === 150 ? 150 : ms - 150);
      shot(`${p.split('/').pop()}-${ms}ms`);
    }
    const got = await waitFor(`${p} done`, async () => {
      const i = itemFor(await reidmItems(), p);
      return i && i.status.state === 'Done' && i;
    });
    check(`${p}: reIDM downloaded it`, !!got);
    if (got) check(`${p}: bytes are right`, Buffer.compare(fs.readFileSync(got.path), FILES[p]) === 0);
    check(`${p}: the browser did start it (it is a header-only download)`, browserRequested(p));
  }
  await sleep(1500);
  const kept = await inWorker('chrome.downloads.search({}).then(d => d.map(x => x.url + " " + x.state))');
  check('the browser kept no download entries', kept.length === 0, kept.join(', '));
  check('the browser wrote no files', browserFiles().length === 0, browserFiles().join(', '));
  // With "Ask where to save each file" on, the checks below would stop at the browser's own
  // Save dialog; the point of that run is only that captured downloads never open one.
  if (process.env.REIDM_PROMPT) return;

  // 3. cookies
  await click('#private');
  const priv = await waitFor('secret done', async () => {
    const i = itemFor(await reidmItems(), '/private/secret.bin');
    return i && i.status.state === 'Done' && i;
  });
  check('cookies reach reIDM: the login-only file downloaded', !!priv);

  // 4. a POST stays with the browser
  await click('#post');
  const csv = await waitFor('export.csv in the browser folder', async () => browserFiles().find((f) => f.startsWith('export')));
  check('POST download: the browser kept it', !!csv);
  check('POST download: reIDM did not take it', !itemFor(await reidmItems(), '/post/export'));
  await openPage(`http://127.0.0.1:${SITE_PORT}/`);

  // 5. Alt+click
  await click('#alt', 1);
  await sleep(1200);
  shot('alt-click-browser-download');
  const altFile = await waitFor('alt.zip in the browser folder', async () => browserFiles().find((f) => f.startsWith('alt')));
  check('Alt+click: the browser downloaded it', !!altFile);
  check('Alt+click: reIDM did not take it', !itemFor(await reidmItems(), '/files/alt.zip'));

  // 6. a "file" link that is a web page
  await click('#fake');
  const opened = await waitFor('the browser opened the page', async () => (await evalPage('location.pathname')) === '/files/fake.zip');
  check('page posing as a file: the browser opened it', !!opened);
  check('page posing as a file: reIDM did not save it', !itemFor(await reidmItems(), '/files/fake.zip'));
  await openPage(`http://127.0.0.1:${SITE_PORT}/`);

  // 10. a magnet link: a torrent for reIDM (kept off the network: no DHT)
  await fetch(`${API}/api/settings`, { method: 'POST', headers: { 'X-Reidm-Token': token(), 'Content-Type': 'application/json' }, body: JSON.stringify({ torrent_dht: false, torrent_port: 0 }) });
  await click('#magnet');
  const tor = await waitFor('the torrent in reIDM', async () => (await reidmItems()).find((i) => i.request.url.startsWith('magnet:')));
  check('magnet link: reIDM has it as a torrent', !!(tor && tor.request.torrent));
  check('magnet link: named after the link', !!tor && /Test Torrent\.bin$/.test(tor.path), tor && tor.path);
  check('magnet link: the page stayed where it was', (await evalPage('location.pathname')) === '/');
  if (tor) await fetch(`${API}/api/downloads/${tor.id}`, { method: 'DELETE', headers: { 'X-Reidm-Token': token() } });

  await requestIdChecks();
  await mediaChecks();
  await openPage(`http://127.0.0.1:${SITE_PORT}/`);

  // 7. reIDM stopped; the native host starts it
  const hostDir = WIN ? WORK : path.join(PROFILE, 'NativeMessagingHosts');
  const hostFile = path.join(hostDir, 'com.zektopic.reidm.json');
  fs.mkdirSync(hostDir, { recursive: true });
  fs.writeFileSync(
    hostFile,
    JSON.stringify({ name: 'com.zektopic.reidm', description: 'test', type: 'stdio', path: path.join(BIN, `reidm-host${EXE}`), allowed_origins: [`chrome-extension://${EXT_ID}/`] }),
  );
  if (WIN) spawnSync('reg', ['add', HOST_KEY, '/ve', '/t', 'REG_SZ', '/d', hostFile, '/f']);
  await stopDaemon();
  await inWorker('reidm.ping().then(i => !!i)');
  await click('#launched');
  const launched = await waitFor('launched done', async () => {
    try {
      const i = itemFor(await reidmItems(), '/dl/launched');
      return i && i.status.state === 'Done' && i;
    } catch {
      return null;
    }
  }, 30000);
  check('reIDM not running: the extension started it and it took the download', !!launched);
  check('reIDM not running: the browser kept nothing', !browserFiles().some((f) => f.startsWith('launched')), browserFiles().join(', '));

  // the in-browser resource grabber
  await grabberChecks();
  await openPage(`http://127.0.0.1:${SITE_PORT}/`);

  // 8. reIDM stopped and no host: the browser keeps the download
  fs.rmSync(hostFile);
  unregisterHost();
  await stopDaemon();
  await inWorker('reidm.ping().then(i => !!i)');
  await click('#fallback');
  const fb = await waitFor('fallback in the browser folder', async () => browserFiles().find((f) => f.startsWith('fallback') && !f.endsWith('.crdownload')), 30000);
  check('reIDM unavailable: the browser downloaded it itself', !!fb);
  if (fb) check('reIDM unavailable: the browser’s file is intact', Buffer.compare(fs.readFileSync(path.join(BROWSER_DL, fb)), FILES['/dl/fallback']) === 0);
}

function unregisterHost() {
  if (WIN) spawnSync('reg', ['delete', HOST_KEY, '/f'], { stdio: 'ignore' });
}
/** Put back the registrations the test's service replaced. */
function restoreRegistry() {
  for (const [key, value] of savedReg) {
    if (value) spawnSync('reg', ['add', key, '/ve', '/t', 'REG_SZ', '/d', value, '/f'], { stdio: 'ignore' });
    else spawnSync('reg', ['delete', key, '/f'], { stdio: 'ignore' });
  }
}

let failed = false;
try {
  await main();
} catch (e) {
  console.error(e);
  failed = true;
} finally {
  try {
    await cdp?.send('Browser.close');
  } catch {
    /* already gone */
  }
  chrome?.kill('SIGTERM');
  await stopDaemon();
  restoreRegistry();
  site.close();
}
const bad = results.filter((r) => !r.ok);
console.log(`\n${results.length - bad.length}/${results.length} checks passed`);
if (!process.env.KEEP) fs.rmSync(WORK, { recursive: true, force: true });
else console.log(`kept ${WORK}`);
process.exit(failed || bad.length ? 1 : 0);
