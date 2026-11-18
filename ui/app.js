const el = id => document.getElementById(id);
const invoke = window.__TAURI__.core.invoke;
const labels = {idle: 'Idle', starting: 'Starting', running: 'Capturing', stopping: 'Stopping'};
let actionError = '';
let pending = false;
let revision = 0;

async function refresh() {
  const request = ++revision;
  try {
    const s = await invoke('status');
    if (request !== revision) return;
    el('phase').textContent = labels[s.phase];
    el('start').disabled = pending || s.phase !== 'idle';
    el('stop').disabled = pending || !['starting', 'running'].includes(s.phase);
    el('video').textContent = s.width ? s.width + ' × ' + s.height :
      s.phase === 'starting' ? 'Choose a source (Esc to cancel)' :
      s.phase === 'running' ? 'Waiting for frames' : 'Waiting for selection';
    el('frames').textContent = s.frames.toLocaleString() + ' frames';
    el('audio').textContent = s.audio_frames.toLocaleString() + ' audio frames';
    el('level').value = s.peak;
    el('message').textContent = s.message || actionError;
  } catch {
    if (request !== revision) return;
    el('message').textContent = 'Capture status is unavailable.';
    el('start').disabled = true;
    el('stop').disabled = pending;
  }
}
async function act(command) {
  if (pending) return;
  pending = true;
  ++revision;
  actionError = '';
  el('start').disabled = el('stop').disabled = true;
  try { await invoke(command); }
  catch (error) { actionError = typeof error === 'string' ? error : 'Capture command failed.'; }
  finally { pending = false; await refresh(); }
}
el('start').addEventListener('click', () => act('start'));
el('stop').addEventListener('click', () => act('stop'));
async function poll() { await refresh(); setTimeout(poll, 200); }
poll();
