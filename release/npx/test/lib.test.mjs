import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { parseArgs, parseSums, pickAssets, sha256File, verify } from '../lib.mjs';

const H = 'a'.repeat(64);

test('SHA256SUMS lines are parsed', () => {
  const m = parseSums(`${H}  xtraos-0.1.0-win-x64.zip\r\n${'B'.repeat(64)} *xtraos-core.exe\nnot a line\n`);
  assert.equal(m.get('xtraos-0.1.0-win-x64.zip'), H);
  assert.equal(m.get('xtraos-core.exe'), 'b'.repeat(64));
  assert.equal(m.size, 2);
});

test('a file must match its listed hash', () => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), 'xi-'));
  const f = path.join(d, 'x.zip');
  fs.writeFileSync(f, 'hello');
  const good = new Map([['x.zip', sha256File(f)]]);
  assert.equal(verify(f, 'x.zip', good), sha256File(f));
  assert.throws(() => verify(f, 'x.zip', new Map([['x.zip', H]])), /checksum mismatch/);
  assert.throws(() => verify(f, 'x.zip', new Map()), /not listed/);
  fs.rmSync(d, { recursive: true, force: true });
});

test('assets only from this repository', () => {
  const base = 'https://github.com/om0852/XtraOS-Agent/releases/download/v0.1.0/';
  const r = { tag_name: 'v0.1.0', assets: [{ name: 'xtraos-0.1.0-win-x64.zip', browser_download_url: base + 'xtraos-0.1.0-win-x64.zip' }, { name: 'SHA256SUMS.txt', browser_download_url: base + 'SHA256SUMS.txt' }] };
  assert.equal(pickAssets(r).zip.name, 'xtraos-0.1.0-win-x64.zip');
  const evil = structuredClone(r);
  evil.assets[0].browser_download_url = 'https://evil.example/xtraos-0.1.0-win-x64.zip';
  assert.throws(() => pickAssets(evil), /unexpected download location/);
  assert.throws(() => pickAssets({ tag_name: 'v9', assets: [] }), /has no xtraos zip/);
});

test('options', () => {
  assert.deepEqual(parseArgs(['--version', 'v0.2.0', '--dry-run']), { version: '0.2.0', from: null, dryRun: true, help: false });
  assert.throws(() => parseArgs(['--version', 'latest']), /bad version/);
  assert.throws(() => parseArgs(['--yes']), /unknown option/);
});

// End to end on the locally built bundle, when it exists: verify only (no install).
test('local zip dry run verifies the real bundle', { skip: !fs.existsSync(new URL('../../../dist/SHA256SUMS.txt', import.meta.url)) }, () => {
  const dist = fileURLToPath(new URL('../../../dist/', import.meta.url));
  const zip = fs.readdirSync(dist).find((f) => /^xtraos-.*\.zip$/.test(f));
  const out = execFileSync(process.execPath, [fileURLToPath(new URL('../index.mjs', import.meta.url)), '--from', path.join(dist, zip), '--dry-run'], { encoding: 'utf8' });
  assert.match(out, /verified xtraos-.*\.zip/);
  assert.match(out, /dry run: not installed/);
});
