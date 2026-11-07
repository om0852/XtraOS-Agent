// P1-REL-04 (ADR 0019): measures the RELEASE core against docs/06-performance-budget.md over its
// real pipe (the operations the budgets define), writes docs/bench/release-0.1.md.
// Opens Calculator and the testbed; types into the testbed (real keyboard): keep hands off.
// Usage: node tests/bench/bench.mjs
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..', '..');
const exe = path.join(root, 'dist', 'xtraos-0.1.0-win-x64', 'bin', 'xtraos-core.exe');
if (!fs.existsSync(exe)) throw new Error('build the bundle first: release/build-release.ps1');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const home = fs.mkdtempSync(path.join(os.tmpdir(), 'xtraos-bench-'));
const pipe = `\\\\.\\pipe\\xtraos-bench-${crypto.randomBytes(5).toString('hex')}`;
const token = crypto.randomBytes(32).toString('hex');
const core = spawn(exe, [], { env: { ...process.env, XTRAOS_HOME: home, XTRAOS_PIPE: pipe, XTRAOS_TOKEN: token }, stdio: ['ignore', 'pipe', 'pipe'] });
await new Promise((resolve, reject) => {
  let out = '';
  const t = setTimeout(() => reject(new Error('core not ready')), 10_000);
  core.stdout.on('data', (d) => { out += d; if (out.includes('READY')) { clearTimeout(t); resolve(); } });
});

const sock = net.connect(pipe);
await new Promise((r) => sock.once('connect', r));
sock.setEncoding('utf8');
let buf = '';
const waiters = new Map();
sock.on('data', (d) => {
  buf += d;
  let i;
  while ((i = buf.indexOf('\n')) >= 0) {
    const m = JSON.parse(buf.slice(0, i)); buf = buf.slice(i + 1);
    waiters.get(m.id)?.(m);
  }
});
let id = 0;
const call = (method, params = {}) => new Promise((resolve) => {
  const my = ++id;
  waiters.set(my, resolve);
  sock.write(JSON.stringify({ jsonrpc: '2.0', id: my, method, params }) + '\n');
});
const ok = async (m, p) => { const r = await call(m, p); if (r.error) throw new Error(`${m}: ${JSON.stringify(r.error)}`); return r.result; };
await ok('pair', { token, client: 'bench' });

async function timeIt(n, fn) {
  const ms = [];
  for (let i = 0; i < n; i++) { const t0 = process.hrtime.bigint(); await fn(i); ms.push(Number(process.hrtime.bigint() - t0) / 1e6); }
  ms.sort((a, b) => a - b);
  const q = (p) => ms[Math.min(ms.length - 1, Math.floor(p * ms.length))];
  return { p50: q(0.5), p95: q(0.95), n };
}
const rows = [];
const row = (op, r, b50, b95, note = '') => {
  const pass = r.p50 <= b50 && (b95 == null || r.p95 <= b95);
  rows.push({ op, ...r, b50, b95, pass, note });
  console.log(`${pass ? 'PASS' : 'MISS'} ${op}: p50 ${r.p50.toFixed(1)} ms, p95 ${r.p95.toFixed(1)} ms (budget ${b50}/${b95 ?? '-'})`);
};

// 1. pipe round trip (no-op)
row('Pipe round trip (status)', await timeIt(500, () => ok('status')), 2, 10);

// 2. observe + click(ref) + screenshot on Calculator
const calc = await ok('launch_app', { app: 'calc', timeout_ms: 15000 });
await sleep(1500);
const first = await ok('observe', { window: calc.ref, full: true });
row('observe Calculator (39 el., full)', await timeIt(30, () => ok('observe', { window: calc.ref, full: true })), 120, 400, `~${Math.ceil(first.text.length / 4)} tokens`);
const clear = first.text.split('\n').find((l) => / Button "Clear"$/.test(l)).split(' ')[0];
row('click(ref) background (Calculator "Clear")', await timeIt(30, () => ok('click', { ref: clear })), 80, 200);
const shot = await ok('screenshot', { window: calc.ref, scale: 0.5 });
row('screenshot window @0.5', await timeIt(15, () => ok('screenshot', { window: calc.ref, scale: 0.5 })), 60, 150, `${shot.width}x${shot.height} PNG ${Math.round(shot.bytes / 1024)} KB`);
await call('close_window', { window: calc.ref });

