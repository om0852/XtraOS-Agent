#!/usr/bin/env node
// xtraos-mcp: MCP (stdio) front door for XtraOS Agent. Stateless translator: validates tool
// calls, forwards them to xtraos-core over the paired pipe, returns compact text.
// No OS logic here (CLAUDE.md §3).
import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { CallToolRequestSchema, ListToolsRequestSchema } from '@modelcontextprotocol/sdk/types.js';
import { CoreClient } from './core-client.ts';
import { CoreError } from './errors.ts';
import { INSTRUCTIONS, TOOLS, listedTools, profileFrom } from './tools.ts';

import { main as setupMain } from './setup.ts';

const VERSION = '0.1.0';

// P5-M3 (ADR 0045): `xtraos setup` / `xtraos doctor` / `xtraos help` instead of serving MCP.
if (process.argv.length > 2) process.exit(await setupMain(process.argv.slice(2)));

const core = new CoreClient();

// P5-M7 (ADR 0044): XTRAOS_TOOLS=core shortens tools/list; every tool stays callable below.
const { profile, warning } = profileFrom(process.env.XTRAOS_TOOLS);
if (warning) console.error(`[xtraos-mcp] ${warning}`);
const listed = listedTools(profile);

const server = new Server({ name: 'xtraos-agent', version: VERSION }, { capabilities: { tools: {} }, instructions: INSTRUCTIONS });

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: listed.map(({ name, description, inputSchema }) => ({ name, description, inputSchema })),
}));

function errorResult(code: string, message: string, hint: string) {
  const text = `ERROR ${code}: ${message}${hint ? `\nhint: ${hint}` : ''}`;
  return { isError: true, content: [{ type: 'text' as const, text }] };
}

server.setRequestHandler(CallToolRequestSchema, async (req) => {
  const tool = TOOLS.find((t) => t.name === req.params.name);
  if (!tool) return errorResult('INVALID_ARGS', `unknown tool '${req.params.name}'`, 'Call tools/list for available tools.');
  const args = (req.params.arguments ?? {}) as Record<string, unknown>;
  const extra = Object.keys(args).filter((k) => !(k in tool.inputSchema.properties));
  // P6-M3 (ADR 0046): name what the tool does accept.
  if (extra.length) return errorResult('INVALID_ARGS', `unexpected argument(s): ${extra.join(', ')}`, `'${tool.name}' accepts: ${Object.keys(tool.inputSchema.properties).join(', ') || '(no arguments)'}.`);
  try {
    const out = await tool.run(core, args);
    return { content: typeof out === 'string' ? [{ type: 'text' as const, text: out }] : out };
  } catch (e) {
    if (e instanceof CoreError) return errorResult(e.code, e.message, e.hint);
    return errorResult('INTERNAL', e instanceof Error ? e.message : String(e), '');
  }
});

await server.connect(new StdioServerTransport());
console.error(`[xtraos-mcp] ${VERSION} ready on stdio (tools: ${profile}, ${listed.length})`);

const shutdown = () => { core.close(); process.exit(0); };
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
process.stdin.on('end', shutdown);
