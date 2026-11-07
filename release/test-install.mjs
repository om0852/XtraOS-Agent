// P1-REL-01 (clean-profile variant, ADR 0019): extract the release zip, install into an empty
// folder with its own data dir and pipe, let the installed MCP server start the core, call
// status (must succeed within 2 minutes of starting the install), then uninstall -Purge and
// check that nothing is left (files, data, login entry, process).
// Usage: node release/test-install.mjs [path\to\xtraos-<ver>-win-x64.zip]
import { spawn, execFileSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const zip = process.argv[2] ?? fs.readdirSync(path.join(root, 'dist')).filter((f) => f.endsWith('.zip')).map((f) => path.join(root, 'dist', f))[0];
if (!zip || !fs.existsSync(zip)) throw new Error('build the bundle first: release/build-release.ps1');
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'xtraos-install-'));
const prefix = path.join(tmp, 'prefix');
const data = path.join(tmp, 'data');
const runValue = `XtraOSCoreTest${crypto.randomBytes(3).toString('hex')}`;
const ps = (args) => execFileSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', ...args], { encoding: 'utf8' });
const results = [];
const check = (name, ok, detail = '') => { results.push({ name, ok, detail }); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ' — ' + detail : ''}`); };

const t0 = Date.now();
ps(['-Command', `Expand-Archive -LiteralPath '${zip}' -DestinationPath '${tmp}\\x' -Force`]);
const bundle = path.join(tmp, 'x', fs.readdirSync(path.join(tmp, 'x'))[0]);
const out = ps(['-File', path.join(bundle, 'install.ps1'), '-Prefix', prefix, '-StartAtLogin', '-RunValueName', runValue]);
check('install.ps1 copies the bundle', fs.existsSync(path.join(prefix, 'bin', 'xtraos-core.exe')) && fs.existsSync(path.join(prefix, 'xtraos-mcp.cmd')));
check('installer prints the MCP config', out.includes('claude mcp add') && out.includes('mcpServers'));
const runKey = ps(['-Command', `(Get-ItemProperty 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run' -Name ${runValue} -ErrorAction SilentlyContinue).${runValue}`]).trim();
check('-StartAtLogin writes the login entry', runKey.includes(path.join(prefix, 'bin', 'xtraos-core.exe')), runKey);

// Talk MCP to the installed launcher exactly like a client config would.
const env = { ...process.env, XTRAOS_HOME: data, XTRAOS_PIPE: `\\\\.\\pipe\\xtraos-install-${crypto.randomBytes(5).toString('hex')}` };
delete env.XTRAOS_TOKEN;
delete env.XTRAOS_CORE_EXE;
delete env.XTRAOS_NO_AUTOSTART;
const mcp = spawn('cmd.exe', ['/d', '/s', '/c', `"${path.join(prefix, 'xtraos-mcp.cmd')}"`], { env, stdio: ['pipe', 'pipe', 'pipe'], windowsVerbatimArguments: true });
let buf = '';
const waiters = new Map();
mcp.stdout.setEncoding('utf8');
mcp.stdout.on('data', (d) => {
  buf += d;
  let i;
  while ((i = buf.indexOf('\n')) >= 0) {
    const line = buf.slice(0, i); buf = buf.slice(i + 1);
    try { const m = JSON.parse(line); waiters.get(m.id)?.(m); } catch { /* not JSON */ }
  }
});
let id = 0;
const rpc = (method, params) => new Promise((resolve, reject) => {
  const my = ++id;
  const timer = setTimeout(() => reject(new Error(`timeout: ${method}`)), 60_000);
  waiters.set(my, (m) => { clearTimeout(timer); resolve(m); });
  mcp.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: my, method, params }) + '\n');
});
let pid = 0;
try {
  await rpc('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'install-test', version: '1' } });
  mcp.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
  const st = await rpc('tools/call', { name: 'status', arguments: {} });
  const text = st.result?.content?.[0]?.text ?? JSON.stringify(st);
  const secs = (Date.now() - t0) / 1000;
  check('status ok through the installed MCP (core auto-started)', !st.result?.isError && text.includes('· running ·'), text);
  check('status within 2 minutes of starting the install', secs <= 120, `${secs.toFixed(1)} s`);
  const tools = await rpc('tools/list', {});
  check('28 tools exposed', tools.result.tools.length === 28, `${tools.result.tools.length}`);
  const conn = JSON.parse(fs.readFileSync(path.join(data, 'connection.json'), 'utf8'));
  pid = conn.pid;
  const exe = ps(['-Command', `(Get-Process -Id ${pid}).Path`]).trim();
  check('the running core is the installed binary', exe.toLowerCase() === path.join(prefix, 'bin', 'xtraos-core.exe').toLowerCase(), exe);
  check('pairing token created in the data dir (not the user\'s real one)', fs.existsSync(path.join(data, 'pairing.token')));
} finally {
  mcp.stdin.end();
  mcp.kill();
}

const un = ps(['-File', path.join(prefix, 'uninstall.ps1'), '-Prefix', prefix, '-Purge', '-DataDir', data, '-RunValueName', runValue]);
const alive = pid ? ps(['-Command', `[bool](Get-Process -Id ${pid} -ErrorAction SilentlyContinue)`]).trim() : 'False';
check('uninstall stops the core gracefully (tray Quit)', alive === 'False' && un.includes('tray Quit'), un.split('\n').find((l) => l.includes('stopped')) ?? '');
check('uninstall removes the install folder', !fs.existsSync(prefix));
check('uninstall -Purge removes the data dir', !fs.existsSync(data));
const runAfter = ps(['-Command', `[bool](Get-ItemProperty 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run' -Name ${runValue} -ErrorAction SilentlyContinue)`]).trim();
check('uninstall removes the login entry', runAfter === 'False');
fs.rmSync(tmp, { recursive: true, force: true });

const failed = results.filter((r) => !r.ok).length;
console.log(`install test: PASS=${results.length - failed} FAIL=${failed}`);
process.exit(failed ? 1 : 0);