// 2b. ocr of an opaque (hand-drawn) window
const op = await ok('launch_app', { app: path.join(root, 'testbed', 'bin', 'testbed.exe'), args: ['200', '160', '--opaque'], timeout_ms: 15000 });
await sleep(800);
const firstOcr = await ok('ocr', { window: op.ref });
row('ocr opaque window', await timeIt(15, () => ok('ocr', { window: op.ref })), 400, 1000, `~${Math.ceil(firstOcr.text.length / 4)} tokens, ${firstOcr.lines} lines`);
await call('close_window', { window: op.ref });

// 3. type 50 chars (unicode mode) into the testbed text box
const tbExe = path.join(root, 'testbed', 'bin', 'testbed.exe');
const tb = await ok('launch_app', { app: tbExe, args: ['200', '160'], timeout_ms: 15000 });
await sleep(800);
const o = await ok('observe', { window: tb.ref, full: true });
const field = o.text.split('\n').find((l) => / Edit "text"/.test(l)).split(' ')[0];
await ok('focus_window', { window: tb.ref });
await ok('click', { ref: field, restore_cursor: false });
const text50 = 'XtraOS bench नमस्ते 🙂 0123456789 abcdefghijklmnopq'.slice(0, 50);
let allOk = true;
// Reset with real keys (same queue as the typing, so no race); time `type` only; read back at
// once (type returns only after the app has taken the text).
const typeMs = [];
for (let k = 0; k < 10; k++) {
  await ok('press_keys', { window: tb.ref, keys: ['Ctrl+A', 'Delete'], idle_ms: 0 });
  const t0 = process.hrtime.bigint();
  await ok('type', { window: tb.ref, text: text50, idle_ms: 0 });
  typeMs.push(Number(process.hrtime.bigint() - t0) / 1e6);
  const r = await ok('read_text', { ref: field });
  if (r.text !== text50) { allOk = false; console.log('read back right after type:', JSON.stringify(r.text)); }
}
typeMs.sort((a, b) => a - b);
row('type 50 chars (unicode), until the app has it', { p50: typeMs[5], p95: typeMs[9], n: 10 }, 150, 400, 'WinForms testbed field; read back exactly after every call');
const typedOk = allOk;
console.log(`typed text verified: ${typedOk}`);
await call('close_window', { window: tb.ref });

sock.end();
core.kill();
await sleep(300);
fs.rmSync(home, { recursive: true, force: true });

const md = ['# Bench — Release 0.1 (P1-REL-04)', '',
  `Recorded ${new Date().toISOString()} by tests/bench/bench.mjs: release \`xtraos-core.exe\` over its real named pipe, ${os.cpus()[0].model}, ${os.cpus().length} threads, Windows build ${os.release()}. Budgets from docs/06.`, '',
  '| Operation | p50 | p95 | Budget p50 / p95 | Result | Note |', '|---|---|---|---|---|---|',
  ...rows.map((r) => `| ${r.op} | ${r.p50.toFixed(1)} ms | ${r.p95.toFixed(1)} ms | ${r.b50} / ${r.b95 ?? '–'} ms | ${r.pass ? 'met' : '**missed**'} | ${r.note} |`),
  '| Kill switch → input halted | — | 15 ms | — / 100 ms | met | measured in the M5 gate (ADR 0018) |',
  '',
  `Typed text read back exactly: ${typedOk ? 'yes' : '**no**'}.`, '',
  '## Deviations (ADR 0020)', '',
  '- **type 50 chars**: since Release 0.1 `type` returns only when the app has really taken the text (before, it returned while keys were still queued, so a following observe could see half of it — found by this bench). The time is now app-bound: the WinForms testbed field processes ~13 ms per character (measured the same with and without pacing). The 150 ms budget assumed "keys sent"; it is kept as the target for fast fields and re-measured on more apps in Phase 2.',
  '- **screenshot @0.5**: 52–84 ms p50 across runs on this PC (PrintWindow + downscale + PNG), around the 60 ms budget; p95 always within 150 ms.',
  '- **observe** on windows with hundreds of list items (Explorer with 1,566 files: 170–222 ms p50) — ADR 0016, Phase 2 scoped observe.', ''];
fs.mkdirSync(path.join(root, 'docs', 'bench'), { recursive: true });
fs.writeFileSync(path.join(root, 'docs', 'bench', 'release-0.1.md'), md.join('\n'));
console.log(`bench: ${rows.filter((r) => r.pass).length}/${rows.length} within budget; typed ok=${typedOk}`);
