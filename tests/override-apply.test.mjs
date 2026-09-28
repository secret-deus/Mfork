import test from 'node:test';
import assert from 'node:assert/strict';
import { createSerialQueue, confirmedMode, applyPreservingMode } from '../src/lib/core-queue.ts';
import { persistOverride } from '../src/lib/override-apply.ts';

const delay = (ms) => new Promise(r => setTimeout(r, ms));
for (const mode of ['global', 'direct', 'rule']) {
  test(`reload confirms and preserves ${mode}`, async () => {
    const calls = [];
    assert.equal(await applyPreservingMode({
      isCurrent: () => true, readMode: async () => mode,
      generate: async m => { calls.push(['generate', m]); return 'config.yaml'; },
      apply: async p => { calls.push(['apply', p]); },
      observed: m => { calls.push(['observed', m]); },
    }), true);
    assert.deepEqual(calls, [['generate', mode], ['apply', 'config.yaml'], ['observed', mode]]);
  });
}
test('unconfirmed mode never generates or applies', async () => {
  let generated = false;
  await assert.rejects(applyPreservingMode({ isCurrent: () => true, readMode: async () => null,
    generate: async () => { generated = true; return 'x'; }, apply: async () => {}, observed: () => {},
  }), /confirm/);
  assert.equal(generated, false);
  for (const mode of [undefined, null, '', 'bogus', 4]) assert.throws(() => confirmedMode(mode));
});
test('reload and mode change share strict FIFO order; failures do not poison queue', async () => {
  const enqueue = createSerialQueue(); const order = []; let mode = 'global';
  const first = enqueue(() => applyPreservingMode({
    isCurrent: () => true, readMode: async () => mode,
    generate: async m => { order.push(m); await delay(15); return 'p'; },
    apply: async () => { order.push('apply'); }, observed: () => {},
  }));
  const second = enqueue(async () => { mode = 'direct'; order.push(mode); });
  const failed = enqueue(async () => { throw new Error('test failure'); });
  const recovered = enqueue(async () => order.push('recovered'));
  await Promise.all([first, second, assert.rejects(failed, /test failure/), recovered]);
  assert.deepEqual(order, ['global', 'apply', 'direct', 'recovered']);
});
test('core exit during generation refuses stale config application', async () => {
  let current = true, applied = false;
  assert.equal(await applyPreservingMode({ isCurrent: () => current, readMode: async () => 'global',
    generate: async () => { current = false; return 'p'; }, apply: async () => { applied = true; }, observed: () => {},
  }), false);
  assert.equal(applied, false);
});
test('saved but apply failed stays saved and can retry without another write', async () => {
  let dirty = true, pending = false, writes = 0, fail = true;
  const save = () => persistOverride({
    write: dirty ? async () => { writes++; } : null,
    saved: () => { dirty = false; },
    apply: async () => { pending = true; if (fail) throw new Error('controller rejected'); pending = false; },
  });
  await assert.rejects(save(), /controller rejected/);
  assert.equal(dirty, false); assert.equal(pending, true);
  fail = false; await save();
  assert.equal(writes, 1); assert.equal(pending, false);
});
test('failed disk save remains dirty and never applies', async () => {
  let dirty = true, applied = false;
  await assert.rejects(persistOverride({ write: async () => { throw new Error('disk full'); },
    saved: () => { dirty = false; }, apply: async () => { applied = true; },
  }), /disk full/);
  assert.equal(dirty, true); assert.equal(applied, false);
});
test('disabled override saves without applying', async () => {
  let saved = false;
  await persistOverride({ write: async () => {}, saved: () => { saved = true; }, apply: null });
  assert.equal(saved, true);
});
