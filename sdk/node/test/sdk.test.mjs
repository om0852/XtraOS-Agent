// Node SDK against a private XtraOS core (no windows touched, no input).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { XtraOS, XtraOSError } from '../index.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const exe = process.env.XTRAOS_CORE_EXE ?? path.join(root, 'core', 'target', 'release', 'xtraos-core.exe');

test('SDK-N1: call status / list_windows; errors become XtraOSError', async (t) => {
  if (!fs.existsSync(exe)) return t.skip(`build the core first: ${exe}`);
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'xtraos-sdk-'));
  const pipe = String.raw`\\.\pipe\xtraos-sdk-` + crypto.randomBytes(4).toString('hex');
  const token = crypto.randomBytes(32).toString('hex');
  const core = spawn(exe, [], { env: { ...process.env, XTRAOS_HOME: home, XTRAOS_PIPE: pipe, XTRAOS_TOKEN: token }, stdio: ['ignore', 'pipe', 'pipe'] });
  try {
    await new Promise((r) => { let o = ''; core.stdout.on('data', (d) => { o += d; if (o.includes('READY')) r(); }); });
    const x = new XtraOS({ pipe, token, client: 'sdk-test' });
    const st = await x.call('status');
    assert.equal(st.state, 'running');
    assert.equal(st.client, 'sdk-test');
    const w = await x.call('list_windows');
    assert.ok(Array.isArray(w.windows));
    await assert.rejects(x.call('volume', { op: 'louder' }), (e) => e instanceof XtraOSError && e.code === 'INVALID_ARGS');
    x.close();
    const bad = new XtraOS({ pipe, token: 'f'.repeat(64), autostart: false });
    await assert.rejects(bad.call('status'), (e) => e instanceof XtraOSError);
    bad.close();
  } finally {
    core.kill();
    await new Promise((r) => setTimeout(r, 300));
    fs.rmSync(home, { recursive: true, force: true });
  }
});
