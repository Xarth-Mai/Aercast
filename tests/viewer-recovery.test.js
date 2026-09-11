import { expect, test } from 'bun:test';
import vm from 'node:vm';

const script = (await Bun.file(new URL('../src/viewer.html', import.meta.url)).text())
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

test('connection timeout aborts the request and clears its timer', async () => {
  const b = browser();
  b.context.fetch = () => new Promise(() => {});
  const result = b.run('nextResponse(attempt)').catch(e => e);
  await b.advance(10000);
  expect((await result).message).toMatch(/connection timed out/);
  expect(b.run('attempt.controller.signal.aborted')).toBe(true);
  expect(b.timers.size).toBe(0);
});
test('425 responses keep polling without a total waiting deadline', async () => {
  const b = browser();
  let calls = 0;
  b.context.fetch = async () => {
    if (++calls <= 40) return { status: 425, body: { cancel: async () => {} } };
    return { status: 200 };
  };
  const result = b.run('nextResponse(attempt)');
  await b.advance(20000);
  expect((await result).status).toBe(200);
  expect(b.run('attempt.controller.signal.aborted')).toBe(false);
  expect(calls).toBe(41);
});
test.each(['read', 'append'])('stalled %s aborts consumption and stops the watchdog', async stalled => {
  const b = browser();
  const buffer = Object.assign(new EventTarget(), { buffered: { length: 0 }, appendBuffer() {} });
  b.video.paused = true;
  b.run('attempt.positioned = true');
  b.context.buffer = buffer;
  b.run('attempt.opened = Promise.resolve(); attempt.media = { addSourceBuffer: () => buffer }');
  b.context.response = { body: { getReader: () => ({ read: () => stalled === 'read'
    ? new Promise(() => {})
    : Promise.resolve({ done: false, value: new Uint8Array([1]) }) }) } };
  const result = b.run('consume(attempt, response, "video/mp4")').catch(e => e);
  await b.advance(15000);
  expect((await result).message).toBe(stalled === 'read'
    ? 'Stream received no data' : 'Media append made no progress');
  expect(b.run('attempt.controller.signal.aborted')).toBe(true);
  expect([...b.timers.values()].filter(t => t.interval)).toHaveLength(0);
});
test('decoder stalls cannot be hidden by advancing playback time', async () => {
  const b = browser();
  b.run('attempt.positioned = true; globalThis.stop = watchPlayback(attempt)');
  // Clock seeks alone must not disguise a decoder that presents no frames
  for (let i = 0; i < 15; i++) { b.video.currentTime++; await b.advance(1000); }
  expect(b.run('attempt.error.message')).toMatch(/Playback made no progress/);
  b.run('stop()');
  expect(b.timers.size).toBe(0);
});
test.each(['paused', 'hidden'])('%s playback suspends progress detection', async mode => {
  const b = browser();
  b.run('attempt.positioned = true; globalThis.stop = watchPlayback(attempt)');
  if (mode === 'paused') b.video.paused = true;
  else b.document.hidden = true;
  await b.advance(60000);
  expect(b.run('attempt.controller.signal.aborted')).toBe(false);
  b.video.paused = false;
  b.document.hidden = false;
  await b.advance(15000);
  expect(b.run('attempt.controller.signal.aborted')).toBe(true);
  b.run('stop()');
});
test('healthy playback has no fixed lifetime and retry jitter stays bounded', async () => {
  const b = browser();
  b.run('globalThis.stop = watchPlayback(attempt)');
  for (let i = 0; i < 60; i++) {
    b.video.getVideoPlaybackQuality = () => ({ totalVideoFrames: i + 1, droppedVideoFrames: 0 });
    await b.advance(1000);
  }
  expect(b.run('attempt.controller.signal.aborted')).toBe(false);
  expect(b.run('attempt.healthy')).toBe(true);
  b.run('stop()');
  for (let failures = 1; failures <= 100; failures++) {
    b.run('Math.random = () => 0');
    const min = b.run(`retryDelay(${failures})`);
    b.run('Math.random = () => 1');
    const max = b.run(`retryDelay(${failures})`);
    expect(min).toBe(max / 2);
    expect(max).toBeLessThanOrEqual(10000);
  }
});

