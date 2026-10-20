// reIDM Integration — options page.
//
// The defaults come from defaults.js, which the service worker and content scripts share.

const DEFAULTS = globalThis.REIDM_DEFAULTS;

const $ = (id) => document.getElementById(id);
const lines = (id) =>
  $(id)
    .value.split('\n')
    .map((l) => l.trim())
    .filter(Boolean);

function setStatus(text, cls = '') {
  $('status').textContent = text;
  $('status').className = cls;
}

function show(s) {
  for (const k of ['capture', 'interceptClicks', 'captureAll', 'captureIncognito', 'launchReidm', 'notifications', 'askBeforeDownload', 'sniffMedia']) $(k).checked = !!s[k];
  $('port').value = s.port;
  $('minSizeKB').value = s.minSizeKB;
  $('fileTypes').value = s.fileTypes;
  $('skipHosts').value = (s.skipHosts || []).join('\n');
  $('ignore').value = (s.ignore || []).join('\n');
  syncEnabled();
}

/** Sub-options of "take over" mean nothing while it is off. */
function syncEnabled() {
  const on = $('capture').checked;
  for (const id of ['interceptClicks', 'captureAll', 'fileTypes', 'minSizeKB', 'skipHosts', 'captureIncognito']) $(id).disabled = !on;
}

/** Whether the user has edited something not yet saved. Until then, a change made elsewhere
 *  (the popup's switches) is shown here, so Save cannot quietly undo it. */
let dirty = false;
const load = async () => show({ ...DEFAULTS, ...(await chrome.storage.sync.get(DEFAULTS)) });
load();
document.addEventListener('input', () => (dirty = true));
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === 'sync' && !dirty) load();
});

const clampPort = (v) => {
  const n = Math.round(Number(v));
  return n >= 1 && n <= 65535 ? n : null;
};

$('capture').addEventListener('change', syncEnabled);

$('test').addEventListener('click', async () => {
  const port = clampPort($('port').value);
  if (!port) return setStatus('a port is a number from 1 to 65535', 'err');
  setStatus('checking…');
  // Through the service worker: a fetch here carries this page's origin, which reIDM would
  // rightly refuse. The port is tried, not saved: Save does that.
  const r = await new Promise((resolve) =>
    chrome.runtime.sendMessage({ type: 'ping', port }, (x) => {
      void chrome.runtime.lastError;
      resolve(x);
    }),
  );
  if (r && r.ok) setStatus(`connected — reIDM ${r.info.version}`, 'ok');
  else setStatus(`no answer on port ${port}`, 'err');
});

$('save').addEventListener('click', async () => {
  const fileTypes = $('fileTypes')
    .value.toUpperCase()
    .split(/[\s,;|]+/)
    .map((t) => t.replace(/^\./, ''))
    .filter(Boolean)
    .join(' ');
  const port = clampPort($('port').value);
  if (!port) return setStatus('a port is a number from 1 to 65535', 'err');
  await chrome.storage.sync.set({
    port,
    capture: $('capture').checked,
    interceptClicks: $('interceptClicks').checked,
    captureAll: $('captureAll').checked,
    fileTypes,
    minSizeKB: Math.max(0, Number($('minSizeKB').value) || 0),
    skipHosts: lines('skipHosts').map((h) => h.toLowerCase().replace(/^https?:\/\//, '').replace(/\/.*$/, '').replace(/:\d+$/, '')).filter(Boolean),
    captureIncognito: $('captureIncognito').checked,
    launchReidm: $('launchReidm').checked,
    notifications: $('notifications').checked,
    askBeforeDownload: $('askBeforeDownload').checked,
    sniffMedia: $('sniffMedia').checked,
    ignore: lines('ignore'),
  });
  dirty = false;
  load();
  $('saved').textContent = 'saved';
  setTimeout(() => ($('saved').textContent = ''), 2000);
});

$('reset').addEventListener('click', async () => {
  if (!confirm('Put every setting back to its default? Your list of sites to leave alone is kept.')) return;
  const { skipHosts, ...rest } = DEFAULTS;
  await chrome.storage.sync.set(rest);
  dirty = false;
  await load();
  $('saved').textContent = 'defaults restored';
  setTimeout(() => ($('saved').textContent = ''), 2000);
});
