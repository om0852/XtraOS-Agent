#!/usr/bin/env node
// xtraos-mcp: MCP front door for XtraOS Agent (stdio; optionally local HTTP, ADR 0072).
// Stateless translator: validates tool calls, forwards them to xtraos-core over the paired pipe,
// returns compact text. No OS logic here (CLAUDE.md §3).
import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { CallToolRequestSchema, ListToolsRequestSchema } from '@modelcontextprotocol/sdk/types.js';
import { CoreClient } from './core-client.ts';
import { CoreError } from './errors.ts';
import { httpPortFrom, httpTokenPath, loadOrCreateToken, serveHttp } from './http.ts';
import { INSTRUCTIONS, TOOLS, listedTools, mcpClientName, profileFrom } from './tools.ts';

import { main as setupMain } from './setup.ts';

const VERSION = '0.1.0';
const argv = process.argv.slice(2);

// ADR 0072: `--http [--port N]` (or XTRAOS_HTTP_PORT) serves MCP on 127.0.0.1 with a token.
const httpPort = httpPortFrom(argv, process.env);

// P5-M3 (ADR 0045): `xtraos setup` / `xtraos doctor` / `xtraos help` instead of serving MCP.
if (httpPort === null && argv.length > 0) process.exit(await setupMain(argv));

// The HTTP endpoint has no per-session client name (stateless), so it pairs as `mcp-http`;
// profiles in policy.json can limit it (ADR 0060).
const core = new CoreClient(httpPort === null ? {} : { clientName: 'mcp-http' });

// P5-M7 (ADR 0044): XTRAOS_TOOLS=core shortens tools/list; every tool stays callable below.
const { profile, warning } = profileFrom(process.env.XTRAOS_TOOLS);
if (warning) console.error(`[xtraos-mcp] ${warning}`);
const listed = listedTools(profile);

function errorResult(code: string, message: string, hint: string) {
  const text = `ERROR ${code}: ${message}${hint ? `\nhint: ${hint}` : ''}`;
  return { isError: true, content: [{ type: 'text' as const, text }] };
}

/** One MCP server with every handler (stdio uses one; HTTP makes one per request). */
function makeServer(): Server {
  const server = new Server({ name: 'xtraos-agent', version: VERSION }, { capabilities: { tools: {} }, instructions: INSTRUCTIONS });
  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: listed.map(({ name, description, inputSchema }) => ({ name, description, inputSchema })),
  }));
  server.setRequestHandler(CallToolRequestSchema, async (req) => {
    const tool = TOOLS.find((t) => t.name === req.params.name);
    if (!tool) return errorResult('INVALID_ARGS', `unknown tool '${req.params.name}'`, 'Call tools/list for available tools.');
    const args = (req.params.arguments ?? {}) as Record<string, unknown>;
    const extra = Object.keys(args).filter((k) => !(k in tool.inputSchema.properties));
    // P6-M3 (ADR 0046): name what the tool does accept.
    if (extra.length) return errorResult('INVALID_ARGS', `unexpected argument(s): ${extra.join(', ')}`, `'${tool.name}' accepts: ${Object.keys(tool.inputSchema.properties).join(', ') || '(no arguments)'}.`);
    try {
      core.healthWarning = null;
      const out = await tool.run(core, args);
      const content = typeof out === 'string' ? [{ type: 'text' as const, text: out }] : out;
      // ADR 0050: every result says when the PC is under strain.
      if (core.healthWarning) content.push({ type: 'text' as const, text: `⚠ ${core.healthWarning}` });
      return { content };
    } catch (e) {
      if (e instanceof CoreError) return errorResult(e.code, e.message, e.hint);
      return errorResult('INTERNAL', e instanceof Error ? e.message : String(e), '');
    }
  });
  return server;
}

const shutdown = () => { core.close(); process.exit(0); };
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);

if (httpPort !== null) {
  const tokenFile = httpTokenPath();
  const token = loadOrCreateToken(tokenFile);
  const srv = await serveHttp(makeServer, httpPort, token);
  const addr = srv.address() as { address: string; port: number };
  console.error(`[xtraos-mcp] ${VERSION} ready on http://${addr.address}:${addr.port}/mcp (loopback only; token in ${tokenFile})`);
  // Machine-readable line for launchers and tests.
  console.log(`LISTENING http://127.0.0.1:${addr.port}/mcp`);
} else {
  const server = makeServer();
  // P8-M2 (ADR 0060): pair as the MCP client itself (e.g. `mcp:claude-code`), so per-client
  // profiles and window locks tell Claude Code, Codex and others apart.
  server.oninitialized = () => core.setClientName(mcpClientName(server.getClientVersion()?.name));
  await server.connect(new StdioServerTransport());
  console.error(`[xtraos-mcp] ${VERSION} ready on stdio (tools: ${profile}, ${listed.length})`);
  process.stdin.on('end', shutdown);
}
