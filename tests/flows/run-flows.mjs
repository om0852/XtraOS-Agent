// P1-REL-02 (ADR 0019): records flows F2, F3, F4, F6, F10 by driving the RELEASE bundle through
// MCP exactly as an agent would (tools/call only). Writes transcripts to docs/flows/.
// It opens Calculator, Chrome, Firefox, Explorer, Windows Terminal and the testbed, and moves
// the real mouse in F4/F10: keep hands off. Uses its own data dir + pipe (not the user's core).
// Usage: node tests/flows/run-flows.mjs [F2 F3 ...]
import { spawn, execFileSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..', '..');
const bundle = path.join(root, 'dist', 'xtraos-0.1.1-win-x64');
const launcher = path.join(bundle, 'xtraos-mcp.cmd');
if (!fs.existsSync(launcher)) throw new Error('build the bundle first: release/build-release.ps1');
const outDir = path.join(root, 'docs', 'flows');
fs.mkdirSync(outDir, { recursive: true });
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'xtraos-flows-'));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tokens = (s) => Math.ceil([...s].length / 4);
const ps = (cmd) => execFileSync('powershell', ['-NoProfile', '-Command', cmd], { encoding: 'utf8' });
// One core for all flows (one data dir + pipe), started by the first MCP server on demand.
const PIPE = `\\\\.\\pipe\\xtraos-flows-${crypto.randomBytes(5).toString('hex')}`;

