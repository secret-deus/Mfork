import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeProcessRules as normalizeResult } from '../src/lib/process-rules.ts';
const normalizeProcessRules = rules => {
  const result = normalizeResult(rules);
  if (!result.ok) throw new Error(result.error.key);
  return result.rules;
};

const rule = (process_name = 'chrome.exe', target = 'DIRECT', enabled = true) => ({ process_name, target, enabled });

test('normalization trims input, keeps exact spelling and does not mutate the source', () => {
  const input = [rule(' chrome.exe ', ' 🚀 节点选择 '), rule('Steam.exe', 'REJECT', false)];
  const snapshot = structuredClone(input);
  assert.deepEqual(normalizeProcessRules(input), [rule('chrome.exe', '🚀 节点选择'), rule('Steam.exe', 'REJECT', false)]);
  assert.deepEqual(input, snapshot);
});

test('empty rules are valid and remove all managed process rules', () => {
  assert.deepEqual(normalizeProcessRules([]), []);
});

test('reject duplicate process names irrespective of case, whitespace or enabled status', () => {
  assert.throws(() => normalizeProcessRules([rule(), rule(' CHROME.EXE ', 'REJECT')]));
  assert.throws(() => normalizeProcessRules([rule(), rule('chrome.exe', 'REJECT', false)]));
});

for (const name of ['', '   ', 'C:\\apps\\chrome.exe', '/usr/bin/chrome', '*.exe', 'app?.exe', 'chrome.exe,DIRECT', 'chrome\n.exe', 'chrome\r.exe', 'chrome\u0000.exe']) {
  test(`reject invalid process basename: ${JSON.stringify(name)}`, () => {
    assert.throws(() => normalizeProcessRules([rule(name)]));
  });
}

for (const target of ['', ' ', 'A,B', 'Proxy\nMATCH,DIRECT', 'A\u0000B']) {
  test(`reject invalid rule target: ${JSON.stringify(target)}`, () => {
    assert.throws(() => normalizeProcessRules([rule('app.exe', target)]));
  });
}

test('length and count limits bound persisted configuration', () => {
  assert.throws(() => normalizeProcessRules([rule('a'.repeat(261))]));
  assert.throws(() => normalizeProcessRules([rule('app.exe', 'a'.repeat(257))]));
  assert.throws(() => normalizeProcessRules(Array.from({ length: 257 }, (_, i) => rule(`app${i}.exe`))));
  assert.equal(normalizeProcessRules(Array.from({ length: 256 }, (_, i) => rule(`app${i}.exe`))).length, 256);
});

test('Unicode executable names and explicit policies retain user order', () => {
  const input = [rule('应用.exe', '工作网络'), rule('firefox', 'DIRECT'), rule('steam.exe', 'REJECT')];
  assert.deepEqual(normalizeProcessRules(input), input);
});

test('Unicode length limits count scalar characters, matching Rust', () => {
  assert.equal(normalizeProcessRules([rule('a.exe', '🚀'.repeat(256))])[0].target, '🚀'.repeat(256));
  assert.throws(() => normalizeProcessRules([rule('a.exe', '🚀'.repeat(257))]));
});
