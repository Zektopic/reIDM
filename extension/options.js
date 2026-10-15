// reIDM Integration — options page.
//
// The defaults must match `DEFAULTS` in service-worker.js.

const DEFAULTS = {
  port: 61002,
  capture: true,
  interceptClicks: true,
  captureAll: true,
  fileTypes:
    '3GP 7Z AAC ACE AIF APK ARJ ASF AVI BIN BZ2 DEB DMG EXE FLAC GZ GZIP IMG ISO LZH M4A M4V MKV MOV MP3 MP4 ' +
    'MPA MPE MPEG MPG MSI MSU OGG OGV PDF PKG PLJ PPS PPT QT RA RAR RM RMVB RPM SEA SIT SITX TAR TGZ TIF TIFF ' +
    'WAV WEBM WMA WMV XZ Z ZIP ZST',
  minSizeKB: 0,
  skipHosts: [],
  captureIncognito: false,
  launchReidm: true,
  notifications: true,
  sniffMedia: true,
  ignore: ['blob:', 'data:', 'chrome:', 'chrome-extension://', 'about:', 'file://', 'filesystem:'],
};

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
  for (const k of ['capture', 'interceptClicks', 'captureAll', 'captureIncognito', 'launchReidm', 'notifications', 'sniffMedia']) $(k).checked = !!s[k];
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

(async () => show({ ...DEFAULTS, ...(await chrome.storage.sync.get(DEFAULTS)) }))();

$('capture').addEventListener('change', syncEnabled);

$('test').addEventListener('click', async () => {
  const port = Number($('port').value) || DEFAULTS.port;
  setStatus('checking…');
  // Through the service worker: a fetch here carries this page's origin, which reIDM would
  // rightly refuse.
  await chrome.storage.sync.set({ port });
  const r = await new Promise((resolve) =>
    chrome.runtime.sendMessage({ type: 'ping' }, (x) => {
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
  await chrome.storage.sync.set({
    port: Number($('port').value) || DEFAULTS.port,
    capture: $('capture').checked,
    interceptClicks: $('interceptClicks').checked,
    captureAll: $('captureAll').checked,
    fileTypes,
    minSizeKB: Math.max(0, Number($('minSizeKB').value) || 0),
    skipHosts: lines('skipHosts').map((h) => h.toLowerCase().replace(/^https?:\/\//, '').replace(/\/.*$/, '')),
    captureIncognito: $('captureIncognito').checked,
    launchReidm: $('launchReidm').checked,
    notifications: $('notifications').checked,
    sniffMedia: $('sniffMedia').checked,
    ignore: lines('ignore'),
  });
  $('fileTypes').value = fileTypes;
  $('saved').textContent = 'saved';
  setTimeout(() => ($('saved').textContent = ''), 2000);
});

$('reset').addEventListener('click', async () => {
  await chrome.storage.sync.set(DEFAULTS);
  show(DEFAULTS);
  $('saved').textContent = 'defaults restored';
  setTimeout(() => ($('saved').textContent = ''), 2000);
});