// ------------------------------------------------------------------ MCP client (stdio)
class Mcp {
  constructor() {
    const env = { ...process.env, XTRAOS_HOME: path.join(tmp, 'data'), XTRAOS_PIPE: PIPE };
    for (const k of ['XTRAOS_TOKEN', 'XTRAOS_CORE_EXE', 'XTRAOS_NO_AUTOSTART']) delete env[k];
    this.home = env.XTRAOS_HOME;
    // cmd /s strips one pair of quotes: double-quote so paths with spaces survive.
    this.p = spawn('cmd.exe', ['/d', '/s', '/c', `""${launcher}""`], { env, stdio: ['pipe', 'pipe', 'pipe'], windowsVerbatimArguments: true });
    this.buf = ''; this.id = 0; this.waiters = new Map();
    this.p.stdout.setEncoding('utf8');
    this.p.stdout.on('data', (d) => {
      this.buf += d;
      let i;
      while ((i = this.buf.indexOf('\n')) >= 0) {
        const line = this.buf.slice(0, i); this.buf = this.buf.slice(i + 1);
        try { const m = JSON.parse(line); this.waiters.get(m.id)?.(m); } catch { /* ignore */ }
      }
    });
  }
  rpc(method, params, timeout = 90_000) {
    return new Promise((resolve, reject) => {
      const id = ++this.id;
      const t = setTimeout(() => reject(new Error(`timeout ${method}`)), timeout);
      this.waiters.set(id, (m) => { clearTimeout(t); resolve(m); });
      this.p.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
    });
  }
  async init() {
    await this.rpc('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'flow-recorder', version: '0.1' } });
    this.p.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
  }
  close() {
    this.p.stdin.end();
    try { execFileSync('taskkill', ['/PID', String(this.p.pid), '/T', '/F'], { stdio: 'ignore' }); } catch { /* gone */ }
  }
}

// ------------------------------------------------------------------ transcript
class Flow {
  constructor(mcp, id, title) { this.mcp = mcp; this.id = id; this.title = title; this.steps = []; this.ok = true; this.notes = []; }
  async call(name, args = {}, why = '') {
    const t0 = Date.now();
    const r = await this.mcp.rpc('tools/call', { name, arguments: args });
    const ms = Date.now() - t0;
    const content = r.result?.content ?? [];
    const text = content.map((c) => (c.type === 'text' ? c.text : `[image ${c.mimeType}, ${Math.round((c.data?.length ?? 0) * 0.75 / 1024)} KB]`)).join('\n');
    const isError = !!r.result?.isError;
    this.steps.push({ name, args, why, text, ms, isError, tok: tokens(text) });
    return { text, isError };
  }
  note(s) { this.notes.push(s); }
  expect(cond, what) { if (!cond) { this.ok = false; this.notes.push(`FAILED: ${what}`); } return cond; }
  write() {
    const calls = this.steps.length;
    const tok = this.steps.reduce((a, s) => a + s.tok, 0);
    const ms = this.steps.reduce((a, s) => a + s.ms, 0);
    const show = (a) => JSON.stringify(a).replace(/"text":"[^"]*"/, (m) => m); // args are already free of secrets
    const lines = [`# ${this.id} — ${this.title}`, '', `Recorded ${new Date().toISOString()} by tests/flows/run-flows.mjs against the release bundle (MCP stdio → core).`, '',
      `**Result: ${this.ok ? 'PASS' : 'FAIL'}** · ${calls} tool calls · ~${tok} result tokens · ${(ms / 1000).toFixed(1)} s in tools`, ''];
    if (this.notes.length) lines.push(...this.notes.map((n) => `> ${n}`), '');
    lines.push('| # | Tool | Arguments | ms | ~tok | Result |', '|---|---|---|---|---|---|');
    this.steps.forEach((s, i) => {
      const res = s.text.replace(/\|/g, '\\|').replace(/\n/g, '<br>');
      lines.push(`| ${i + 1} | \`${s.name}\`${s.why ? ` — ${s.why}` : ''} | \`${show(s.args).replace(/\|/g, '\\|')}\` | ${s.ms} | ${s.tok} | ${s.isError ? '**error** ' : ''}${res.length > 900 ? res.slice(0, 900) + '…' : res} |`);
    });
    fs.writeFileSync(path.join(outDir, `${this.id}.md`), lines.join('\n') + '\n');
    return { id: this.id, title: this.title, ok: this.ok, calls, tok, ms };
  }
}

const firstRef = (text, re) => text.split('\n').find((l) => re.test(l))?.split(' ')[0];
const winRef = (text) => text.match(/\bw\d+\b/)?.[0];

// ------------------------------------------------------------------ flows
async function F2(f) {
  const l = await f.call('launch_app', { app: 'calc' }, 'open Calculator');
  const w = winRef(l.text);
  await sleep(1200);
  const o = await f.call('observe', { window: w }, 'read it (refs)');
  const ref = (n) => firstRef(o.text, new RegExp(`^e\\d+ Button "${n}"`));
  for (const n of ['Seven', 'Plus', 'Five', 'Equals']) {
    const r = ref(n);
    if (!f.expect(r, `button ${n} in observe`)) return;
    await f.call('click', { ref: r }, `press ${n}`);
  }
  const d = await f.call('observe', { window: w }, 'diff after the clicks');
  f.expect(/Display is 12/.test(d.text), 'display shows 12');
  await f.call('close_window', { window: w }, 'done');
}

/** Re-checks the window title like an agent waiting for a page (≤ 4 × 2 s; network-bound). */
async function titleShows(f, w, query, what) {
  for (let i = 0; i < 4; i++) {
    await sleep(2000);
    const l = await f.call('list_windows', { filter: query }, i ? 'still loading? check again' : 'results loaded?');
    if (l.text.includes(w)) return true;
  }
  const t = await f.call('list_windows', {}, `what does ${what} show instead?`);
  f.note(`${what} title instead: ${t.text.split('\n').find((l) => l.startsWith(w)) ?? '(window gone)'}`);
  return false;
}

async function F3(f) {
  const c = await f.call('launch_app', { app: 'chrome', args: ['--new-window', 'https://www.google.com/search?q=xtraos+agent'] }, 'search in Chrome');
  const cw = winRef(c.text);
  f.expect(await titleShows(f, cw, 'xtraos agent', 'Chrome'), 'Chrome title shows the query');
  await f.call('close_window', { window: cw }, 'close Chrome window');
  const ff = await f.call('launch_app', { app: 'firefox', args: ['-new-window', 'about:blank'] }, 'Firefox, no extension');
  const fw = winRef(ff.text);
  await sleep(2500);
  const fo = await f.call('find', { window: fw, query: 'enter address' }, 'address bar');
  const bar = firstRef(fo.text, /^e\d+ /);
  if (!f.expect(bar, 'address bar found')) return;
  // The address bar is a ComboBox: a background "click" would just expand its list, so the
  // agent gives Firefox the keyboard, selects the bar the standard way and types.
  await f.call('focus_window', { window: fw }, 'keys go to Firefox');
  await f.call('press_keys', { window: fw, keys: ['Ctrl+L'] }, 'select the address bar');
  await f.call('type', { window: fw, text: 'xtraos agent', submit: true }, 'search');
  f.expect(await titleShows(f, fw, 'xtraos agent', 'Firefox'), 'Firefox title shows the query');
  await f.call('close_window', { window: fw }, 'close Firefox window');
}

async function F4(f) {
  const dir = path.join(tmp, 'f4');
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'report.txt'), 'quarterly report');
  const page = path.join(tmp, 'dropzone.html');
  fs.writeFileSync(page, `<!doctype html><title>Drop zone</title><body style="font:20px sans-serif">
<div id="z" role="region" aria-label="Drop files here" style="margin:40px;height:300px;border:4px dashed #888;display:flex;align-items:center;justify-content:center">Drop files here</div>
<script>const z=document.getElementById('z');
['dragenter','dragover'].forEach(e=>z.addEventListener(e,ev=>{ev.preventDefault();}));
document.addEventListener('dragover',e=>e.preventDefault());document.addEventListener('drop',e=>e.preventDefault());
z.addEventListener('drop',ev=>{ev.preventDefault();const n=[...ev.dataTransfer.files].map(f=>f.name).join(',');document.title='dropped: '+n;z.textContent='Received '+n;});</script>`);
  const c = await f.call('launch_app', { app: 'chrome', args: ['--new-window', `file:///${page.replace(/\\/g, '/')}`] }, 'web page with a drop zone');
  const cw = winRef(c.text);
  await f.call('window_bounds', { window: cw, x: 60, y: 80, w: 820, h: 640 }, 'left half');
  const e = await f.call('launch_app', { app: 'explorer', args: [dir] }, 'folder with the file');
  const ew = winRef(e.text);
  await f.call('window_bounds', { window: ew, x: 920, y: 80, w: 820, h: 640 }, 'right half');
  await sleep(1800);
  const fe = await f.call('find', { window: ew, query: 'report' }, 'the file');
  const file = firstRef(fe.text, /^e\d+ (ListItem|DataItem|Edit|Text)/) ?? firstRef(fe.text, /^e\d+ /);
  await f.call('observe', { window: cw }, 'read the page (Chrome turns on its accessibility tree)');
  const fz = await f.call('find', { window: cw, query: 'Drop files here' }, 'the drop zone');
  const zone = firstRef(fz.text, /^e\d+ /);
  if (!f.expect(file && zone, 'file and drop zone found')) return;
  // Both ends must be visible on screen for a real drag: bring both windows to the front.
  await f.call('focus_window', { window: cw }, 'show the page');
  await f.call('focus_window', { window: ew }, 'show the folder');
  await f.call('drag', { from_ref: file, to_ref: zone, steps: 30, step_ms: 15, hold_ms: 200 }, 'real OS drag-and-drop');
  await sleep(800);
  const lw = await f.call('list_windows', { filter: 'dropped' }, 'page received it?');
  f.expect(lw.text.includes('dropped: report.txt'), 'page title says dropped: report.txt');
  await f.call('close_window', { window: ew }, 'close Explorer');
  await f.call('close_window', { window: cw }, 'close Chrome');
}

