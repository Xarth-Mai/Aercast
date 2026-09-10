import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const script = readFileSync(new URL('../src/viewer.html', import.meta.url), 'utf8')
  .split('<script>')[1].split('</script>')[0];
function browser() {
  let now = 0;
  let id = 0;
  const timers = new Map();
  const video = Object.assign(new EventTarget(), {
    currentTime: 0, paused: false, muted: false, seeking: false,
    getVideoPlaybackQuality: () => ({ totalVideoFrames: 0, droppedVideoFrames: 0 }),
  });
  const document = Object.assign(new EventTarget(), { hidden: false, body: { dataset: {} },
    querySelector: selector => selector === 'video' ? video : {} });
  const context = vm.createContext({
    document, self: { MediaSource: class { static isTypeSupported() { return true; } } }, localStorage: { getItem() {}, setItem() {} },
    crypto: { getRandomValues: bytes => bytes.fill(1) },
    location: { pathname: '/s/test' }, console: { info() {}, warn() {}, log() {}, error() {} },
    AbortController, URL, Uint8Array,
    performance: { now: () => now },
    setTimeout: (fn, ms) => { timers.set(++id, { fn, at: now + ms }); return id; },
    setInterval: (fn, ms) => { timers.set(++id, { fn, at: now + ms, interval: ms }); return id; },
    clearTimeout: id => timers.delete(id), clearInterval: id => timers.delete(id),
  });
  // Skip only automatic bootstrap; exercise the shipped functions with browser API doubles
  vm.runInContext(script.slice(0, script.lastIndexOf('  if (MediaSourceClass)')), context);
  const run = source => vm.runInContext(source, context);
  const flush = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };
  async function advance(ms) {
    const end = now + ms;
    await flush();
    while (true) {
      const next = [...timers].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
      if (!next) break;
      const [key, timer] = next;
      now = timer.at;
      if (timer.interval) timer.at += timer.interval;
      else timers.delete(key);
      timer.fn();
      await flush();
    }
    now = end;
    await flush();
  }
  run('globalThis.attempt = { controller: new AbortController(), positioned: false, healthy: false }');
  return { context, video, document, run, advance, timers };
}

{
  const b = browser();
  b.context.fetch = () => new Promise(() => {});
  const result = b.run('nextResponse(attempt)').catch(e => e);
  await b.advance(10000);
  assert.match((await result).message, /connection timed out/);
  assert.equal(b.run('attempt.controller.signal.aborted'), true);
  assert.equal(b.timers.size, 0);
}
{
  const b = browser();
  let calls = 0;
  b.context.fetch = async () => {
    if (++calls <= 40) return { status: 425, body: { cancel: async () => {} } };
    return { status: 200 };
  };
  const result = b.run('nextResponse(attempt)');
  await b.advance(20000);
  assert.equal((await result).status, 200);
  assert.equal(b.run('attempt.controller.signal.aborted'), false);
  assert.equal(calls, 41);
}
{
  const b = browser();
  const buffer = Object.assign(new EventTarget(), { buffered: { length: 0 } });
  b.context.buffer = buffer;
  b.run('attempt.opened = Promise.resolve(); attempt.media = { addSourceBuffer: () => buffer }');
  b.context.response = { body: { getReader: () => ({ read: () => new Promise(() => {}) }) } };
  const result = b.run('consume(attempt, response, "video/mp4")').catch(e => e);
  await b.advance(15000);
  assert.match((await result).message, /no (data|progress)/);
  assert.equal(b.run('attempt.controller.signal.aborted'), true);
  assert.equal([...b.timers.values()].filter(t => t.interval).length, 0);
}
{
  const b = browser();
  b.run('attempt.positioned = true; globalThis.stop = watchPlayback(attempt)');
  // Clock seeks alone must not disguise a decoder that presents no frames
  for (let i = 0; i < 15; i++) { b.video.currentTime++; await b.advance(1000); }
  assert.match(b.run('attempt.error.message'), /Playback made no progress/);
  b.run('stop()');
  assert.equal(b.timers.size, 0);
}
for (const mode of ['paused', 'hidden']) {
  const b = browser();
  b.run('attempt.positioned = true; globalThis.stop = watchPlayback(attempt)');
  if (mode === 'paused') b.video.paused = true;
  else b.document.hidden = true;
  await b.advance(60000);
  assert.equal(b.run('attempt.controller.signal.aborted'), false, mode);
  b.video.paused = false;
  b.document.hidden = false;
  await b.advance(15000);
  assert.equal(b.run('attempt.controller.signal.aborted'), true, `${mode} resumed`);
  b.run('stop()');
}
{
  const b = browser();
  b.run('globalThis.stop = watchPlayback(attempt)');
  for (let i = 0; i < 60; i++) {
    b.video.getVideoPlaybackQuality = () => ({ totalVideoFrames: i + 1, droppedVideoFrames: 0 });
    await b.advance(1000);
  }
  assert.equal(b.run('attempt.controller.signal.aborted'), false);
  assert.equal(b.run('attempt.healthy'), true);
  b.run('stop()');
  for (let failures = 1; failures <= 100; failures++) {
    b.run('Math.random = () => 0');
    const min = b.run(`retryDelay(${failures})`);
    b.run('Math.random = () => 1');
    const max = b.run(`retryDelay(${failures})`);
    assert.equal(min, max / 2);
    assert.ok(max <= 10000);
  }
}
console.log('Viewer recovery checks passed: connection, 425 polling, stalled reads, decoder stalls, pause/visibility, healthy stream, bounded jitter');

for (const failed of [false, true]) {
  const b = browser();
  b.run(`
    let attempts = 0;
    globalThis.restartedAt = null;
    Math.random = () => 1;
    nextResponse = async () => ({ status: attempts ? 404 : 200, ok: true, body: attempts ? null : {}, headers: { get: () => 'video/mp4' } });
    consume = async () => { ${failed ? 'throw new Error("injected failure")' : ''} };
    stopAttempt = async () => {};
    beginAttempt = () => { attempts++; restartedAt = performance.now(); return attempt; };
  `);
  const result = b.run('connect(attempt)');
  await b.advance(0);
  assert.equal(b.run('restartedAt'), failed ? null : 0);
  if (failed) {
    await b.advance(500);
    assert.equal(b.run('restartedAt'), 500);
  }
  await result;
}
console.log('Normal EOF reconnects immediately; failure follows backoff');

{
  const b = browser();
  b.run(`
    nextResponse = async () => { throw new Error('injected failure'); };
    stopAttempt = async () => {};
    beginAttempt = () => { throw new Error('inactive tab restarted'); };
  `);
  const result = b.run('connect(attempt)');
  await b.advance(0);
  b.run('running = false');
  await b.advance(1000);
  await result;
}
{
  const b = browser();
  b.run('attempt.opened = new Promise(() => {})');
  const result = b.run('consume(attempt, {}, "video/mp4")').catch(e => e);
  await b.advance(10000);
  assert.match((await result).message, /source did not open/);
  assert.equal(b.timers.size, 0);
}
console.log('Inactive retry stays stopped; unopened media source times out and clears its timer');
