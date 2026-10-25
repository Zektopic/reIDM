#!/usr/bin/env node
// Stress the reIDM service the way the window uses it, for the hang report: a large download
// while the list is polled, repeated bulk add + purge of playlist-sized batches, and a server
// that never answers (Pause and delete must still work promptly).
//
//   cargo build --release --locked -p reidm
//   node tools/stress-test.mjs [--big-mib 2048] [--batch 300] [--cycles 10] [--soak 10]
//                              [--profile none|potato|laptop]
//
// Linux only (reads /proc for memory). Throw-away service: own data folder, own HOME,
// ports 61201/61202, and a local site on 61290 — never the real reIDM.

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const opt = (n, d) => { const i = args.indexOf(`--${n}`); return i >= 0 ? args[i + 1] : d; };
const BIN = opt('bin', path.join(ROOT, 'target', 'release', 'reidm'));
const PROFILE = opt('profile', 'none');
// The same imitated machines profile-test uses: the service is capped, the site is not.
const LIMITS = {
  potato: ['-p', 'CPUQuota=50%', '-p', 'MemoryMax=256M', '-p', 'MemorySwapMax=0'],
  laptop: ['-p', 'CPUQuota=200%', '-p', 'MemoryMax=1G', '-p', 'MemorySwapMax=0'],
};
if (!fs.existsSync(BIN)) {
  console.error(`no release build at ${BIN}: run  cargo build --release --locked -p reidm`);
  process.exit(2);
}
const BIG_MIB = Number(opt('big-mib', 2048));
const BATCH = Number(opt('batch', 300));
const CYCLES = Number(opt('cycles', 10));
const SOAK = Number(opt('soak', 10));
const UI = 61202;
const LOCAL = 61201;
const SITE = 61290;
const MiB = 1024 * 1024;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const pct = (a, p) => a.length ? [...a].sort((x, y) => x - y)[Math.min(a.length - 1, Math.floor((a.length * p) / 100))] : 0;

const run = fs.mkdtempSync(path.join(os.tmpdir(), 'reidm-stress-'));
const home = path.join(run, 'home');
const data = path.join(run, 'data');
const dl = path.join(run, 'dl');
for (const d of [home, data, dl, path.join(home, '.config'), path.join(home, '.local/share')]) fs.mkdirSync(d, { recursive: true });
const big = path.join(run, 'big.bin');
// A sparse file: the kernel serves zeros, so the test measures reIDM (and the network), not
// this script's ability to generate gigabytes.
fs.closeSync(fs.openSync(big, 'w'));
fs.truncateSync(big, BIG_MIB * MiB);
const small = Buffer.alloc(32 * 1024, 0x5a);

// --- the site ---------------------------------------------------------------------------------
// Serves ranges of the sparse file at about 8 MiB/s per connection, so a 2 GiB download runs
// long enough to overlap the cycles; one path answers the 1-byte probe and then never answers.
let openSockets = new Set();
const site = http.createServer((req, res) => {
  const url = new URL(req.url, 'http://x');
  if (url.pathname === '/hang.bin') {
    if ((req.headers.range || '') === 'bytes=0-0') {
      res.writeHead(206, { 'Content-Range': `bytes 0-0/${64 * MiB}`, 'Content-Length': 1, 'Accept-Ranges': 'bytes' });
      res.end('x');
    } else {
      openSockets.add(res);
      res.on('close', () => openSockets.delete(res));
    }
    return;
  }
  if (url.pathname.startsWith('/small/')) {
    res.writeHead(200, { 'Content-Length': small.length });
    res.end(small);
    return;
  }
  if (url.pathname !== '/big.bin') { res.writeHead(404); res.end(); return; }
  const m = /bytes=(\d+)-(\d*)/.exec(req.headers.range || '');
  let start = m ? Number(m[1]) : 0;
  let end = m && m[2] ? Math.min(Number(m[2]), BIG_MIB * MiB - 1) : BIG_MIB * MiB - 1;
  const total = BIG_MIB * MiB;
  const headers = { 'Content-Length': end - start + 1, 'Accept-Ranges': 'bytes' };
  if (m) headers['Content-Range'] = `bytes ${start}-${end}/${total}`;
  res.writeHead(m ? 206 : 200, headers);
  const stream = fs.createReadStream(big, { start, end, highWaterMark: 256 * 1024 });
  let closed = false;
  res.on('close', () => { closed = true; stream.destroy(); });
  // Pace to ~8 MiB/s: 256 KiB every 32 ms.
  stream.on('data', (chunk) => {
    if (closed) return;
    stream.pause();
    res.write(chunk);
    // A quarter-MiB every 32 ms is ~8 MiB/s, far below what the socket buffers hold, so the
    // write's backpressure can be ignored here.
    setTimeout(() => { if (!closed) stream.resume(); }, 32);
  });
  stream.on('end', () => { if (!closed) res.end(); });
});

