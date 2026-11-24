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

function controls() {
  el('start').disabled = pending || phase !== 'idle' || !ready;
  el('stop').disabled = pending || !['starting', 'running'].includes(phase);
  el('provider').disabled = el('save').disabled = phase !== 'idle';
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
  el('keyrow').hidden = !provider;
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
      if (e.ms != null) el('latency').textContent = (e.ms / 1000).toFixed(1) + ' s after speech ended';
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
  act('start', {provider: el('provider').value || null, events: events()});
});
el('stop').addEventListener('click', () => { notice = ''; act('stop'); });
async function poll() { await refresh(); setTimeout(poll, 200); }
account();
poll();
