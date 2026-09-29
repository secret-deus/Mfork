import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { mockIPC } from '@tauri-apps/api/mocks';

test('a failed startup read retains the service session and applying TUN-off actually restarts it', async () => {
  globalThis.window = {
    crypto: globalThis.crypto, setTimeout,
    matchMedia: () => ({ matches: false, addEventListener() {} }),
  };
  globalThis.document = { documentElement: { dataset: {}, style: {} } };
  const calls = [];
  let running = false, failRead = true, serial = 0;
  let api;
  let settings = { theme:'dark', locale:'zh-CN', tun:{enable:true} };
  mockIPC((cmd, args) => {
    calls.push(cmd);
    if (cmd === 'core_start') {
      running = true;
      api = { id:`session-${++serial}`, transport:'unix', port:0, mixed_port:7890, started_at_ms:1 };
      return api;
    }
    if (cmd === 'core_status') return { running, api:running ? api : null };
    if (cmd === 'core_stop') { running = false; return true; }
    if (cmd === 'core_request') {
      if (failRead) throw new Error('/version: simulated transport interruption');
      assert.equal(args.sessionId, api.id);
      return args.path === '/version' ? {version:'test'} : {mode:'rule'};
    }
    if (cmd === 'settings_get') return settings;
    if (cmd === 'settings_set') settings = args.settings;
  });
  // SSR transforms the real store and imports without opening an HTTP server
  // or invoking any native Tauri command.
  const server = await createServer({ configFile:false, server:{middlewareMode:true, hmr:false, watch:null}, appType:'custom', optimizeDeps:{noDiscovery:true} });
  try {
    const { useApp } = await server.ssrLoadModule('/src/store.ts');
    useApp.setState({ settings });
    await useApp.getState().start();
    assert.equal(useApp.getState().phase, 'error');
    assert.equal(useApp.getState().api.id, 'session-1');
    assert.equal(running, true);
    failRead = false;
    calls.length = 0;
    await useApp.getState().applySettings({tun:{enable:false}}, 'restart');
    assert.equal(settings.tun.enable, false);
    assert.ok(calls.indexOf('core_stop') < calls.indexOf('core_start'));
    assert.ok(calls.includes('core_stop'));
    assert.equal(useApp.getState().phase, 'running');
    assert.equal(useApp.getState().api.id, 'session-2');
    assert.equal(calls.filter(c => c === 'core_subscribe').length, 4);
    await useApp.getState().stop();
    assert.equal(running, false);
    assert.equal(useApp.getState().api, null);
    assert.equal(calls.filter(c => c === 'core_unsubscribe').length, 4);
  } finally {
    await server.close();
  }
});
