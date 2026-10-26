#!/usr/bin/env node
// How reIDM behaves on different machines: a low-end PC, a mid-range laptop and a desktop,
// imitated with cgroup limits on CPU and memory, measured on real transfers from a local server.
//
//   cargo build --release --locked -p reidm
//   node tools/profile-test.mjs [--profiles potato,laptop,desktop] [--big-mib 1024] [--json out.json]
//
// Linux only: each service runs in its own `systemd-run --user --scope`, whose cgroup both
// limits it and reports what it used (CPU time, peak memory). Nothing touches the real reIDM:
// the service gets its own data folder, its own ports (REIDM_UI_PORT) and a
// throw-away HOME, so it registers no browser helpers.
//
// For each profile:
//   big      one large file, downloaded and compared byte for byte with what was served
//   crash    a large file killed (SIGKILL) part way, the service started again, the download
//            finished and compared byte for byte — what a power cut does
//   small    many small files at once
//   list     a long list of waiting downloads: memory, idle CPU, and the cost of one full fetch
//
// Results are printed as a table, and written as JSON with --json.

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const BIN = path.join(ROOT, 'target', 'release', 'reidm');
const args = process.argv.slice(2);
const opt = (name, dflt) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : dflt;
};
const BIG_MIB = Number(opt('big-mib', 1024));
const CRASH_MIB = Number(opt('crash-mib', 512));
const SMALL_N = Number(opt('small-n', 300));
const SMALL_MIB = Number(opt('small-mib', 2));
const LIST_N = Number(opt('list-n', 20000));
const JSON_OUT = opt('json', null);

// What each imitated machine may use, and the performance preset a user of it would pick.
// The presets are reIDM's own (Options › Downloads › Performance); an older service without them
// gets the equivalent connections and downloads-at-once instead.
const PROFILES = {
  potato: { cpu: '50%', mem: '256M', preset: 'light', connections: 4, at_once: 2, note: 'half a core, 256 MB' },
  laptop: { cpu: '200%', mem: '1G', preset: 'balanced', connections: 8, at_once: 4, note: 'two cores, 1 GB' },
  desktop: { cpu: null, mem: null, preset: 'maximum', connections: 16, at_once: 8, note: 'no limits' },
};
const chosen = opt('profiles', 'potato,laptop,desktop').split(',');

if (os.platform() !== 'linux') {
  console.error('profile-test needs Linux (systemd-run --user --scope and cgroup v2)');
  process.exit(2);
}
if (!fs.existsSync(BIN)) {
  console.error(`no release build at ${BIN}: run  cargo build --release --locked -p reidm`);
  process.exit(2);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const MiB = 1024 * 1024;

// ── the server: deterministic content, ranges, generated on the fly ──────────────────────────
//
// The content of every file is a function of its name and the offset, so the server needs no
// disk or memory for it, and a downloaded file can be checked against it byte for byte.

const BLOCK = 64 * 1024;
const BASE = (() => {
  const b = Buffer.alloc(BLOCK); // not pooled: its own aligned ArrayBuffer
  let x = 0x9e3779b9;
  for (let i = 0; i < BLOCK; i++) {
    x ^= x << 13;
    x ^= x >>> 17;
    x ^= x << 5;
    b[i] = x & 0xff;
  }
  return b;
})();
const seedOf = (name) => [...name].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7);

// Bytes [start, end) of the file `name`: whole blocks, each the base block XOR a word that
// depends on the block's number, generated 32 bits at a time so the server keeps up with reIDM.
const BASE32 = new Uint32Array(BASE.buffer, BASE.byteOffset, BLOCK / 4);
function content(name, start, end) {
  const seed = seedOf(name);
  const b0 = Math.floor(start / BLOCK);
  const b1 = Math.ceil(end / BLOCK);
  const out = new Uint32Array(((b1 - b0) * BLOCK) / 4);
  for (let b = b0; b < b1; b++) {
    const k = (Math.imul(b, 2654435761) + seed) >>> 0;
    const o = ((b - b0) * BLOCK) / 4;
    for (let i = 0; i < BLOCK / 4; i++) out[o + i] = BASE32[i] ^ k;
  }
  return Buffer.from(out.buffer, start - b0 * BLOCK, end - start);
}