// --- the service ------------------------------------------------------------------------------
const env = {
  ...process.env,
  HOME: home,
  XDG_CONFIG_HOME: path.join(home, '.config'),
  XDG_DATA_HOME: path.join(home, '.local/share'),
  XDG_CACHE_HOME: path.join(home, '.cache'),
  REIDM_DATA_DIR: data,
  REIDM_UI_PORT: String(UI),
  REIDM_LOCAL_PORT: String(LOCAL),
  RUST_LOG: 'reidm=warn,reidm_core=warn',
};
const launch = [BIN, 'daemon', '--no-idm-port', '--dir', dl];
const [cmd, cmdArgs] = PROFILE === 'none'
  ? [BIN, launch.slice(1)]
  : ['systemd-run', ['--user', '--scope', '--quiet', '--collect', `--unit=reidm-stress-${process.pid}-${Date.now()}`, ...(LIMITS[PROFILE] ?? []), '--', ...launch]];
if (PROFILE !== 'none' && !LIMITS[PROFILE]) {
  console.error(`unknown profile ${PROFILE}: use potato, laptop or none`);
  process.exit(2);
}
const svc = spawn(cmd, cmdArgs, { env, stdio: ['ignore', 'ignore', 'pipe'] });
let svcErr = '';
svc.stderr.on('data', (d) => (svcErr += d));
let token = null;
async function up() { try { return (await fetch(`http://127.0.0.1:${UI}/ext/ping`)).ok; } catch { return false; } }
async function api(method, p, body, timeout = 30000) {
  const t0 = performance.now();
  const r = await fetch(`http://127.0.0.1:${UI}${p}`, {
    method,
    headers: { 'X-Reidm-Token': token, ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}) },
    body: body !== undefined ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(timeout),
  });
  const text = await r.text();
  const ms = performance.now() - t0;
  // 304 is the list saying "nothing changed", as the window's poll expects; an empty body.
  if (!r.ok && r.status !== 304) throw new Error(`${method} ${p}: ${r.status} ${text}`);
  return { ms, body: text ? JSON.parse(text) : null };
}
const rss = () => Number(/VmRSS:\s+(\d+) kB/.exec(fs.readFileSync(`/proc/${svc.pid}/status`, 'utf8'))[1]) / 1024;
const cpu = () => { const s = fs.readFileSync(`/proc/${svc.pid}/stat`, 'utf8').split(' '); return (Number(s[13]) + Number(s[14])) / 100; };

// The window's poll: rows since a cursor, 10 times a second.
class Poller {
  constructor() { this.times = []; this.errors = []; this.on = false; this.cursor = null; this.max = 0; }
  start() { this.on = true; this.loop(); }
  async loop() {
    while (this.on) {
      try {
        const q = this.cursor ? `?since=${encodeURIComponent(this.cursor)}` : '';
        const { ms, body } = await api('GET', `/api/downloads/rows${q}`, undefined, 5000);
        this.times.push(ms); this.max = Math.max(this.max, ms);
        if (body.cursor) this.cursor = body.cursor;
      } catch (e) { this.errors.push(String(e)); }
      await sleep(100);
    }
  }
  async stop() { this.on = false; await sleep(150); return this.stats(); }
  stats() {
    const t = this.times.slice(this.mark); this.mark = this.times.length;
    return { max: Math.max(0, ...t), p95: pct(t, 95), p50: pct(t, 50), n: t.length, errors: this.errors.length };
  }
}

const results = [];
const failures = [];
function record(phase, extra = {}) {
  results.push({ phase, ...extra });
  console.log(phase.padEnd(34), Object.entries(extra).map(([k, v]) => `${k}=${typeof v === 'number' ? v.toFixed(1) : v}`).join(' '));
}
function check(what, ok, why) { if (!ok) { failures.push(`${what}: ${why}`); console.log('FAIL', what, '-', why); } }