async function F6(f) {
  f.note('Claude Code (`claude`) is not installed on the recording PC; the same terminal flow runs `node --version` (no paid model call).');
  const t = await f.call('launch_app', { app: 'wt', args: ['-w', 'new', '-d', tmp] }, 'Windows Terminal');
  const tw = winRef(t.text);
  await sleep(2500);
  await f.call('focus_window', { window: tw }, 'make sure keys go there');
  await f.call('type', { window: tw, text: 'node --version', submit: true }, 'run a command');
  await sleep(1500);
  const r = await f.call('read_text', { window: tw, last_lines: 6 }, 'read the output (UIA TextPattern)');
  f.expect(/v24\.\d+\.\d+/.test(r.text), 'terminal shows the Node version');
  const x = await f.call('type', { window: tw, text: 'exit', submit: true }, 'close the shell (closes the window)');
  if (x.isError) await f.call('close_window', { window: tw }, 'close the terminal instead');
}

// F8 (P2-M2, ADR 0024): an opaque window — observe finds nothing, OCR finds the text to click.
async function F8(f) {
  const exe = path.join(root, 'testbed', 'bin', 'testbed.exe');
  const l = await f.call('launch_app', { app: exe, args: ['200', '160', '--opaque'] }, 'a hand-drawn app (like a game)');
  const w = winRef(l.text);
  const o = await f.call('batch', { steps: [
    { tool: 'wait_for', args: { window: w, idle_ms: 300 } },
    { tool: 'observe', args: { window: w, full: true } },
  ] }, 'read it the normal way');
  f.expect(o.text.includes('opaque'), 'observe reports opaque');
  const q = await f.call('ocr', { window: w, query: 'start game' }, 'find the text by OCR');
  const m = q.text.match(/@(\d+),(\d+)/);
  if (!f.expect(m, 'OCR found "Start Game"')) return;
  await f.call('click', { x: Number(m[1]), y: Number(m[2]), window: w }, 'click the text');
  const v = await f.call('ocr', { window: w, query: 'last click: start game' }, 'did the app react?');
  f.expect(/1 matching lines/.test(v.text), 'the app shows "Last click: Start Game"');
  await f.call('close_window', { window: w }, 'clean up');
}

