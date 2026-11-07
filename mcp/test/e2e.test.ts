// L3 integration / e2e: real MCP server <-> real xtraos-core binary over a real named pipe.
// Isolated: random pipe name, random token, temp XTRAOS_HOME (never touches the user's core).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { McpClient, startCore, stopProcess, testEnv } from './helpers.ts';

// P1-IPC-01 (end to end through MCP)
test('status works end to end: MCP -> pipe -> core', async () => {
  const env = testEnv();
  const core = await startCore(env);
  const mcp = new McpClient(env);
  try {
    await mcp.init();
    const r = await mcp.callTool('status');
    assert.equal(r.isError, false, r.text);
    assert.match(r.text, /^XtraOS core 0\.1\.0 · running · session s\d+ \(xtraos-mcp\) · up \d+s$/);
  } finally {
    await mcp.close(); await stopProcess(core); fs.rmSync(env.XTRAOS_HOME, { recursive: true, force: true });
  }
});

// P1-IPC-02 (through MCP): wrong token
test('wrong token is refused with NOT_PAIRED', async () => {
  const env = testEnv();
  const core = await startCore(env);
  const mcp = new McpClient({ ...env, XTRAOS_TOKEN: '0'.repeat(64) });
  try {
    await mcp.init();
    const r = await mcp.callTool('status');
    assert.ok(r.isError, r.text);
    assert.match(r.text, /^ERROR NOT_PAIRED/);
  } finally {
    await mcp.close(); await stopProcess(core); fs.rmSync(env.XTRAOS_HOME, { recursive: true, force: true });
  }
});

// P1-IPC-05: core down -> clear error with hint; core back -> MCP reconnects by itself.
test('core restart: NOT_PAIRED with hint while down, automatic reconnect when back', async () => {
  const env = testEnv();
  let core = await startCore(env);
  const mcp = new McpClient(env);
  try {
    await mcp.init();
    assert.equal((await mcp.callTool('status')).isError, false);

    await stopProcess(core);
    const down = await mcp.callTool('status');
    assert.ok(down.isError, down.text);
    assert.match(down.text, /^ERROR NOT_PAIRED/);
    assert.match(down.text, /hint: Start the XtraOS core/);

    core = await startCore(env);
    const up = await mcp.callTool('status');
    assert.equal(up.isError, false, up.text);
  } finally {
    await mcp.close(); await stopProcess(core); fs.rmSync(env.XTRAOS_HOME, { recursive: true, force: true });
  }
});

// P1-LOG-01 (through the real binary): the core writes an action log under XTRAOS_HOME\logs.
test('core binary writes the action log without the token', async () => {
  const env = testEnv();
  const core = await startCore(env);
  const mcp = new McpClient(env);
  try {
    await mcp.init();
    await mcp.callTool('status');
  } finally {
    await mcp.close(); await stopProcess(core);
  }
  const logsDir = `${env.XTRAOS_HOME}\\logs`;
  const files = fs.readdirSync(logsDir).filter((f) => f.endsWith('.jsonl'));
  assert.equal(files.length, 1);
  const text = fs.readFileSync(`${logsDir}\\${files[0]}`, 'utf8');
  assert.ok(!text.includes(env.XTRAOS_TOKEN), 'token leaked into log');
  const methods = text.trim().split('\n').map((l) => JSON.parse(l).method);
  assert.deepEqual(methods, ['pair', 'status']);
  // connection.json is written for the MCP server to discover the pipe
  const conn = JSON.parse(fs.readFileSync(`${env.XTRAOS_HOME}\\connection.json`, 'utf8'));
  assert.equal(conn.pipe, env.XTRAOS_PIPE);
  fs.rmSync(env.XTRAOS_HOME, { recursive: true, force: true });
});

// M4 through MCP: a screenshot comes back as an MCP image block plus the mapping line; perception
// errors keep their contract codes. (Region of 16x16 px only: reads nothing of the user's windows.)
test('screenshot returns an image block; observe errors map to contract codes', async () => {
  const env = testEnv();
  const core = await startCore(env);
  const mcp = new McpClient(env);
  try {
    await mcp.init();
    const r = await mcp.rpc('tools/call', { name: 'screenshot', arguments: { region: { x: 0, y: 0, w: 16, h: 16 }, scale: 1 } });
    const content = r.result.content as Array<{ type: string; data?: string; mimeType?: string; text?: string }>;
    assert.equal(r.result.isError ?? false, false, JSON.stringify(r.result));
    assert.equal(content[0].type, 'image');
    assert.equal(content[0].mimeType, 'image/png');
    assert.ok(Buffer.from(content[0].data!, 'base64').subarray(1, 4).toString() === 'PNG');
    assert.match(content[1].text!, /^region 16x16px · screen = \(0,0\) \+ px \/ 1$/);
    const o = await mcp.callTool('observe', { window: 'w1' });
    assert.ok(o.isError && o.text.startsWith('ERROR WINDOW_NOT_FOUND'), o.text);
    const s = await mcp.callTool('click', { ref: 'e999999' });
    assert.ok(s.isError && s.text.startsWith('ERROR STALE_REF'), s.text);
  } finally {
    await mcp.close(); await stopProcess(core); fs.rmSync(env.XTRAOS_HOME, { recursive: true, force: true });
  }
});
