#!/usr/bin/env node
// Live permutations against a throw-away reIDM service: the API and CLI surfaces that the unit
// tests exercise one layer down, here through HTTP and the real binaries.
//
//   cargo build --release --locked -p reidm
//   node tools/api-test.mjs
//
// Own ports (61401/61402), site (61490), proxy (61491), data folder and HOME. Linux.

import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const BIN = path.join(ROOT, 'target', 'release', 'reidm');
const UI = 61402;
const LOCAL = 61401;
const SITE = 61490;
const PROXY = 61491;
const MiB = 1024 * 1024;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const run = fs.mkdtempSync(path.join(os.tmpdir(), 'reidm-perms-'));
const home = path.join(run, 'home');
const data = path.join(run, 'data');
const dl = path.join(run, 'dl');
for (const d of [home, data, dl, path.join(home, '.config'), path.join(home, '.local/share')]) fs.mkdirSync(d, { recursive: true });

const F = Buffer.alloc(MiB / 2);
for (let i = 0; i < F.length; i++) F[i] = (i * 31 + 7) & 0xff;
const SLOW = Buffer.alloc(3 * MiB);
for (let i = 0; i < SLOW.length; i++) SLOW[i] = (i * 17 + 3) & 0xff;
const GOOD = crypto.createHash('sha256').update(F).digest('hex');

function serve(buf, req, res, pace) {
  const m = /bytes=(\d+)-(\d*)/.exec(req.headers.range || '');
  const start = m ? Number(m[1]) : 0;
  const end = m && m[2] ? Math.min(Number(m[2]), buf.length - 1) : buf.length - 1;
  const head = { 'Content-Length': end - start + 1, 'Accept-Ranges': 'bytes' };
  if (m) head['Content-Range'] = `bytes ${start}-${end}/${buf.length}`;
  res.writeHead(m ? 206 : 200, head);
  res.on('error', () => {});
  if (!pace) { res.end(buf.subarray(start, end + 1)); return; }
  let at = start;
  const step = () => {
    if (res.destroyed || res.writableEnded) return;
    if (at > end) { res.end(); return; }
    const to = Math.min(at + 64 * 1024, end + 1);
    res.write(buf.subarray(at, to));
    at = to;
    setTimeout(step, 64);
  };
  step();
}

const site = http.createServer((req, res) => {
  const u = new URL(req.url, 'http://x');
  if (u.pathname === '/f.bin') return serve(F, req, res, false);
  if (u.pathname === '/slow.bin') return serve(SLOW, req, res, true);
  res.writeHead(404); res.end();
});

let proxyHits = 0;
const proxy = http.createServer((req, res) => {
  proxyHits++;
  let u;
  try { u = new URL(req.url); } catch { res.writeHead(400); res.end(); return; }
  const up = http.request({ hostname: u.hostname, port: u.port, path: u.pathname + u.search, method: req.method, headers: req.headers }, (r) => {
    res.writeHead(r.statusCode, r.headers);
    r.pipe(res);
  });
  up.on('error', () => { try { res.writeHead(502); res.end(); } catch {} });
  req.pipe(up);
});

const env = {
  ...process.env,
  HOME: home,
  XDG_CONFIG_HOME: path.join(home, '.config'),
  XDG_DATA_HOME: path.join(home, '.local/share'),
  XDG_CACHE_HOME: path.join(home, '.cache'),
  REIDM_DATA_DIR: data,
  REIDM_UI_PORT: String(UI),
  RUST_LOG: 'reidm=warn,reidm_core=warn',
};
const svc = spawn(BIN, ['daemon', '--dir', dl], { env, stdio: ['ignore', 'ignore', 'pipe'] });
let svcErr = '';
svc.stderr.on('data', (d) => (svcErr += d));
let token = null;
const failures = [];
const check = (what, ok, why = '') => { console.log(ok ? 'PASS' : 'FAIL', what, ok ? '' : why); if (!ok) failures.push(`${what}: ${why}`); };
async function api(method, p, body) {
  const r = await fetch(`http://127.0.0.1:${UI}${p}`, {
    method,
    headers: { 'X-Reidm-Token': token, ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}) },
    body: body !== undefined ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(30000),
  });
  const text = await r.text();
  return { status: r.status, ok: r.ok, body: text ? (() => { try { return JSON.parse(text); } catch { return text; } })() : null };
}
const add = async (b) => { const r = await api('POST', '/api/downloads', b); if (r.status >= 300) throw new Error(`add ${JSON.stringify(b)}: ${r.status} ${JSON.stringify(r.body)}`); return r.body.id; };
const item = async (id) => (await api('GET', `/api/downloads/${id}`)).body;
async function until(what, fn, ms = 20000) {
  const end = Date.now() + ms;
  while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(50); }
  throw new Error(`timed out: ${what}`);
}
async function done(id, ms = 30000) {
  return until(`#${id} done`, async () => { const it = await item(id); return it && ['Done', 'Failed'].includes(it.status.state) ? it : null; }, ms);
}
const cli = (args) => new Promise((resolve) => {
  const p = spawn(BIN, args, { env });
  let out = '';
  p.stdout.on('data', (d) => (out += d));
  p.stderr.on('data', (d) => (out += d));
  p.on('exit', (code) => resolve({ code, out }));
});

