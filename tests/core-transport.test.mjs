import test from 'node:test';
import assert from 'node:assert/strict';
import { mockIPC } from '@tauri-apps/api/mocks';
import { apiGet, apiPatch, apiPut, apiPost, apiDelete, subscribeCoreStream, reconcileCoreSession } from '../src/lib/core-transport.ts';

globalThis.window = { crypto: globalThis.crypto };
const api = { id: 'session-1', transport: 'unix', port: 0, mixed_port: 7890, started_at_ms: 1 };
const tick = () => new Promise(resolve => setImmediate(resolve));

test('all controller methods use native IPC with an explicit session and structured bodies', async () => {
  const calls = [];
  mockIPC((cmd, args) => { calls.push([cmd, args]); return { ok: true }; });
  assert.deepEqual(await apiGet(api, '/proxies/a%2Fb'), { ok: true });
  await apiPatch(api, '/configs', { mode: 'global' });
  await apiPut(api, '/configs?force=true', { path: '/tmp/test.yaml' });
  await apiPost(api, '/cache/fakeip/flush');
  await apiDelete(api, '/connections/test');
  assert.deepEqual(calls.map(([cmd, a]) => [cmd, a.sessionId, a.method]),
    ['GET','PATCH','PUT','POST','DELETE'].map(method => ['core_request','session-1',method]));
  assert.equal(calls[0][1].path, '/proxies/a%2Fb');
  assert.deepEqual(calls[1][1].body, { mode: 'global' });
  assert.equal(calls[3][1].body, null);
});

test('native errors retain controller context', async () => {
  mockIPC(() => Promise.reject('/configs: HTTP 503 — not ready'));
  await assert.rejects(apiGet(api, '/configs'), error => error === '/configs: HTTP 503 — not ready');
});

test('startup failure keeps a running core addressable, including when status is temporarily unavailable', async () => {
  mockIPC(() => ({ running: true, api }));
  assert.deepEqual(await reconcileCoreSession(null), api);
  mockIPC(() => Promise.reject('IPC unavailable'));
  assert.deepEqual(await reconcileCoreSession(api), api);
  mockIPC(() => ({ running: false, api: null }));
  assert.equal(await reconcileCoreSession(api), null);
});

test('all four streams deliver data through channels and ignore packets after close', async () => {
  for (const kind of ['logs','traffic','memory','connections']) {
    let subscription;
    const data = [], errors = [], stopped = [];
    mockIPC((cmd, args) => {
      if (cmd === 'core_subscribe') subscription = args;
      if (cmd === 'core_unsubscribe') stopped.push(args.subscriptionId);
    });
    const close = subscribeCoreStream(api, kind, event => data.push(event), message => errors.push(message));
    await tick();
    subscription.onEvent.onmessage({ event:'data', data:{ kind } });
    subscription.onEvent.onmessage({ event:'error', message:'disconnected' });
    subscription.onEvent.onmessage({ event:'error', message:'retrying' });
    assert.deepEqual(errors, ['disconnected']);
    subscription.onEvent.onmessage({ event:'connected' });
    subscription.onEvent.onmessage({ event:'error', message:'disconnected again' });
    close();
    subscription.onEvent.onmessage({ event:'data', data:{ stale:true } });
    assert.deepEqual(data, [{ kind }]);
    assert.deepEqual(errors, ['disconnected','disconnected again']);
    assert.deepEqual(stopped, [subscription.subscriptionId]);
  }
});

test('closing before native registration completes also cancels the late subscription', async () => {
  let registered;
  const cancelled = [];
  mockIPC((cmd, args) => {
    if (cmd === 'core_subscribe') return new Promise(resolve => { registered = resolve; });
    if (cmd === 'core_unsubscribe') cancelled.push(args.subscriptionId);
  });
  const close = subscribeCoreStream(api, 'logs', () => assert.fail('late message'), () => assert.fail('late error'));
  close();
  registered();
  await tick();
  assert.equal(cancelled.length, 2);
  assert.equal(cancelled[0], cancelled[1]);
});