// S1 (P2-M3, ADR 0026): a routine file task in ONE call through a macro skill.
async function S1(f) {
  const data = path.join(tmp, 'data');
  fs.mkdirSync(path.join(data, 'skills'), { recursive: true });
  fs.writeFileSync(path.join(data, 'skills', 'reports.json'), JSON.stringify({
    name: 'reports.file', description: 'put files into a new folder and show it',
    args: { files: {}, folder: {} },
    steps: [
      { tool: 'run_skill', args: { name: 'files.mkdir', args: { path: '${folder}' } } },
      { tool: 'run_skill', args: { name: 'files.copy', args: { from: '${files}', to_dir: '${folder}' } } },
      { tool: 'run_skill', args: { name: 'files.open', args: { path: '${folder}' } } },
    ],
  }));
  const src = path.join(tmp, 'q3-report.pdf');
  fs.writeFileSync(src, 'report');
  const folder = path.join(tmp, 'Q3 Reports');
  const l = await f.call('list_skills', { app: 'reports' }, 'which skill?');
  f.expect(l.text.includes('reports.file('), 'macro skill listed');
  const r = await f.call('run_skill', { name: 'reports.file', args: { files: [src], folder } }, 'ONE call: new folder + copy + open');
  f.expect(r.text.startsWith('ok skill 3/3'), 'all 3 steps ok');
  f.expect(fs.existsSync(path.join(folder, 'q3-report.pdf')), 'file copied');
  const w = await f.call('wait_for', { window_title: 'Q3 Reports', timeout_ms: 8000 }, 'Explorer shows the folder');
  const ref = w.text.match(/\bw\d+\b/)?.[0];
  if (f.expect(ref, 'Explorer window found')) await f.call('close_window', { window: ref }, 'clean up');
}

