const el = id => document.getElementById(id);
const {invoke, Channel} = window.__TAURI__.core;
const labels = {idle: 'Idle', starting: 'Starting', running: 'Capturing', stopping: 'Stopping'};
const vendors = {claude: 'Anthropic', codex: 'OpenAI'};
let actionError = '';
let notice = '';
let pending = false;
let revision = 0;
let ready = false;
let phase = 'idle';
let importing = false;

function controls() {
  el('start').disabled = pending || importing || phase !== 'idle' || !ready;
  el('stop').disabled = pending || !['starting', 'running'].includes(phase);
  el('provider').disabled = el('save').disabled = pending || importing || phase !== 'idle';
  el('resume').disabled = importing || pending || phase !== 'idle';
  el('about').disabled = pending || phase !== 'idle';
  el('solve').disabled = phase !== 'running' || !el('provider').value;
  el('forget').disabled = phase !== 'idle' || !ready || !el('provider').value;
}
async function refresh() {
  const request = ++revision;
  try {
    const s = await invoke('status');
    if (request !== revision) return;
    phase = s.phase;
    el('phase').textContent = labels[s.phase];
    controls();
    el('video').textContent = s.width ? s.width + ' × ' + s.height :
      s.phase === 'starting' ? 'Choose a source (Esc to cancel)' :
      s.phase === 'running' ? 'Waiting for frames' : 'Waiting for selection';
    el('frames').textContent = s.frames.toLocaleString() + ' frames';
    el('audio').textContent = s.audio_frames.toLocaleString() + ' audio frames';
    el('level').value = s.peak;
    el('message').textContent = s.message || actionError || notice;
  } catch {
    if (request !== revision) return;
    el('message').textContent = 'Capture status is unavailable.';
    el('start').disabled = true;
    el('stop').disabled = pending;
  }
}
async function act(command, args) {
  if (pending) return;
  pending = true;
  ++revision;
  actionError = '';
  controls();
  try { return await invoke(command, args); }
  catch (error) { actionError = typeof error === 'string' ? error : 'Command failed.'; }
  finally { pending = false; await refresh(); }
}
async function account() {
  const provider = el('provider').value;
  el('keyrow').hidden = el('aboutrow').hidden = el('solve').hidden = !provider;
  el('assist').hidden = !provider;
  if (!provider) {
    ready = true;
    el('funding').textContent = 'Nothing leaves this computer. No provider is contacted.';
    return controls();
  }
  ready = false;
  controls();
  try {
    const a = await invoke('account', {provider});
    if (provider !== el('provider').value) return;
    ready = a.saved;
    el('key').placeholder = a.saved ? 'Key saved. Enter a new one to replace it.' : '';
    el('funding').textContent = a.saved
      ? `Billed to your ${vendors[provider]} API key (${a.remembered ? 'saved in the system keychain' : 'this session only'}). ` +
        `Speech is transcribed on this computer with a 50 MB model downloaded once from huggingface.co. The transcript and one screenshot per spoken turn are sent to ${vendors[provider]}.`
      : `Enter your ${vendors[provider]} API key to use this assistant.`;
  } catch {
    el('funding').textContent = 'Account status is unavailable.';
  }
  controls();
}
function events() {
  const channel = new Channel();
  let turn = 0;
  channel.onmessage = e => {
    if (e.type === 'model') notice = e.total ? `Downloading speech model: ${Math.floor(100 * e.done / e.total)}%` : '';
    else if (e.type === 'heard') { if (e.turn < turn) return; turn = e.turn; el('heard').textContent = e.text; el('suggestion').textContent = ''; el('latency').textContent = ''; }
    else if (e.type === 'delta' && e.turn === turn) {
      el('suggestion').textContent += e.text;
      if (e.ms != null) el('latency').textContent = (e.ms / 1000).toFixed(1) + ' s';
    }
    else if (e.type === 'notice') notice = e.text;
  };
  return channel;
}
el('provider').addEventListener('change', account);
el('account').addEventListener('submit', async e => {
  e.preventDefault();
  const key = el('key').value.trim();
  if (!key) return;
  const saved = await act('set_key', {provider: el('provider').value, key, remember: el('remember').checked});
  if (saved) {
    el('key').value = '';
    notice = saved.remembered || !el('remember').checked ? '' : 'The system keychain is unavailable. The key is kept for this session only.';
  }
  await account();
});
el('forget').addEventListener('click', async () => { await act('forget_key', {provider: el('provider').value}); await account(); });
el('start').addEventListener('click', () => {
  notice = '';
  el('heard').textContent = el('suggestion').textContent = el('latency').textContent = '';
  act('start', {provider: el('provider').value || null, about: el('about').value, events: events()});
});
el('stop').addEventListener('click', () => { notice = ''; act('stop'); });
el('solve').addEventListener('click', () => act('ask'));
const main = document.querySelector('main');

function hideUntilChecked() {
  main.hidden = true;
  el('privacy').hidden = false;
  el('privacy-status').textContent = 'Checking screen-share privacy…';
  el('privacy-retry').hidden = true;
}

async function checkSharing() {
  hideUntilChecked();
  try {
    el('sharing').textContent = await invoke('sharing_hint');
    el('privacy').hidden = true;
    main.hidden = false;
  } catch (error) {
    el('privacy-status').textContent = typeof error === 'string'
      ? error : 'Could not confirm screen-share privacy. Your Argos content is hidden.';
    try { await invoke('stop'); } catch { /* The native bridge may be unavailable. */ }
    el('privacy-retry').hidden = false;
  }
}

el('privacy-retry').addEventListener('click', checkSharing);
el('float').addEventListener('click', async () => {
  const button = el('float');
  const on = !document.documentElement.classList.contains('overlay');
  button.disabled = true;
  hideUntilChecked();
  try {
    await invoke('overlay', {on});
    document.documentElement.classList.toggle('overlay', on);
    button.setAttribute('aria-pressed', on);
    button.textContent = on ? 'Exit float' : 'Float on top';
  } catch {
    actionError = 'Could not change the floating window.';
  } finally {
    await checkSharing();
    button.disabled = false;
  }
});
// Storage can be unavailable. The notes then last for this run only.
try { el('about').value = localStorage.getItem('about') || ''; } catch {}
el('about').addEventListener('input', () => { try { localStorage.setItem('about', el('about').value); } catch {} });
el('resume').addEventListener('change', async () => {
  const input = el('resume');
  const file = input.files[0];
  if (!file) return;
  importing = true;
  actionError = '';
  controls();
  try {
    if (file.size > 5 * 1024 * 1024) throw 'Choose a résumé under 5 MB.';
    let text;
    if (/\.pdf$/i.test(file.name)) text = await invoke('resume_pdf', await file.arrayBuffer());
    else if (/\.(txt|md)$/i.test(file.name)) text = await file.text();
    else throw 'Choose a PDF or plain-text résumé.';
    if (!text.trim()) throw 'The résumé has no text.';
    const notes = el('about');
    const combined = [notes.value.trimEnd(), text.trim()].filter(Boolean).join('\n\n');
    if (combined.length > notes.maxLength) throw 'The combined notes exceed 20,000 characters. Paste a shorter excerpt.';
    notes.value = combined;
    notes.dispatchEvent(new Event('input'));
  } catch (error) {
    actionError = typeof error === 'string' ? error : 'Could not read the résumé. Paste its text instead.';
  } finally {
    input.value = '';
    importing = false;
    controls();
    await refresh();
  }
});
checkSharing();
async function poll() { await refresh(); setTimeout(poll, 200); }
account();
poll();