try {
  await new Promise((r) => site.listen(SITE, '127.0.0.1', r));
  await new Promise((r) => proxy.listen(PROXY, '127.0.0.1', r));
  for (let i = 0; i < 200; i++) { try { if ((await fetch(`http://127.0.0.1:${UI}/ext/ping`)).ok) break; } catch {} await sleep(50); }
  token = fs.readFileSync(path.join(data, 'api-token'), 'utf8').trim();
  await api('POST', '/api/settings', { min_free_space_mib: 0, max_concurrent: 1, connections: 2 });

  // 1. A plain download, through the API.
  const a = await add({ url: `http://127.0.0.1:${SITE}/f.bin` });
  const ia = await done(a);
  check('plain download finishes', ia.status.state === 'Done', JSON.stringify(ia.status));
  check('plain download is byte-identical', fs.readFileSync(ia.path).equals(F));

  // 2. Checksums: the right one passes and is recorded; a wrong one fails and keeps the file.
  const g = await add({ url: `http://127.0.0.1:${SITE}/f.bin?good`, checksum: { algo: 'sha256', value: GOOD } });
  const ig = await done(g);
  check('matching checksum: Done', ig.status.state === 'Done' && ig.checksum_ok === true, JSON.stringify([ig.status, ig.checksum_ok]));
  const b = await add({ url: `http://127.0.0.1:${SITE}/f.bin?bad`, checksum: { algo: 'sha256', value: '0'.repeat(64) } });
  const ib = await done(b);
  check('wrong checksum: Failed with the reason', ib.status.state === 'Failed' && /checksum mismatch/.test(ib.status.error || ''), JSON.stringify(ib.status));
  check('wrong checksum: the file is kept', fs.existsSync(ib.path));

  // 3. Proxy: the HTTP client goes through it, and a proxy that cannot work is refused.
  let r = await api('POST', '/api/settings', { proxy: { mode: 'manual', kind: 'http', host: '127.0.0.1', port: PROXY, bypass: [] } });
  check('proxy settings accepted', r.status < 300, `${r.status} ${JSON.stringify(r.body)}`);
  proxyHits = 0;
  const p = await add({ url: `http://127.0.0.1:${SITE}/f.bin?proxy` });
  const ip = await done(p);
  check('proxied download finishes', ip.status.state === 'Done', JSON.stringify(ip.status));
  check('proxied download is byte-identical', fs.readFileSync(ip.path).equals(F));
  check('the proxy saw the request', proxyHits > 0, `${proxyHits} hits`);
  r = await api('POST', '/api/settings', { proxy: { mode: 'manual', kind: 'http', host: '', port: 0, bypass: [] } });
  check('an unusable proxy is refused with a reason', r.status === 400 && String(r.body).length > 10, `${r.status} ${JSON.stringify(r.body)}`);
  await api('POST', '/api/settings', { proxy: { mode: 'none' } });

  // 4. Queue order: three later downloads, the last moved to the top, and it starts first.
  const q = [];
  for (let i = 0; i < 3; i++) q.push(await add({ url: `http://127.0.0.1:${SITE}/slow.bin?q${i}`, later: true }));
  const orders = (await Promise.all(q.map(item))).map((it) => it.order);
  check('queued downloads have a place', orders.every((o) => typeof o === 'number'), JSON.stringify(orders));
  r = await api('POST', `/api/downloads/${q[2]}/move`, { to: 'top' });
  check('move to top accepted', r.status < 300, `${r.status}`);
  check('the moved download is first in the queue', (await item(q[2])).order < Math.min(...orders), JSON.stringify((await item(q[2])).order));
  r = await api('POST', '/api/queue/start');
  check('queue start reports a count', r.status < 300 && typeof r.body.count === 'number', JSON.stringify(r.body));
  const running = await until('one queued download starts', async () => {
    const all = await Promise.all(q.map(item));
    return all.find((it) => it.status.state === 'Running');
  });
  check('the download moved to the top started first', running.id === q[2], `${running.id} vs ${q[2]}`);
  await api('POST', '/api/queue/stop');
  for (const id of q) await api('DELETE', `/api/downloads/${id}?purge=true`);

  // 5. Not enough space: it waits, says why, and carries on once there is room.
  await api('POST', '/api/settings', { min_free_space_mib: 1 << 20 });
  const sp = await add({ url: `http://127.0.0.1:${SITE}/f.bin?space` });
  const isp = await until('stopped for space', async () => { const it = await item(sp); return it && it.waiting_for_space ? it : null; });
  check('a download short of space waits', isp.status.state === 'Paused', JSON.stringify(isp.status));
  check('the wait explains itself', /not enough space on .*more free space/.test(isp.note || ''), JSON.stringify(isp.note));
  await api('POST', '/api/settings', { min_free_space_mib: 0 });
  await api('POST', `/api/downloads/${sp}/resume`);
  check('it carries on when there is room', (await done(sp)).status.state === 'Done');

  // 6. Restart: the old part file is dropped and the download starts again at once.
  const rs = await add({ url: `http://127.0.0.1:${SITE}/slow.bin?restart` });
  await until('some progress', async () => (await item(rs)).downloaded > 0);
  r = await api('POST', `/api/downloads/${rs}/restart`);
  check('restart answers', r.status < 300, `${r.status}`);
  const rsState = await until('running again', async () => { const st = (await item(rs)).status.state; return ['Running', 'Queued'].includes(st) ? st : null; }, 5000);
  check('the download is running again', !!rsState, String(rsState));
  await api('POST', `/api/downloads/${rs}/pause`);
  await api('DELETE', `/api/downloads/${rs}?purge=true`);

  // 7. Remove with and without the file.
  const pu = await add({ url: `http://127.0.0.1:${SITE}/f.bin?purge` });
  const ipu = await done(pu);
  await api('DELETE', `/api/downloads/${pu}?purge=true`);
  check('remove and delete deletes the file', !fs.existsSync(ipu.path));
  const ke = await add({ url: `http://127.0.0.1:${SITE}/f.bin?keep` });
  const ike = await done(ke);
  await api('DELETE', `/api/downloads/${ke}`);
  check('remove alone keeps the file', fs.existsSync(ike.path));

  // 8. CLI: list, add --later, rm against the same service.
  const list1 = await cli(['list']);
  check('cli list works', list1.code === 0 && list1.out.includes('f.bin'), list1.out.slice(0, 120));
  const cliUrl = `http://127.0.0.1:${SITE}/f.bin?cli`;
  const added = await cli(['add', '--later', cliUrl]);
  check('cli add --later works', added.code === 0, added.out.slice(0, 120));
  const all = (await api('GET', '/api/downloads')).body;
  const cliItem = all.find((it) => it.request.url === cliUrl);
  check('cli add queued the download', !!cliItem, JSON.stringify(all.map((x) => x.request.url)));
  if (cliItem) {
    const removed = await cli(['rm', String(cliItem.id)]);
    check('cli rm works', removed.code === 0, removed.out.slice(0, 120));
    check('cli rm removed it', (await api('GET', `/api/downloads/${cliItem.id}`)).status === 404);
  }

  await api('POST', '/api/shutdown');
} catch (e) {
  failures.push(String(e));
  console.error('ERROR', e);
  console.error(svcErr.slice(-1500));
} finally {
  site.close();
  proxy.close();
  if (svc.exitCode === null) svc.kill('SIGKILL');
  console.log(`\n${failures.length} failure(s)`);
  for (const f of failures) console.log(' -', f);
  console.log(`work dir ${run}`);
  process.exit(failures.length ? 1 : 0);
}
