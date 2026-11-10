// L2 contract test: the live tool list must match docs/04-mcp-tool-contract.md, so the code
// can never claim a tool the contract does not define (CLAUDE.md §2/§3).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { McpClient, ROOT } from './helpers.ts';

const contract = fs.readFileSync(path.join(ROOT, 'docs', '04-mcp-tool-contract.md'), 'utf8');
const contractTools = new Set([...contract.matchAll(/^\|\s*`([a-z_]+)`\s*\|/gm)].map((m) => m[1]));
const ALLOWED = ['app_map', 'ask_human', 'audit_export', 'batch', 'browser', 'cleanup', 'click', 'clipboard', 'close_window', 'drag', 'elevated_helper', 'find', 'focus_window', 'frame_diff', 'launch_app', 'list_skills', 'list_windows', 'mouse', 'observe', 'ocr', 'pen', 'press_keys', 'read_text', 'record_session', 'run_skill', 'sandbox', 'screenshot', 'scroll', 'set_value', 'status', 'system', 'touch', 'type', 'volume', 'wait_for', 'wait_for_human', 'window_bounds'];

test('tools/list exposes exactly the built tools (M1-M6, P2-M1..M3)', async () => {
  const mcp = new McpClient({});
  try {
    await mcp.init();
    const r = await mcp.rpc('tools/list');
    const names = r.result.tools.map((t: { name: string }) => t.name).sort();
    assert.deepEqual(names, ALLOWED);
  } finally { await mcp.close(); }
});

test('every exposed tool is defined in the contract doc with a description and object schema', async () => {
  const mcp = new McpClient({});
  try {
    await mcp.init();
    const r = await mcp.rpc('tools/list');
    for (const t of r.result.tools) {
      assert.ok(contractTools.has(t.name), `tool '${t.name}' is not in docs/04-mcp-tool-contract.md`);
      assert.ok(t.description && t.description.length > 20, `tool '${t.name}' needs a real description`);
      assert.equal(t.inputSchema.type, 'object');
    }
  } finally { await mcp.close(); }
});

test('unknown tool and unexpected arguments are INVALID_ARGS (no core needed)', async () => {
  const mcp = new McpClient({ XTRAOS_PIPE: '\\\\.\\pipe\\xtraos-none', XTRAOS_TOKEN: 'x' });
  try {
    await mcp.init();
    const a = await mcp.callTool('no_such_tool', {});
    assert.ok(a.isError && a.text.startsWith('ERROR INVALID_ARGS'), a.text);
    const b = await mcp.callTool('click', { x: 1, y: 1, bogus: 1 });
    assert.ok(b.isError && b.text.startsWith('ERROR INVALID_ARGS'), b.text);
    // P6-M3 (ADR 0046): the error names what the tool accepts (the session's `target:` mistake).
    const c = await mcp.callTool('click', { target: 'e12' });
    assert.ok(c.text.includes('unexpected argument(s): target') && c.text.includes("'click' accepts:") && c.text.includes('ref'), c.text);
  } finally { await mcp.close(); }
});

// M5: there is no way for the agent to resume after the kill switch (ADR 0017).
test('no resume / unstop tool exists', async () => {
  const mcp = new McpClient({});
  try {
    await mcp.init();
    const r = await mcp.rpc('tools/list');
    const names: string[] = r.result.tools.map((t: { name: string }) => t.name);
    assert.ok(!names.some((n) => /(^|_)(resume|unstop|unpause|kill)(_|$)|policy/.test(n)), names.join(','));
  } finally { await mcp.close(); }
});

// P5-M7 (ADR 0044): tool profiles and server instructions.
const CORE = ['click', 'find', 'focus_window', 'launch_app', 'list_skills', 'list_windows', 'observe', 'press_keys', 'read_text', 'run_skill', 'screenshot', 'scroll', 'status', 'type', 'wait_for'];

test('P5-M7: XTRAOS_TOOLS=core lists exactly the 15 core tools within the size budget', async () => {
  const mcp = new McpClient({ XTRAOS_TOOLS: 'core' } as any);
  try {
    await mcp.init();
    const r = await mcp.rpc('tools/list');
    assert.deepEqual(r.result.tools.map((t: { name: string }) => t.name).sort(), CORE);
    const chars = JSON.stringify(r.result.tools).length;
    assert.ok(chars <= 14_000, `core tools/list is ${chars} chars (budget 14,000 ≈ 3,500 tokens)`);
  } finally { await mcp.close(); }
});

test('P5-M7: a tool outside the core list is still callable (a profile is not a boundary)', async () => {
  const mcp = new McpClient({ XTRAOS_TOOLS: 'core', XTRAOS_PIPE: '\\\\.\\pipe\\xtraos-none', XTRAOS_TOKEN: 'x' } as any);
  try {
    await mcp.init();
    const r = await mcp.callTool('drag', { bogus: 1 });
    assert.ok(r.isError && r.text.includes('unexpected argument(s): bogus'), r.text);
  } finally { await mcp.close(); }
});

test('P5-M7: an unknown XTRAOS_TOOLS value falls back to the full list', async () => {
  const mcp = new McpClient({ XTRAOS_TOOLS: 'everything' } as any);
  try {
    await mcp.init();
    const r = await mcp.rpc('tools/list');
    assert.equal(r.result.tools.length, ALLOWED.length);
  } finally { await mcp.close(); }
});

test('P5-M7: initialize returns short server instructions', async () => {
  const mcp = new McpClient({});
  try {
    const r = await mcp.rpc('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'xtraos-test', version: '1' } });
    const text: string = r.result.instructions ?? '';
    assert.ok(text.includes('observe') && text.includes('STALE_REF') && text.includes('Ctrl+Alt+Esc'), text);
    assert.ok(text.split(/\s+/).length <= 200, `instructions are ${text.split(/\s+/).length} words (max 200)`);
  } finally { await mcp.close(); }
});