// P2-BENCH-01 (ADR 0022): a routine task in ≤ 3 calls / ≤ 2,000 tokens with wait_for + batch.
async function R1(f) {
  const l = await f.call('launch_app', { app: 'calc' }, 'call 1: open Calculator');
  const w = winRef(l.text);
  const o = await f.call('batch', { steps: [
    { tool: 'wait_for', args: { window: w, idle_ms: 400 } },
    { tool: 'observe', args: { window: w, full: true } },
  ] }, 'call 2: wait until it settled, read it (refs)');
  const ref = (n) => firstRef(o.text, new RegExp(`^e\\d+ Button "${n}"`));
  const keys = ['Clear', 'Seven', 'Plus', 'Five', 'Equals'].map(ref);
  if (!f.expect(keys.every(Boolean), 'all buttons found')) return;
  const b = await f.call('batch', { steps: [
    ...keys.map((r) => ({ tool: 'click', args: { ref: r } })),
    { tool: 'wait_for', args: { window: w, text_contains: 'Display is 12' } },
    { tool: 'close_window', args: { window: w } },
  ] }, 'call 3: press 7 + 5 =, wait for 12, close');
  f.expect(b.text.startsWith('ok batch 7/7'), 'all steps ok and the display showed 12');
  const calls = f.steps.length;
  const tok = f.steps.reduce((a, s) => a + s.tok, 0);
  f.note(`Routine task budget (docs/06): ${calls} calls (≤ 3), ~${tok} result tokens (≤ 2,000).`);
  f.expect(calls <= 3 && tok <= 2000, 'within ≤ 3 calls and ≤ 2,000 tokens');
}

async function F10(f) {
  const exe = path.join(root, 'testbed', 'bin', 'testbed.exe');
  const l = await f.call('launch_app', { app: exe, args: ['200', '160'] }, 'drawing surface (testbed)');
  const w = winRef(l.text);
  await sleep(800);
  const s = await f.call('screenshot', { window: w, scale: 0.3 }, 'look');
  const b = await f.call('window_bounds', { window: w });
  const m = b.text.match(/(-?\d+),(-?\d+) (\d+)x(\d+)/);
  const [x, y] = [Number(m[1]) + 40, Number(m[2]) + 420];
  const pts = Array.from({ length: 400 }, (_, i) => [x + (i % 100) * 3, y + Math.floor(i / 100) * 15]);
  // The human presses Ctrl+Alt+Esc 0.8 s into a 6 s drag.
  const hotkey = sleep(800).then(() => ps(`Add-Type -Namespace K -Name I -MemberDefinition '[DllImport("user32.dll")] public static extern void keybd_event(byte v, byte s, uint f, UIntPtr e);'; foreach ($k in 0x11,0x12,0x1B) { [K.I]::keybd_event($k,0,0,[UIntPtr]::Zero) }; foreach ($k in 0x1B,0x12,0x11) { [K.I]::keybd_event($k,0,2,[UIntPtr]::Zero) }`));
  const d = await f.call('mouse', { action: 'path', points: pts, step_ms: 15, window: w }, 'long drag (user presses Ctrl+Alt+Esc)');
  await hotkey;
  f.expect(d.text.startsWith('ERROR STOPPED'), 'drag stopped by the kill switch');
  const st = await f.call('status', {}, 'what now?');
  f.expect(st.text.includes('· stopped ·'), 'status says stopped');
  const o = await f.call('observe', { window: w }, 'agent tries to continue');
  f.expect(o.text.startsWith('ERROR STOPPED'), 'refused while stopped');
  // A program posting the tray "Resume" command (WM_COMMAND 1002) must NOT resume (ADR 0041):
  // only the user's real click on the tray menu does, which belongs to the human pass.
  const pid = JSON.parse(fs.readFileSync(path.join(f.mcp.home, 'connection.json'), 'utf8')).pid;
  ps(`Add-Type -Namespace T -Name W -MemberDefinition '[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr p, IntPtr a, string c, string t); [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid); [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, UIntPtr w, IntPtr l);'; $h=[IntPtr]::Zero; while (($h=[T.W]::FindWindowExW([IntPtr]::Zero,$h,'XtraOSCoreUi',[NullString]::Value)) -ne [IntPtr]::Zero) { [uint32]$p=0; [void][T.W]::GetWindowThreadProcessId($h,[ref]$p); if ($p -eq ${pid}) { [void][T.W]::PostMessageW($h,0x111,[UIntPtr]::new([uint64]1002),[IntPtr]::Zero) } }`);
  await sleep(300);
  const st2 = await f.call('status', {}, 'after a program posted Resume');
  f.expect(st2.text.includes('· stopped ·'), 'a posted Resume is ignored: still stopped');
  f.note('Resuming needs the user\'s real click on the tray menu (human pass); the recorder only checks that a posted Resume does nothing.');
  // The core stays stopped, so it cannot close the window: end the testbed directly.
  ps(`Get-Process testbed -ErrorAction SilentlyContinue | Stop-Process -Force`);
  void s;
}