test.each([false, true])('reconnect delay applies only to failure: %p', async failed => {
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
  expect(b.run('restartedAt')).toBe(failed ? null : 0);
  if (failed) {
    await b.advance(500);
    expect(b.run('restartedAt')).toBe(500);
  }
  await result;
});

test('an inactive tab does not restart after backoff', async () => {
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
});
test('an unopened media source times out and clears its timer', async () => {
  const b = browser();
  b.run('attempt.opened = new Promise(() => {})');
  const result = b.run('consume(attempt, {}, "video/mp4")').catch(e => e);
  await b.advance(10000);
  expect((await result).message).toMatch(/source did not open/);
  expect(b.timers.size).toBe(0);
});

test('live correction ignores transient lag and spaces sustained corrections apart', async () => {
  const b = browser();
  let nextRead;
  let end = 106;
  let time = 100;
  const seeks = [];
  Object.defineProperty(b.video, 'currentTime', {
    get: () => time,
    set: value => { time = value; seeks.push(value); },
  });
  b.video.getVideoPlaybackQuality = undefined;
  b.context.buffer = { buffered: { length: 1, start: () => 0, end: () => end } };
  b.context.response = { body: { getReader: () => ({
    read: () => new Promise(resolve => { nextRead = resolve; }),
  }) } };
  b.run(`
    attempt.positioned = true;
    attempt.opened = Promise.resolve();
    attempt.media = { addSourceBuffer: () => buffer };
    append = async () => {};
    reportTelemetry = async () => {};
  `);
  const result = b.run('consume(attempt, response, "video/mp4")');
  await b.advance(0);
  async function sample(ms, lag) {
    time += ms / 1000;
    await b.advance(ms);
    end = time + lag;
    nextRead({ done: false, value: new Uint8Array([1]) });
    await b.advance(0);
  }
  for (const [lag, rate] of [[1.3, 1], [2, 1], [2.01, 1.0008], [2.25, 1.02], [2.5, 1.04], [2.625, 1.05], [4, 1.05]]) {
    await sample(0, lag);
    expect(b.video.playbackRate).toBeCloseTo(rate, 6);
  }
  await sample(0, 5);
  await sample(4000, 5);
  expect(seeks).toHaveLength(0);
  await sample(0, 6);
  await sample(2000, 6);
  expect(seeks).toHaveLength(0);
  expect(b.video.playbackRate).toBe(1.05);
  await sample(0, 5);
  await sample(1000, 6);
  await sample(2000, 6);
  expect(seeks).toHaveLength(0);
  await sample(1000, 6);
  expect(seeks).toEqual([end - 1.75]);
  expect(b.video.playbackRate).toBe(1);
  await sample(1000, 6);
  await sample(3000, 6);
  await sample(5000, 6);
  expect(seeks).toHaveLength(1);
  await sample(1000, 6);
  expect(seeks).toHaveLength(2);
  for (const [target, key] of [[b.video, 'paused'], [b.video, 'seeking'], [b.document, 'hidden']]) {
    await sample(1000, 6);
    target[key] = true;
    await sample(1000, 6);
    expect(b.video.playbackRate).toBe(1);
    target[key] = false;
    await sample(10000, 6);
    const count = seeks.length;
    await sample(2000, 6);
    expect(seeks).toHaveLength(count);
    await sample(1000, 6);
    expect(seeks).toHaveLength(count + 1);
  }
  await sample(0, 1.2);
  expect(b.video.playbackRate).toBe(1);
  nextRead({ done: true });
  await result;
  expect(b.timers.size).toBe(0);
});
