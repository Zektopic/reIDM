// IDM's "Download file info" dialog: what is about to be downloaded, with Start download,
// Download later and Cancel. The service worker opened this window and waits for the answer,
// which goes back over a port; the port also keeps the worker alive while the user decides.

const $ = (id) => document.getElementById(id);
const id = new URLSearchParams(location.search).get('id');
const port = chrome.runtime.connect({ name: `confirm:${id}` });
let answered = false;

function human(n) {
  if (!(n > 0)) return null;
  const u = ['bytes', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024;
    i++;
  }
  return i ? `${n.toFixed(n < 10 ? 2 : 1)} ${u[i]}` : `${n} bytes`;
}

port.onMessage.addListener((m) => {
  if (m.type === 'info') {
    $('url').textContent = m.url || '—';
    $('url').title = m.url || '';
    if (!$('name').dataset.edited) $('name').value = m.filename || '';
    if (m.dir) {
      $('dir').textContent = m.dir;
      $('dir').title = m.dir;
    }
    $('size').textContent = human(m.size) || (m.sizeNote || 'unknown');
    $('name').focus();
    $('name').select();
  } else if (m.type === 'size') {
    $('size').textContent = human(m.size) || m.note || 'unknown';
    if (m.filename && !$('name').dataset.edited) $('name').value = m.filename;
  } else if (m.type === 'error') {
    $('err').textContent = m.error;
    for (const b of ['start', 'later']) $(b).disabled = false;
  } else if (m.type === 'done') {
    window.close();
  }
});
port.postMessage({ type: 'hello' });
// A message every 20 s keeps the service worker from being stopped while the user decides.
setInterval(() => port.postMessage({ type: 'alive' }), 20000);

$('name').addEventListener('input', () => ($('name').dataset.edited = '1'));

function answer(action) {
  if (answered && action !== 'cancel') return;
  answered = true;
  $('err').textContent = '';
  for (const b of ['start', 'later']) $(b).disabled = true;
  port.postMessage({ type: 'answer', action, filename: $('name').value.trim(), noAsk: $('noAsk').checked });
  if (action === 'cancel') window.close();
}

$('start').addEventListener('click', () => answer('start'));
$('later').addEventListener('click', () => answer('later'));
$('cancel').addEventListener('click', () => answer('cancel'));
document.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') answer('start');
  else if (e.key === 'Escape') answer('cancel');
});