const FLOWS = {
  F2: ['Observe → act loop (Calculator, background clicks)', F2],
  F3: ['Chrome search → close → Firefox (no extension) search → close', F3],
  F4: ['Drag a file from Explorer onto a web page drop zone (refs)', F4],
  F6: ['Terminal: run a command and read its output', F6],
  F10: ['Kill switch during a long drag; a posted Resume is ignored', F10],
  F8: ['Opaque (hand-drawn) window: observe finds nothing, OCR finds and clicks the text', F8],
  S1: ['Routine file task in ONE call: macro skill (new folder + copy + open)', S1],
  R1: ['Routine task in ≤ 3 calls: Calculator 7 + 5 with wait_for + batch (P2-BENCH-01)', R1],
};

const pick = process.argv.slice(2).length ? process.argv.slice(2) : Object.keys(FLOWS);
const summary = [];
for (const id of pick) {
  const mcp = new Mcp();
  await mcp.init();
  const f = new Flow(mcp, id, FLOWS[id][0]);
  try { await FLOWS[id][1](f); } catch (e) { f.expect(false, `exception: ${e.message}`); }
  // Close whatever this flow opened and left behind (e.g. after a failed step); never the
  // user's windows (ADR 0028). Not part of the recorded transcript.
  try {
    const c = await mcp.rpc('tools/call', { name: 'cleanup', arguments: {} }, 20_000);
    const t = (c.result?.content ?? []).map((x) => x.text ?? '').join(' ');
    if (!/^closed 0:/.test(t)) console.log(`  cleanup after ${id}: ${t.replace(/\n/g, ' / ')}`);
  } catch { /* core gone */ }
  summary.push(f.write());
  console.log(`${f.ok ? 'PASS' : 'FAIL'} ${id} ${FLOWS[id][0]} (${f.steps.length} calls)${f.notes.length ? '\n  ' + f.notes.join('\n  ') : ''}`);
  mcp.close();
  await sleep(500);
}
// stop the core this recorder started (its own data dir), then clean up
try {
  const pid = JSON.parse(fs.readFileSync(path.join(tmp, 'data', 'connection.json'), 'utf8')).pid;
  ps(`Stop-Process -Id ${pid} -Force -ErrorAction SilentlyContinue`);
} catch { /* not started */ }
await sleep(500);
try { fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 5, retryDelay: 300 }); } catch (e) { console.log(`note: could not remove ${tmp}: ${e.code}`); }
const idx = ['# Recorded flows (Release 0.1)', '', 'P1-REL-02: each flow drives the release bundle through MCP exactly as an agent would. Re-record: `node tests/flows/run-flows.mjs`.', '',
  '| Flow | What | Result | Calls | ~Result tokens | Time in tools |', '|---|---|---|---|---|---|',
  ...summary.map((s) => `| [${s.id}](${s.id}.md) | ${s.title} | ${s.ok ? 'PASS' : 'FAIL'} | ${s.calls} | ${s.tok} | ${(s.ms / 1000).toFixed(1)} s |`)];
if (pick.length === Object.keys(FLOWS).length) fs.writeFileSync(path.join(outDir, 'README.md'), idx.join('\n') + '\n');
const failed = summary.filter((s) => !s.ok).length;
console.log(`flows: PASS=${summary.length - failed} FAIL=${failed}`);
process.exit(failed ? 1 : 0);
