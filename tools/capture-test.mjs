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
//  10. a magnet link goes to reIDM as a torrent, and the browser does nothing with it.
//
// REIDM_PROMPT=1 turns on the browser's "Ask where to save each file" and runs checks 1–2 only;
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
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, all) => (a.startsWith('--') ? [...acc, [a.slice(2), all[i + 1]]] : acc), []),
);
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const WIN = process.platform === 'win32';
const EXE = WIN ? '.exe' : '';
const CHROME = args.chrome || process.env.CHROME_PATH || path.join(os.homedir(), '.cache/ms-playwright/chromium-1243/chrome-linux64/chrome');
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
};
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
<a id="magnet" href="magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Test%20Torrent.bin">a magnet link</a>
<form method="post" action="/post/export"><input type="hidden" name="q" value="1">
<button id="post" type="submit">POST export</button></form>
</body></html>`;

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
  log.push({ method: req.method, path: u.pathname, range: req.headers.range, cookie: req.headers.cookie || '', mode: req.headers['sec-fetch-mode'] || '' });
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
  // A daemon the native host started is not our child; stop whatever holds the port.
  for (let i = 0; i < 50 && (await apiUp()); i++) {
    if (WIN) {
      const pids = new Set(spawnSync('netstat', ['-ano', '-p', 'TCP'], { encoding: 'utf8' }).stdout.split('\n')
        .filter((l) => /127\.0\.0\.1:61002\s.*LISTENING/.test(l)).map((l) => l.trim().split(/\s+/).pop()));
      for (const pid of pids) spawnSync('taskkill', ['/F', '/PID', pid]);
    } else {
      spawnSync('fuser', ['-k', '-TERM', '61002/tcp']);
    }
    await sleep(100);
  }
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
  send(method, params = {}, sessionId) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params, sessionId }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }
}

let chrome = null;
let cdp = null;
let page = null; // session id
let worker = null; // session id

async function launchChrome() {
  fs.writeFileSync(
    path.join(PROFILE, 'Default', 'Preferences'),
    JSON.stringify({
      download: { default_directory: BROWSER_DL, prompt_for_download: !!process.env.REIDM_PROMPT, directory_upgrade: true },
      savefile: { default_directory: BROWSER_DL },
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
  let version;
  for (let i = 0; i < 100 && !version; i++) {
    try {
      version = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`)).json();
    } catch {
      await sleep(100);
    }
  }
  if (!version) throw new Error('Chromium did not start');
  cdp = await Cdp.connect(version.webSocketDebuggerUrl);
  console.log(`browser: ${version.Browser}`);
}

async function attachWorker() {
  for (let i = 0; i < 100; i++) {
    const { targetInfos } = await cdp.send('Target.getTargets');
    const sw = targetInfos.find((t) => t.type === 'service_worker' && t.url.startsWith(`chrome-extension://${EXT_ID}/`));
    if (sw) {
      const { sessionId } = await cdp.send('Target.attachToTarget', { targetId: sw.targetId, flatten: true });
      worker = sessionId;
      return sw.url;
    }
    await sleep(100);
  }
  return null;
}

async function inWorker(expression) {
  // The worker may have been stopped and restarted since we attached.
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      const r = await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, worker);
      if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
      return r.result.value;
    } catch (e) {
      if (attempt) throw e;
      await attachWorker();
    }
  }
}

async function openPage(url) {
  const { targetId } = await cdp.send('Target.createTarget', { url });
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

async function main() {
  await new Promise((r) => site.listen(SITE_PORT, '127.0.0.1', r));
  await startDaemon();
  await launchChrome();
  const sw = await attachWorker();
  check('the extension service worker runs', !!sw, sw || 'no worker');
  check('the extension has the fixed ID', !!sw && sw.includes(EXT_ID));
  // Let the worker publish reIDM's status for the content scripts.
  await inWorker('reidm.ping().then(i => !!i)');
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
    await dlg.run(`document.getElementById('start').click()`);
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
  if (cdlg) await cdlg.run(`document.getElementById('cancel').click()`);
  await sleep(2500);
  check('Cancel: reIDM did not take it', !itemFor(await reidmItems(), '/dl/cancelled'));
  check('Cancel: the browser kept no download either', !(await inWorker('chrome.downloads.search({}).then(d => d.length)')) && !browserFiles().some((f) => f.startsWith('cancelled')));
  // The rest of the checks are about the hand-off itself: no dialog.
  await inWorker('chrome.storage.sync.set({ askBeforeDownload: false })');
  await sleep(300);

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