try {
  await new Promise((r) => site.listen(SITE, '127.0.0.1', r));
  for (let i = 0; i < 200; i++) { if (await up()) break; if (svc.exitCode !== null) throw new Error(`service exited: ${svcErr}`); await sleep(50); }
  token = fs.readFileSync(path.join(data, 'api-token'), 'utf8').trim();
  await api('POST', '/api/settings', { max_concurrent: 64, connections: 8 });

  // 1. A large download, polled like the window.
  const before = { rss: rss(), cpu: cpu() };
  const p1 = new Poller(); p1.start();
  const bigAdd = await api('POST', '/api/downloads', { url: `http://127.0.0.1:${SITE}/big.bin` });
  const bigId = bigAdd.body.id;
  await sleep(1500);
  const bigRunning = await api('GET', `/api/downloads/${bigId}`);
  record('big download', { addMs: bigAdd.ms, state: bigRunning.body.status.state, rssMB: rss() });
  check('the large download runs', ['Running', 'Queued'].includes(bigRunning.body.status.state), bigRunning.body.status.state);
  // Sustained polling while the large download checkpoints and saves, as the window does.
  await sleep(SOAK * 1000);
  const soakStats = p1.stats();
  record(`soak ${SOAK}s`, soakStats);
  check('no rows request over 2 s while soaking', soakStats.max < 2000, `${soakStats.max.toFixed(0)} ms`);

  // 2. Cycles of a playlist-sized batch: one batch add, wait for all done, one bulk purge,
  // while the large download is still running and the window keeps polling.
  let firstPurge = 0, lastPurge = 0, lastRss = 0;
  for (let c = 1; c <= CYCLES; c++) {
    const urls = Array.from({ length: BATCH }, (_, i) => `http://127.0.0.1:${SITE}/small/${c}-${i}.bin`);
    const add = await api('POST', '/api/downloads/batch', { urls });
    const ids = add.body.results.filter((r) => r.id).map((r) => r.id);
    // Wait until they are done (or failed), without blocking the poller.
    let left = ids.length;
    for (let i = 0; i < 300 && left > 0; i++) {
      await sleep(100);
      const list = await api('GET', '/api/downloads');
      const set = new Set(ids);
      left = list.body.filter((it) => set.has(it.id) && !['Done', 'Failed'].includes(it.status.state)).length;
    }
    check(`cycle ${c}: all small downloads finished`, left === 0, `${left} left`);
    const beforePurge = rss();
    const purge = await api('POST', '/api/downloads/bulk', { ids, action: 'purge' }, 120000);
    const afterPurge = rss();
    record(`cycle ${c}`, { batchMs: add.ms, purgeMs: purge.ms, left, rssBefore: beforePurge, rssAfter: afterPurge });
    if (c === 1) firstPurge = purge.ms;
    lastPurge = purge.ms; lastRss = afterPurge;
    check(`cycle ${c}: purge reports success`, purge.body.ok === ids.length, `${purge.body.ok}/${ids.length}`);
    check(`cycle ${c}: purge took under 30 s`, purge.ms < 30000, `${purge.ms.toFixed(0)} ms`);
  }
  const p1r = await p1.stop();
  record('polling under load', p1r);
  check('no rows request over 2 s', p1r.max < 2000, `${p1r.max.toFixed(0)} ms`);
  check('no failed rows request', p1r.errors === 0, `${p1r.errors} failed`);
  check('purge does not get slower each cycle', lastPurge < Math.max(5000, firstPurge * 3), `${firstPurge.toFixed(0)} -> ${lastPurge.toFixed(0)} ms`);
  record('memory', { rssStart: before.rss, rssNow: lastRss, growthMB: lastRss - before.rss });
  check('memory growth under 150 MB', lastRss - before.rss < 150, `${(lastRss - before.rss).toFixed(0)} MB`);

  // 3. The large download's own progress is not stalled by the cycles.
  const bigNow = await api('GET', `/api/downloads/${bigId}`);
  record('big download after cycles', { state: bigNow.body.status.state, doneMiB: bigNow.body.downloaded / MiB });
  check('the large download progressed', bigNow.body.downloaded > 64 * MiB, `${(bigNow.body.downloaded / MiB).toFixed(0)} MiB`);

  // 4. A server that never answers: Pause and delete must not wait for it.
  const hangAdd = await api('POST', '/api/downloads', { url: `http://127.0.0.1:${SITE}/hang.bin` });
  const hangId = hangAdd.body.id;
  await sleep(2000);
  const t0 = performance.now();
  await api('POST', `/api/downloads/${hangId}/pause`, undefined, 10000);
  const pauseMs = performance.now() - t0;
  let st = null, paused = false;
  for (let i = 0; i < 100; i++) {
    st = (await api('GET', `/api/downloads/${hangId}`)).body.status;
    if (st.state === 'Paused') { paused = true; break; }
    await sleep(50);
  }
  record('pause a stalled download', { pauseMs, state: st.state });
  check('pause answers at once', pauseMs < 3000, `${pauseMs.toFixed(0)} ms`);
  check('the stalled download is paused', paused, st.state);
  const t1 = performance.now();
  await api('DELETE', `/api/downloads/${hangId}?purge=true`, undefined, 10000);
  const delMs = performance.now() - t1;
  record('delete a stalled download', { deleteMs: delMs });
  check('delete answers at once', delMs < 3000, `${delMs.toFixed(0)} ms`);

  // 5. Clean up the large download too (it is still running).
  const t2 = performance.now();
  await api('DELETE', `/api/downloads/${bigId}?purge=true`, undefined, 30000);
  record('delete a running large download', { deleteMs: performance.now() - t2 });

  await api('POST', '/api/shutdown');
} catch (e) {
  failures.push(String(e));
  console.error('ERROR', e);
  console.error(svcErr.slice(-2000));
} finally {
  for (const s of openSockets) s.destroy();
  site.close();
  if (svc.exitCode === null) { svc.kill('SIGKILL'); }
  console.log(`\n${results.length} phases, ${failures.length} failure(s)`);
  for (const f of failures) console.log(' -', f);
  console.log(`work dir ${run}`);
  process.exit(failures.length ? 1 : 0);
}
