// The MCP tool surface. MUST match docs/04-mcp-tool-contract.md (a contract test enforces it).
// Phase 1 M1 exposes only `status`; other tools are added milestone by milestone.
import type { CoreClient } from './core-client.ts';

export type ToolDef = {
  name: string;
  description: string;
  inputSchema: { type: 'object'; properties: Record<string, unknown>; additionalProperties: false };
  run: (core: CoreClient, args: Record<string, unknown>) => Promise<string>;
};

type Status = { version: string; session: string; client: string; state: string; uptime_ms: number };

export const TOOLS: ToolDef[] = [
  {
    name: 'status',
    description:
      'Shows whether the XtraOS core is running and paired: core version, session, and state. ' +
      'Call this first; if it fails, ask the user to start the XtraOS core.',
    inputSchema: { type: 'object', properties: {}, additionalProperties: false },
    async run(core) {
      const s = (await core.call('status')) as Status;
      // Compact one-liner (docs/06-performance-budget.md: action results <= 30 tokens).
      return `XtraOS core ${s.version} · ${s.state} · session ${s.session} (${s.client}) · up ${Math.round(s.uptime_ms / 1000)}s`;
    },
  },
];
