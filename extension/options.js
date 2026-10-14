// reIDM Integration — options page.

const DEFAULTS = {
  port: 61002,
  capture: true,
  sniffMedia: true,
  ignore: ['blob:', 'data:', 'chrome:', 'chrome-extension://', 'about:', 'file://'],
};

const $ = (id) => document.getElementById(id);

function setStatus(text, cls = '') {
  const el = $('status');
  el.textContent = text;
  el.className = cls;
}

(async () => {
  const s = { ...DEFAULTS, ...(await chrome.storage.sync.get(DEFAULTS)) };
  $('port').value = s.port;
  $('capture').checked = s.capture;
  $('sniffMedia').checked = s.sniffMedia;
  $('ignore').value = (s.ignore || []).join('\n');
})();

$('test').addEventListener('click', async () => {
  const port = Number($('port').value) || DEFAULTS.port;
  setStatus('checking…');
  try {
    // Through the service worker: a fetch here carries this page's origin and would be
    // rejected by reIDM, which is the point of the origin check.
    await chrome.storage.sync.set({ port });
    const r = await new Promise((resolve) =>
      chrome.runtime.sendMessage({ type: 'ping' }, (x) => {
        void chrome.runtime.lastError;
        resolve(x);
      }),
    );
    if (r && r.ok) setStatus(`connected — reIDM ${r.info.version}`, 'ok');
    else setStatus(`no answer on port ${port}`, 'err');
  } catch (e) {
    setStatus(String(e && e.message ? e.message : e), 'err');
  }
});

$('save').addEventListener('click', async () => {
  const ignore = $('ignore')
    .value.split('\n')
    .map((l) => l.trim())
    .filter(Boolean);
  await chrome.storage.sync.set({
    port: Number($('port').value) || DEFAULTS.port,
    capture: $('capture').checked,
    sniffMedia: $('sniffMedia').checked,
    ignore,
  });
  $('saved').textContent = 'saved';
  setTimeout(() => ($('saved').textContent = ''), 2000);
});
