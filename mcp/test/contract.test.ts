// L2 contract test: the live tool list must match docs/04-mcp-tool-contract.md, so the code
// can never claim a tool the contract does not define (CLAUDE.md §2/§3).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { McpClient, ROOT } from './helpers.ts';

const contract = fs.readFileSync(path.join(ROOT, 'docs', '04-mcp-tool-contract.md'), 'utf8');
const contractTools = new Set([...contract.matchAll(/^\|\s*`([a-z_]+)`\s*\|/gm)].map((m) => m[1]));
const ALLOWED = ['ask_human', 'click', 'clipboard', 'close_window', 'drag', 'find', 'focus_window', 'launch_app', 'list_windows', 'mouse', 'observe', 'press_keys', 'read_text', 'screenshot', 'scroll', 'set_value', 'status', 'type', 'wait_for_human', 'window_bounds'];

test('tools/list exposes exactly the built tools (M1-M5)', async () => {
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
    const a = await mcp.callTool('wait_for', {});
    assert.ok(a.isError && a.text.startsWith('ERROR INVALID_ARGS'), a.text);
    const b = await mcp.callTool('click', { x: 1, y: 1, bogus: 1 });
    assert.ok(b.isError && b.text.startsWith('ERROR INVALID_ARGS'), b.text);
  } finally { await mcp.close(); }
});

// M5: there is no way for the agent to resume after the kill switch (ADR 0017).
test('no resume / unstop tool exists', async () => {
  const mcp = new McpClient({});
  try {
    await mcp.init();
    const r = await mcp.rpc('tools/list');
    const names: string[] = r.result.tools.map((t: { name: string }) => t.name);
    assert.ok(!names.some((n) => /resume|unstop|unpause|kill|policy/.test(n)), names.join(','));
  } finally { await mcp.close(); }
});