const site = http.createServer(async (req, res) => {
  const u = new URL(req.url, 'http://x');
  const m = /^\/f\/([\w.-]+)$/.exec(u.pathname);
  const size = Number(u.searchParams.get('size'));
  if (!m || !size) return res.writeHead(404).end();
  const name = m[1];
  let a = 0;
  let b = size - 1;
  const r = /bytes=(\d+)-(\d*)/.exec(req.headers.range || '');
  const head = { 'Content-Type': 'application/octet-stream', 'Accept-Ranges': 'bytes', ETag: `"${name}-${size}"` };
  if (r) {
    a = Number(r[1]);
    b = r[2] ? Math.min(Number(r[2]), size - 1) : size - 1;
    if (a >= size) return res.writeHead(416, { 'Content-Range': `bytes */${size}` }).end();
    res.writeHead(206, { ...head, 'Content-Range': `bytes ${a}-${b}/${size}`, 'Content-Length': b - a + 1 });
  } else {
    res.writeHead(200, { ...head, 'Content-Length': size });
  }
  if (req.method === 'HEAD') return res.end();
  // `rate` (bytes per second, per connection) imitates a server that limits each connection.
  const rate = Number(u.searchParams.get('rate')) || 0;
  const step = rate ? Math.min(MiB, Math.max(16 * 1024, Math.floor(rate / 10))) : MiB;
  let closed = false;
  let wake = null;
  res.on('close', () => {
    closed = true;
    wake?.();
  });
  res.on('drain', () => wake?.());
  const t0 = Date.now();
  for (let p = a; p <= b && !closed; p += step) {
    const chunk = content(name, p, Math.min(p + step, b + 1));
    if (!res.write(chunk)) await new Promise((ok) => (wake = ok));
    if (rate) {
      const due = t0 + ((p + step - a) / rate) * 1000;
      if (due > Date.now()) await sleep(due - Date.now());
    }
  }
  res.end();
});
await new Promise((ok) => site.listen(0, '127.0.0.1', ok));
const SITE = `http://127.0.0.1:${site.address().port}`;
const fileUrl = (name, bytes, rate = 0) => `${SITE}/f/${name}?size=${bytes}${rate ? `&rate=${rate}` : ''}`;

// Compare a downloaded file with what the server sent; the first difference, or null.
async function firstDifference(file, name, bytes) {
  const st = fs.statSync(file);
  if (st.size !== bytes) return `size ${st.size}, expected ${bytes}`;
  const fd = fs.openSync(file, 'r');
  const buf = Buffer.allocUnsafe(4 * MiB);
  try {
    for (let p = 0; p < bytes; p += buf.length) {
      const n = fs.readSync(fd, buf, 0, Math.min(buf.length, bytes - p), p);
      const want = content(name, p, p + n);
      if (!buf.subarray(0, n).equals(want)) {
        for (let i = 0; i < n; i++) if (buf[i] !== want[i]) return `differs at byte ${p + i}`;
      }
    }
  } finally {
    fs.closeSync(fd);
  }
  return null;
}

// ── a service in a limited scope ─────────────────────────────────────────────────────────────

let portBase = 61300;
class Service {
  constructor(profile, work) {
    this.profile = profile;
    this.work = work;
    this.data = path.join(work, 'data');
    this.dl = path.join(work, 'dl');
    this.home = path.join(work, 'home');
    for (const d of [this.data, this.dl, this.home]) fs.mkdirSync(d, { recursive: true });
    this.ui = portBase++;
    this.local = portBase++;
    this.unit = `reidm-profile-${process.pid}-${this.ui}`;
  }

