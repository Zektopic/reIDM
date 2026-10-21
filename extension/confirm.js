// IDM's "Download file info" dialog: what is about to be downloaded, with Start download,
// Download later and Cancel. One window shows every waiting download in turn ("2 more
// waiting"). The service worker opened it and waits for each answer, which goes back over a
// port; the port also keeps the worker alive while the user decides.
//
// While reIDM waits for a new address for one of its downloads, a request first asks "Use this
// link to refresh <name>?" (Refresh / New download / Cancel); *New download* turns it into the
// usual form, or starts it at once when the user has the dialog off.
//
// States: `asking` (the buttons work), `sending` (an answer is being handed to reIDM), `result`
// (reIDM refused, or nobody answered in time: only Close/Next), `gone` (the worker this dialog
// belonged to stopped: only Close).

const $ = (id) => document.getElementById(id);
const port = chrome.runtime.connect({ name: 'confirm' });
let current = null; // the id of the download shown
let state = 'idle';
let fallback = '';
let waiting = 0;
/** `offer`: asking whether to refresh; `form`: the usual new-download form. */
let mode = 'form';
/** Whether *New download* shows the form (the dialog is on) or starts at once (it is off). */
let askNew = true;

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

function setState(next) {
  state = next;
  const asking = next === 'asking';
  const offering = mode === 'offer';
  document.body.classList.toggle('refreshing', offering);
  $('offerQ').hidden = !offering;
  $('title').textContent = offering ? 'Refresh download address' : 'Download file info';
  document.title = `${$('title').textContent} — reIDM`;
  document.querySelector('.ask').hidden = offering || !(asking || next === 'sending');
  $('offer').hidden = !offering || !(asking || next === 'sending');
  $('noAskLabel').hidden = !asking || offering;
  for (const b of ['start', 'later', 'all', 'cancel', 'offerRefresh', 'offerNew', 'offerCancel']) $(b).disabled = !asking;
  $('close').hidden = asking || next === 'sending';
  $('close').textContent = next === 'result' && waiting ? 'Next' : 'Close';
  $('all').hidden = !asking || !waiting;
  if (!$('close').hidden) $('close').focus();
}

function showWaiting(n) {
  waiting = n || 0;
  $('waiting').hidden = !waiting;
  $('waiting').textContent = `${waiting} more waiting`;
  $('all').textContent = `Start all (${waiting + 1})`;
  $('all').hidden = state !== 'asking' || !waiting;
}

/** Focus what Enter should do: the name field in the form, *Refresh* in the offer. */
function focusFirst() {
  if (mode === 'offer') $('offerRefresh').focus();
  else selectBaseName();
}

/** Select the name without its extension, as IDM does: the part one usually changes. */
function selectBaseName() {
  const el = $('name');
  el.focus();
  const dot = el.value.lastIndexOf('.');
  el.setSelectionRange(0, dot > 0 ? dot : el.value.length);
}

port.onMessage.addListener((m) => {
  if (m.type === 'info') {
    current = m.id;
    fallback = m.fallback || '';
    $('url').textContent = m.url || '—';
    $('url').title = m.url || '';
    $('from').textContent = m.from || '';
    $('from').hidden = $('fromK').hidden = !m.from;
    $('name').value = m.filename || '';
    delete $('name').dataset.edited;
    $('dir').textContent = m.dir || "reIDM's download folder";
    $('dir').title = m.dir || '';
    $('size').textContent = human(m.size) || (m.probing ? 'checking…' : m.sizeNote || 'unknown');
    $('program').hidden = !m.program;
    $('err').textContent = '';
    mode = m.refresh ? 'offer' : 'form';
    askNew = m.ask !== false;
    $('offerName').textContent = (m.refresh && m.refresh.name) || '';
    showWaiting(m.waiting);
    setState('asking');
    focusFirst();
  } else if (m.type === 'size' && m.id === current) {
    $('size').textContent = human(m.size) || m.note || 'unknown';
    if (m.filename && !$('name').dataset.edited) {
      $('name').value = m.filename;
      if (document.activeElement === $('name')) selectBaseName();
    }
  } else if (m.type === 'waiting') {
    showWaiting(m.waiting);
  } else if (m.type === 'error' && m.id === current) {
    $('err').textContent = m.text;
    showWaiting(m.waiting);
    setState('result');
  } else if (m.type === 'stale') {
    gone('This download is no longer waiting. If it was one the browser started, the browser has it.');
  }
});

/** The worker this window belonged to stopped (an update, a crash). The one that starts next
 *  resumes what was held, so the browser downloads it; say so, and offer only Close. */
function gone(text) {
  clearInterval(heartbeat);
  if (state === 'gone') return;
  const was = state;
  setState('gone');
  $('err').textContent = text || (was === 'sending'
    ? 'The extension restarted while handing this over. If reIDM does not list it, the browser downloads it.'
    : `This download is no longer waiting. ${fallback}`);
  // Wake the next worker now, so what was held is settled at once, not at its next event.
  chrome.runtime.sendMessage({ type: 'wake' }).catch(() => {});
}

port.onDisconnect.addListener(() => gone());
port.postMessage({ type: 'hello' });
// A message every 20 s keeps the service worker from being stopped while the user decides.
const heartbeat = setInterval(() => {
  try {
    port.postMessage({ type: 'alive' });
  } catch {
    gone();
  }
}, 20000);

$('name').addEventListener('input', () => ($('name').dataset.edited = '1'));

function answer(action) {
  if (state !== 'asking') return;
  $('err').textContent = '';
  const filename = $('name').value.trim();
  if (action === 'all') port.postMessage({ type: 'answer-all', filename });
  else port.postMessage({ type: 'answer', id: current, action, filename, noAsk: mode === 'form' && $('noAsk').checked });
  // Cancel moves straight on; Start and Later wait for reIDM's answer, which brings the next
  // download, a refusal to show, or the window's close.
  setState(action === 'cancel' ? 'idle' : 'sending');
}

$('start').addEventListener('click', () => answer('start'));
$('later').addEventListener('click', () => answer('later'));
$('all').addEventListener('click', () => answer('all'));
$('cancel').addEventListener('click', () => answer('cancel'));
$('offerRefresh').addEventListener('click', () => answer('refresh'));
$('offerCancel').addEventListener('click', () => answer('cancel'));
$('offerNew').addEventListener('click', () => {
  if (state !== 'asking') return;
  if (!askNew) return answer('start');
  mode = 'form';
  setState('asking');
  focusFirst();
});
$('close').addEventListener('click', () => {
  if (state === 'result') {
    port.postMessage({ type: 'ack', id: current });
    setState('idle');
  } else window.close();
});
document.addEventListener('keydown', (e) => {
  // Enter starts only from the name field (or with nothing focused): on a focused button it
  // must do what that button says, which the browser already does by clicking it.
  if (e.key === 'Enter' && (e.target === $('name') || e.target === document.body)) {
    e.preventDefault();
    if (state === 'asking') answer(mode === 'offer' ? 'refresh' : 'start');
    else if (!$('close').hidden) $('close').click();
  } else if (e.key === 'Escape') {
    e.preventDefault();
    if (state === 'asking') answer('cancel');
    else if (!$('close').hidden) $('close').click();
  }
});
