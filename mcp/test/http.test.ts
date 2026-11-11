// ADR 0072: the optional local HTTP endpoint — loopback only, token, Host and Origin checks.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, type ChildProcess } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { checkRequest, httpPortFrom, loadOrCreateToken } from '../src/http.ts';
import { MCP_ENTRY, startCore, stopProcess, testEnv } from './helpers.ts';

const T = 'a'.repeat(64);
const ok = { host: '127.0.0.1:8765', authorization: `Bearer ${T}` };

test('HTTP-01: requests are checked before anything runs', () => {
  assert.equal(checkRequest({ url: '/mcp', headers: ok }, T, 8765), null);
  assert.equal(checkRequest({ url: '/mcp', headers: { ...ok, host: 'localhost:8765' } }, T, 8765), null);
  assert.equal(checkRequest({ url: '/mcp', headers: { ...ok, authorization: undefined } }, T, 8765)?.status, 401);
  assert.equal(checkRequest({ url: '/mcp', headers: { ...ok, authorization: `Bearer ${'b'.repeat(64)}` } }, T, 8765)?.status, 401);
  assert.equal(checkRequest({ url: '/mcp', headers: { ...ok, host: 'evil.example:8765' } }, T, 8765)?.status, 403, 'DNS rebinding');
  assert.equal(checkRequest({ url: '/mcp', headers: { ...ok, origin: 'https://evil.example' } }, T, 8765)?.status, 403, 'web pages');
  assert.equal(checkRequest({ url: '/other', headers: ok }, T, 8765)?.status, 404);
});

test('HTTP-02: off by default; --http / --port / XTRAOS_HTTP_PORT', () => {
  assert.equal(httpPortFrom([], {}), null);
  assert.equal(httpPortFrom(['setup'], {}), null);
  assert.equal(httpPortFrom(['--http'], {}), 8765);
  assert.equal(httpPortFrom(['--http', '--port', '9100'], {}), 9100);
  assert.equal(httpPortFrom([], { XTRAOS_HTTP_PORT: '9200' }), 9200);
  assert.throws(() => httpPortFrom(['--http', '--port', 'x'], {}), /bad port/);
});

test('HTTP-03: the token is made once and kept', () => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), 'xt-http-'));
  const f = path.join(d, 'http.token');
  const a = loadOrCreateToken(f);
  assert.match(a, /^[0-9a-f]{64}$/);
  assert.equal(loadOrCreateToken(f), a);
  fs.rmSync(d, { recursive: true, force: true });
});

function request(port: number, headers: Record<string, string>, body?: unknown): Promise<{ status: number; text: string }> {
  return new Promise((resolve, reject) => {
    const data = body === undefined ? undefined : JSON.stringify(body);
    const r = http.request(
      { host: '127.0.0.1', port, path: '/mcp', method: 'POST', headers: { 'content-type': 'application/json', accept: 'application/json, text/event-stream', ...headers } },
      (res) => {
        let text = '';
        res.on('data', (c) => (text += c));
        res.on('end', () => resolve({ status: res.statusCode ?? 0, text }));
      },
    );
    r.on('error', reject);
    if (data) r.write(data);
    r.end();
  });
}

test('HTTP-04: end to end — refused without token, works with it, listens on 127.0.0.1 only', async () => {
  const env = testEnv();
  const core = await startCore(env);
  let mcp: ChildProcess | undefined;
  try {
    mcp = spawn(process.execPath, [MCP_ENTRY, '--http', '--port', '0'], { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
    const url = await new Promise<string>((resolve, reject) => {
      let out = '';
      const t = setTimeout(() => reject(new Error(`no LISTENING line: ${out}`)), 15000);
      mcp!.stdout!.on('data', (c) => {
        out += c;
        const m = /LISTENING (http:\/\/127\.0\.0\.1:(\d+)\/mcp)/.exec(out);
        if (m) { clearTimeout(t); resolve(m[1]); }
      });
    });
    const port = Number(/:(\d+)\//.exec(url)![1]);
    const token = fs.readFileSync(path.join(env.XTRAOS_HOME, 'http.token'), 'utf8').trim();
    const host = `127.0.0.1:${port}`;
    const init = { jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'http-test', version: '1' } } };

    assert.equal((await request(port, { host }, init)).status, 401, 'no token');
    assert.equal((await request(port, { host, authorization: `Bearer ${'0'.repeat(64)}` }, init)).status, 401, 'wrong token');
    assert.equal((await request(port, { host: `evil.example:${port}`, authorization: `Bearer ${token}` }, init)).status, 403, 'bad Host');
    assert.equal((await request(port, { host, origin: 'https://evil.example', authorization: `Bearer ${token}` }, init)).status, 403, 'browser');

    const r1 = await request(port, { host, authorization: `Bearer ${token}` }, init);
    assert.equal(r1.status, 200, r1.text);
    assert.match(r1.text, /"serverInfo":\{"name":"xtraos-agent"/);
    const r2 = await request(port, { host, authorization: `Bearer ${token}` }, { jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'status', arguments: {} } });
    assert.equal(r2.status, 200, r2.text);
    assert.match(r2.text, /XtraOS core .* \(mcp-http\)/, 'the core sees the HTTP client as mcp-http');

    // Nothing answers on another interface address of this PC.
    const lan = Object.values(os.networkInterfaces()).flat().find((i) => i && i.family === 'IPv4' && !i.internal);
    if (lan) {
      const reached = await new Promise<boolean>((resolve) => {
        const s = http.request({ host: lan.address, port, path: '/mcp', method: 'POST', timeout: 1500 }, () => resolve(true));
        s.on('error', () => resolve(false));
        s.on('timeout', () => { s.destroy(); resolve(false); });
        s.end();
      });
      assert.equal(reached, false, `reachable on ${lan.address}`);
    }
  } finally {
    await stopProcess(mcp);
    await stopProcess(core);
    fs.rmSync(env.XTRAOS_HOME, { recursive: true, force: true });
  }
});