  async start() {
    const p = PROFILES[this.profile];
    const props = [];
    if (p.cpu) props.push('-p', `CPUQuota=${p.cpu}`);
    if (p.mem) props.push('-p', `MemoryMax=${p.mem}`, '-p', 'MemorySwapMax=0');
    const env = {
      ...process.env,
      HOME: this.home,
      XDG_CONFIG_HOME: path.join(this.home, '.config'),
      XDG_DATA_HOME: path.join(this.home, '.local/share'),
      XDG_CACHE_HOME: path.join(this.home, '.cache'),
      REIDM_DATA_DIR: this.data,
      REIDM_UI_PORT: String(this.ui),
      RUST_LOG: 'reidm=warn,reidm_core=warn',
    };
    // The unit name is reused after a crash, so the cgroup's counters start again with the
    // restarted service; `cpu` and `peak` are therefore per run of the service.
    this.proc = spawn('systemd-run', ['--user', '--scope', '--quiet', '--collect', `--unit=${this.unit}-${Date.now()}`, ...props, '--', BIN, 'daemon', '--dir', this.dl], {
      env,
      stdio: ['ignore', 'ignore', 'pipe'],
    });
    this.stderr = '';
    this.proc.stderr.on('data', (d) => (this.stderr += d));
    for (let i = 0; i < 200; i++) {
      if (await this.up()) break;
      if (this.proc.exitCode !== null) throw new Error(`the service exited: ${this.stderr}`);
      await sleep(50);
    }
    this.token = fs.readFileSync(path.join(this.data, 'api-token'), 'utf8').trim();
    const cg = fs.readFileSync(`/proc/${this.proc.pid}/cgroup`, 'utf8').trim().split(':').pop();
    this.cgroup = path.join('/sys/fs/cgroup', cg);
  }

  async up() {
    try {
      return (await fetch(`http://127.0.0.1:${this.ui}/ext/ping`)).ok;
    } catch {
      return false;
    }
  }

  // CPU seconds used by the service since it started, and its memory in MiB: `rss` and
  // `rss_peak` are the process's own (what a user sees in a task manager); `cgroup_peak`
  // also counts the page cache of the files it wrote, which the kernel charges to it.
  usage() {
    const stat = fs.readFileSync(path.join(this.cgroup, 'cpu.stat'), 'utf8');
    const usec = Number(/usage_usec (\d+)/.exec(stat)[1]);
    const status = fs.readFileSync(`/proc/${this.proc.pid}/status`, 'utf8');
    const kib = (k) => Number(new RegExp(`${k}:\\s+(\\d+) kB`).exec(status)[1]) / 1024;
    const peak = Number(fs.readFileSync(path.join(this.cgroup, 'memory.peak'), 'utf8')) / MiB;
    return { cpu: usec / 1e6, rss: kib('VmRSS'), rss_peak: kib('VmHWM'), cgroup_peak: peak };
  }

  async api(method, p, body) {
    const r = await fetch(`http://127.0.0.1:${this.ui}${p}`, {
      method,
      headers: { 'X-Reidm-Token': this.token, ...(body ? { 'Content-Type': 'application/json' } : {}) },
      body: body ? JSON.stringify(body) : undefined,
    });
    const text = await r.text();
    if (!r.ok) throw new Error(`${method} ${p}: ${r.status} ${text}`);
    return text ? JSON.parse(text) : null;
  }

  async kill(signal = 'SIGKILL') {
    const done = new Promise((ok) => (this.proc.exitCode !== null ? ok() : this.proc.once('exit', ok)));
    this.proc.kill(signal);
    await done;
    // The scope can linger for a moment; the ports must be free before the next start.
    for (let i = 0; i < 100 && (await this.up()); i++) await sleep(50);
  }

  async stop() {
    try {
      await this.api('POST', '/api/shutdown');
    } catch {
      /* already gone */
    }
    await Promise.race([new Promise((ok) => (this.proc.exitCode !== null ? ok() : this.proc.once('exit', ok))), sleep(10000)]);
    if (this.proc.exitCode === null) await this.kill();
  }
}

async function configure(svc) {
  const p = PROFILES[svc.profile];
  const current = await svc.api('GET', '/api/settings');
  const patch = 'performance' in current ? { performance: p.preset } : { connections: p.connections, max_concurrent: p.at_once };
  await svc.api('POST', '/api/settings', patch);
  return 'performance' in current ? `preset ${p.preset}` : `${p.connections} connections, ${p.at_once} at once`;
}

async function waitDone(svc, ids, timeoutS) {
  const end = Date.now() + timeoutS * 1000;
  const want = new Set(ids);
  while (Date.now() < end) {
    const items = await svc.api('GET', '/api/downloads');
    const mine = items.filter((i) => want.has(i.id));
    const failed = mine.find((i) => i.status.state === 'Failed');
    if (failed) throw new Error(`download ${failed.id} failed: ${failed.status.error}`);
    if (mine.length === want.size && mine.every((i) => i.status.state === 'Done')) return mine;
    await sleep(250);
  }
  throw new Error(`not finished after ${timeoutS} s`);
}

