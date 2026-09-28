// Opt-in process test. Build the debug binary first; never launches the GUI.
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
import path from 'node:path';
const exe = path.resolve(process.argv[2] ?? `src-tauri/target/debug/mfork${process.platform === 'win32' ? '.exe' : ''}`);
async function run(name, script, success, headerOnly) {
  const payload = Buffer.from(JSON.stringify({ magic: 0x4D464A53, script, config: { mode: 'global' } }));
  const header = Buffer.alloc(4); header.writeUInt32LE(headerOnly ?? payload.length);
  let timedOut = false;
  const child = spawn(exe, ['--mfork-js-worker'], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  const output = [], errors = [];
  child.stdout.on('data', d => output.push(d)); child.stderr.on('data', d => errors.push(d));
  child.stdin.on('error', () => {});
  const timer = setTimeout(() => { timedOut = true; child.kill(); }, 25000);
  const closed = new Promise((resolve, reject) => { child.on('error', reject); child.on('close', (code, signal) => resolve({ code, signal })); });
  child.stdin.end(headerOnly ? header : Buffer.concat([header, payload]));
  let status;
  try { status = await closed; } finally { clearTimeout(timer); }
  const raw = Buffer.concat(output); let reply;
  if (raw.length >= 4) { assert.equal(raw.readUInt32LE(), raw.length - 4); reply = JSON.parse(raw.subarray(4)); }
  assert.equal(timedOut, false, `${name}: external watchdog fired`);
  if (success) {
    assert.ok(reply?.Ok, `${name}: ${JSON.stringify({ reply, status, stderr: Buffer.concat(errors).toString() })}`);
    assert.equal(reply.Ok.mode, 'global'); assert.equal(reply.Ok.checked, true);
  } else assert.ok(reply?.Err || status.code !== 0, `${name}: unsafe success`);
  console.log(`PASS ${name}`);
}
await run('safe transform', 'function main(c){c.checked=true;return c;}', true);
await run('large output drains pipe', 'function main(c){c.checked=true;c.text="x".repeat(200000);return c;}', true);
await run('infinite loop budget', 'function main(c){while(true){} return c;}', false);
await run('recursion budget', 'function f(){return f()} function main(c){return f()}', false);
await run('process memory cap', 'function main(c){return {text:"x".repeat(1024*1024*1024)}}', false);
await run('oversized request header', '', false, (16 << 20) + 1);
await run('reject non-object', 'function main(c){return null}', false);
