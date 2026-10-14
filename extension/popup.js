// reIDM Integration — popup.
//
// Everything here goes through the service worker, which is the only context that may talk to
// 127.0.0.1 (see the note at the top of service-worker.js).

const $ = (id) => document.getElementById(id);
let items = [];

function say(text, cls = '') {
  const el = $('msg');
  el.textContent = text;
  el.className = 'msg ' + cls;
}

function baseName(url) {
  try {
    const u = new URL(url);
    const last = decodeURIComponent(u.pathname.split('/').filter(Boolean).pop() || '');
    return last || u.hostname;
  } catch {
    return url;
  }
}

function render() {
  const list = $('list');
  if (!items.length) {
    list.innerHTML = '<div class="empty">No media or download links found on this page yet.<br>Play a video, or use the right-click menu on a link.</div>';
    $('downloadSelected').disabled = true;
    $('downloadAll').disabled = true;
    return;
  }
  $('downloadSelected').disabled = false;
  $('downloadAll').disabled = false;

  list.innerHTML = items
    .map((it, i) => {
      const kind = (it.kind || 'media').toLowerCase();
      return `<div class="row">
        <input type="checkbox" data-i="${i}" checked>
        <span class="kind ${kind}">${kind}</span>
        <span class="meta">
          <div class="fn">${escapeHtml(baseName(it.url))}</div>
          <div class="url" title="${escapeHtml(it.url)}">${escapeHtml(it.url)}</div>
        </span>
      </div>`;
    })
    .join('');
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
}

async function send(details) {
  return new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'add', details }, (r) => {
      void chrome.runtime.lastError;
      resolve(r || { ok: false, error: 'no reply from the extension' });
    });
  });
}

async function queue(urls) {
  let ok = 0;
  const errors = [];
  for (const url of urls) {
    const r = await send({ url, filename: baseName(url) || undefined, referer: location.href });
    if (r.ok) ok++;
    else errors.push(`${baseName(url)}: ${r.error || 'failed'}`);
  }
  if (ok && !errors.length) say(`Queued ${ok} download${ok === 1 ? '' : 's'}.`, 'ok');
  else if (ok) say(`Queued ${ok}; ${errors.length} failed — ${errors[0]}`, '');
  else say(errors[0] || 'nothing was queued', 'err');
}

function selectedUrls() {
  return [...document.querySelectorAll('#list input[type=checkbox]:checked')].map((c) => items[Number(c.dataset.i)].url);
}

// ── startup ─────────────────────────────────────────────────────────────

(async () => {
  const { capture = true } = await chrome.storage.sync.get({ capture: true });
  $('capture').checked = capture;

  const info = await new Promise((resolve) =>
    chrome.runtime.sendMessage({ type: 'ping' }, (r) => {
      void chrome.runtime.lastError;
      resolve(r);
    }),
  );
  if (info && info.ok) {
    $('dot').style.background = 'var(--ok)';
    $('statusText').textContent = `running · v${info.info.version}`;
  } else {
    $('dot').style.background = 'var(--err)';
    $('statusText').textContent = 'not running';
    say('reIDM is not running. Start it, then reopen this popup.', 'err');
  }

  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (tab) {
    const r = await new Promise((resolve) =>
      chrome.runtime.sendMessage({ type: 'media-for-tab', tabId: tab.id }, (x) => {
        void chrome.runtime.lastError;
        resolve(x);
      }),
    );
    items = (r && r.items) || [];
    // Newest first: the thing you just played is the one you want.
    items.sort((a, b) => (b.at || 0) - (a.at || 0));
  }
  render();
})();

// ── events ──────────────────────────────────────────────────────────────

$('capture').addEventListener('change', (e) => chrome.storage.sync.set({ capture: e.target.checked }));

$('downloadSelected').addEventListener('click', () => {
  const urls = selectedUrls();
  if (!urls.length) return say('Select at least one item.', 'err');
  queue(urls);
});

$('downloadAll').addEventListener('click', () => queue(items.map((i) => i.url)));

$('clear').addEventListener('click', async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (tab) chrome.runtime.sendMessage({ type: 'clear-tab', tabId: tab.id }, () => void chrome.runtime.lastError);
  items = [];
  render();
  say('Cleared the list for this page.');
});

$('options').addEventListener('click', () => chrome.runtime.openOptionsPage());

$('addUrl').addEventListener('click', addTyped);
$('url').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') addTyped();
});

function addTyped() {
  const v = $('url').value.trim();
  if (!/^https?:\/\//i.test(v)) return say('Enter an http or https link.', 'err');
  $('url').value = '';
  queue([v]);
}