// ── the scenarios ────────────────────────────────────────────────────────────────────────────

async function big(svc) {
  const bytes = BIG_MIB * MiB;
  const before = svc.usage();
  const t0 = Date.now();
  const { id } = await svc.api('POST', '/api/downloads', { url: fileUrl('big.bin', bytes) });
  const [item] = await waitDone(svc, [id], 3600);
  const secs = (Date.now() - t0) / 1000;
  const u = svc.usage();
  const diff = await firstDifference(item.path, 'big.bin', bytes);
  if (diff) throw new Error(`big: the file is not what was served: ${diff}`);
  return { mib_s: BIG_MIB / secs, secs, cpu_s_per_gib: ((u.cpu - before.cpu) / BIG_MIB) * 1024, peak_mib: u.rss_peak, cgroup_peak_mib: u.cgroup_peak, intact: true };
}

async function crash(svc) {
  const bytes = CRASH_MIB * MiB;
  const name = 'crash.bin';
  // Each connection is held to 4 MiB/s, so the download lasts long enough for reIDM to record
  // its progress a few times before the kill: what is measured is what a crash costs, in
  // seconds of transfer, not how fast loopback is.
  const { id } = await svc.api('POST', '/api/downloads', { url: fileUrl(name, bytes, 4 * MiB) });
  const t0 = Date.now();
  // Kill it when a little under half has arrived, without warning, as a power cut would.
  let at = 0;
  for (let i = 0; i < 30000; i++) {
    const it = await svc.api('GET', `/api/downloads/${id}`);
    if (it.downloaded >= bytes * 0.45) {
      at = it.downloaded;
      break;
    }
    if (it.status.state === 'Done') throw new Error('crash: finished before it could be interrupted');
    await sleep(20);
  }
  const rate = at / ((Date.now() - t0) / 1000);
  await svc.kill('SIGKILL');
  await svc.start();
  let it = await svc.api('GET', `/api/downloads/${id}`);
  const kept = it.downloaded;
  if (!['Running', 'Queued', 'Done'].includes(it.status.state)) await svc.api('POST', `/api/downloads/${id}/resume`);
  [it] = await waitDone(svc, [id], 3600);
  const diff = await firstDifference(it.path, name, bytes);
  if (diff) throw new Error(`crash: the resumed file is not what was served: ${diff}`);
  return { killed_at_mib: at / MiB, kept_mib: kept / MiB, lost_mib: (at - kept) / MiB, lost_s: (at - kept) / rate, mib_s: rate / MiB, intact: true };
}

async function small(svc) {
  const bytes = SMALL_MIB * MiB;
  const urls = Array.from({ length: SMALL_N }, (_, i) => fileUrl(`s${i}.bin`, bytes));
  const t0 = Date.now();
  const before = svc.usage();
  const { results } = await svc.api('POST', '/api/downloads/batch', { urls });
  const ids = results.map((r) => r.id);
  const items = await waitDone(svc, ids, 3600);
  const secs = (Date.now() - t0) / 1000;
  const u = svc.usage();
  // Spot-check a few of them byte for byte (all of them would mostly measure this script).
  for (const it of items.filter((_, i) => i % 50 === 0)) {
    const n = /\/f\/([\w.-]+)/.exec(it.request.url)[1];
    const diff = await firstDifference(it.path, n, bytes);
    if (diff) throw new Error(`small: ${n} is not what was served: ${diff}`);
  }
  return { files_s: SMALL_N / secs, secs, cpu_s: u.cpu - before.cpu, peak_mib: u.rss_peak, cgroup_peak_mib: u.cgroup_peak };
}

