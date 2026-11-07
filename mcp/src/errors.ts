// Shared error codes (docs/04-mcp-tool-contract.md). Only codes the M1 surface can produce
// are listed; the rest arrive from the core as strings and are passed through unchanged.
export type ErrorCode = 'NOT_PAIRED' | 'INVALID_ARGS' | 'NOT_IMPLEMENTED' | 'TIMEOUT' | 'INTERNAL' | string;

export class CoreError extends Error {
  readonly code: ErrorCode;
  readonly hint: string;

  constructor(code: ErrorCode, message: string, hint = '') {
    super(message);
    this.name = 'CoreError';
    this.code = code;
    this.hint = hint;
  }
}

export const HINT_START_CORE =
  'Start the XtraOS core (xtraos-core.exe) on this machine, then retry. It creates the pipe and pairing token.';
