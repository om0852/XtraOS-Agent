export declare class XtraOSError extends Error {
  /** Contract error code: INVALID_ARGS, BLOCKED_BY_POLICY, NEEDS_HUMAN, STOPPED, TIMEOUT, … */
  readonly code: string;
  readonly hint: string;
  constructor(code: string, message: string, hint?: string);
}

export interface XtraOSOptions {
  /** Named pipe (default: from %LOCALAPPDATA%\XtraOS\connection.json, or XTRAOS_PIPE). */
  pipe?: string;
  /** Pairing token (default: %LOCALAPPDATA%\XtraOS\pairing.token, or XTRAOS_TOKEN). */
  token?: string;
  /** Client name shown in the action log (default "xtraos-sdk-node"). */
  client?: string;
  /** Start the installed core if it is not running (default true). */
  autostart?: boolean;
  /** Per-call timeout in ms (default 120000; approval dialogs can take a while). */
  timeoutMs?: number;
}

export declare class XtraOS {
  constructor(opts?: XtraOSOptions);
  /** Calls one XtraOS tool; same names/arguments as the MCP tools (docs/04-mcp-tool-contract.md). */
  call<T = any>(tool: string, params?: Record<string, unknown>): Promise<T>;
  close(): void;
}

export default XtraOS;