async function list(svc) {
  const urls = Array.from({ length: LIST_N }, (_, i) => fileUrl(`l${i}.bin`, MiB));
  const t0 = Date.now();
  for (let i = 0; i < urls.length; i += 5000) await svc.api('POST', '/api/downloads/batch', { urls: urls.slice(i, i + 5000), later: true });
  const addSecs = (Date.now() - t0) / 1000;
  await sleep(3000); // let the debounced save land
  const idle0 = svc.usage();
  await sleep(15000);
  const idle1 = svc.usage();
  const f0 = Date.now();
  const r = await fetch(`http://127.0.0.1:${svc.ui}/api/downloads/rows`, { headers: { 'X-Reidm-Token': svc.token } });
  const rowsBytes = (await r.arrayBuffer()).byteLength;
  const fetchMs = Date.now() - f0;
  return { add_s: addSecs, idle_cpu_pct: ((idle1.cpu - idle0.cpu) / 15) * 100, mem_mib: idle1.rss, rows_mib: rowsBytes / MiB, rows_ms: fetchMs };
}

// ── run ──────────────────────────────────────────────────────────────────────────────────────

const WORK = fs.mkdtempSync(path.join(os.tmpdir(), 'reidm-profile-'));
const report = { when: new Date().toISOString(), host: { cpus: os.cpus().length, mem_gib: os.totalmem() / 2 ** 30 }, sizes: { BIG_MIB, CRASH_MIB, SMALL_N, SMALL_MIB, LIST_N }, profiles: {} };
let failed = false;
try {
  for (const name of chosen) {
    if (!PROFILES[name]) throw new Error(`no profile ${name} (have ${Object.keys(PROFILES).join(', ')})`);
    const svc = new Service(name, path.join(WORK, name));
    const r = { machine: PROFILES[name].note };
    try {
      await svc.start();
      r.settings = await configure(svc);
      for (const [label, f] of [['big', big], ['crash', crash], ['small', small], ['list', list]]) {
        process.stderr.write(`${name}: ${label}…\n`);
        try {
          r[label] = await f(svc);
        } catch (e) {
          r[label] = { error: String(e.message || e) };
          failed = true;
        }
      }
    } finally {
      await svc.stop();
      spawnSync('systemctl', ['--user', 'stop', `${svc.unit}-*`], { stdio: 'ignore' });
    }
    report.profiles[name] = r;
  }
} finally {
  site.close();
  if (!process.env.KEEP) fs.rmSync(WORK, { recursive: true, force: true });
}

const f1 = (v) => (v === undefined ? '—' : typeof v === 'number' ? v.toFixed(v >= 100 ? 0 : 1) : String(v));
const cell = (r, k) => (r?.error ? `**failed**: ${r.error}` : f1(r?.[k]));
console.log(`\nreIDM on imitated machines (host: ${report.host.cpus} CPUs, ${report.host.mem_gib.toFixed(0)} GiB)\n`);
console.log('| | ' + chosen.map((n) => `${n} (${PROFILES[n].note})`).join(' | ') + ' |');
console.log('|---|' + chosen.map(() => '---').join('|') + '|');
const rows = [
  ['settings', (r) => r.settings],
  [`big ${BIG_MIB} MiB: MiB/s`, (r) => cell(r.big, 'mib_s')],
  ['big: CPU s per GiB', (r) => cell(r.big, 'cpu_s_per_gib')],
  ['big: peak memory (resident) MiB', (r) => cell(r.big, 'peak_mib')],
  [`crash ${CRASH_MIB} MiB: transfer lost to SIGKILL, seconds`, (r) => cell(r.crash, 'lost_s')],
  ['crash: file intact after resume', (r) => cell(r.crash, 'intact')],
  [`small ${SMALL_N}×${SMALL_MIB} MiB: files/s`, (r) => cell(r.small, 'files_s')],
  ['small: peak memory (resident) MiB', (r) => cell(r.small, 'peak_mib')],
  [`list ${LIST_N}: seconds to add`, (r) => cell(r.list, 'add_s')],
  ['list: idle CPU %', (r) => cell(r.list, 'idle_cpu_pct')],
  ['list: memory (resident) MiB', (r) => cell(r.list, 'mem_mib')],
  ['list: one full rows fetch, ms', (r) => cell(r.list, 'rows_ms')],
];
for (const [label, f] of rows) console.log(`| ${label} | ` + chosen.map((n) => f(report.profiles[n] || {})).join(' | ') + ' |');
if (JSON_OUT) fs.writeFileSync(JSON_OUT, JSON.stringify(report, null, 2));
process.exit(failed ? 1 : 0);
