#!/usr/bin/env node
// Build the Firefox variant of reIDM's extension from the Chromium sources.
//
//   node tools/build-firefox-extension.mjs [--self-test]
//
// The shared code is written for Chrome's MV3 shape. The copy made here gets a one-line shim
// (tools/firefox-shim.mjs) and `chrome.` is renamed to the shim, so both Chrome's callback and
// Firefox's promise APIs work. The manifest is rewritten for Firefox: an event page with module
// scripts (Firefox has no MV3 service workers), the gecko ID the native host admits, and
// Firefox 128 — the first with registered content scripts in the page's own world, which the
// page hook needs.
//
// Output: dist/firefox-extension/ and dist/reidm-extension-firefox-<version>.zip

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { SHIM, selfTest } from './firefox-shim.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const SRC = path.join(ROOT, 'extension');
const OUT = path.join(ROOT, 'dist', 'firefox-extension');
const FIREFOX_ID = 'reidm@zektopic.in';

if (process.argv.includes('--self-test')) {
  const passed = await selfTest();
  console.log(`shim self-test: ${passed.length} checks passed`);
}

const chrome = JSON.parse(fs.readFileSync(path.join(SRC, 'manifest.json'), 'utf8'));
fs.rmSync(OUT, { recursive: true, force: true });
fs.mkdirSync(path.dirname(OUT), { recursive: true });
fs.cpSync(SRC, OUT, { recursive: true });

const replaced = [];
for (const name of fs.readdirSync(OUT)) {
  if (!name.endsWith('.js') || name === 'inject.js') continue;
  const file = path.join(OUT, name);
  let code = fs.readFileSync(file, 'utf8');
  if (!code.includes('chrome.')) continue;
  code = code.replace(/\bchrome\./g, 'reidmApi.');
  fs.writeFileSync(file, `${SHIM}\n${code}`);
  replaced.push(name);
}

// Firefox's manifest: an event page running the same module (it imports defaults.js), the
// gecko ID, and no Chrome-only fields. The page hook is a registered MAIN-world content script,
// supported from Firefox 128, so inject.js needs no web_accessible_resources entry.
const manifest = {
  manifest_version: 3,
  name: chrome.name,
  version: chrome.version,
  description: chrome.description,
  browser_specific_settings: {
    gecko: {
      id: FIREFOX_ID,
      strict_min_version: '128.0',
      // What the extension hands to reIDM on this machine: addresses, and the cookies, page
      // headers and content that let it download what the user can see. Nothing goes to us.
      data_collection_permissions: { required: ['browsingActivity', 'websiteContent'] },
    },
  },
  icons: chrome.icons,
  background: { scripts: ['service-worker.js'], type: 'module' },
  action: chrome.action,
  options_ui: { page: 'options.html', open_in_tab: true },
  permissions: chrome.permissions,
  host_permissions: chrome.host_permissions,
  content_scripts: chrome.content_scripts,
};
fs.writeFileSync(path.join(OUT, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);

const zip = path.join(ROOT, 'dist', `reidm-extension-firefox-${chrome.version}.zip`);
fs.rmSync(zip, { force: true });
const z = spawnSync('zip', ['-r', '-X', zip, '.'], { cwd: OUT, encoding: 'utf8' });
if (z.status !== 0) {
  console.error(z.stderr || z.stdout);
  process.exit(1);
}
console.log(`Firefox extension ${chrome.version}: ${zip}`);
console.log(`shimmed: ${replaced.join(', ')}`);
