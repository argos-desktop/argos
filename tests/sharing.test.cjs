// Run with node --test tests/sharing.test.cjs.
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {runInNewContext} = require('node:vm');
const {setImmediate: settle} = require('node:timers/promises');

test('private content waits for Windows at startup and after floating mode changes', async () => {
  const html = readFileSync('ui/index.html', 'utf8');
  assert.match(html, /<main hidden>/);
  const main = {hidden: true};
  const elements = new Map();
  function element(id) {
    if (!elements.has(id)) elements.set(id, {
      value: '', addEventListener(event, handler) { this[event] = handler; },
      setAttribute() {},
    });
    return elements.get(id);
  }

  let accepted = false;
  let stopRequests = 0;
  async function invoke(command) {
    if (command === 'sharing_hint') {
      assert.equal(main.hidden, true);
      if (!accepted) throw 'Windows rejected exclusion.';
      return 'Windows accepted exclusion.';
    }
    if (command === 'overlay') assert.equal(main.hidden, true);
    if (command === 'stop') stopRequests++;
    if (command === 'status') return {phase: 'idle', frames: 0, audio_frames: 0};
  }
  const document = {
    getElementById: element, querySelector: () => main,
    documentElement: {classList: {contains: () => false, toggle() {}}},
  };
  runInNewContext(readFileSync('ui/app.js', 'utf8'), {
    document, window: {__TAURI__: {core: {invoke}}}, setTimeout() {},
  });
  await settle();
  assert.equal(main.hidden, true);
  assert.equal(element('privacy-retry').hidden, false);
  assert.equal(stopRequests, 1);

  accepted = true;
  await element('privacy-retry').click();
  assert.equal(main.hidden, false);
  accepted = false;
  await element('float').click();
  assert.equal(main.hidden, true);
  assert.equal(stopRequests, 2);
  assert.equal(element('privacy-retry').hidden, false);
});
