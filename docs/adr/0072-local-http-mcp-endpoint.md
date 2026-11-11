# ADR 0072 — Optional local HTTP endpoint for the MCP server (loopback only, token)

- Status: accepted (owner decision "implement it", 2026-11-11; plan row P5-M8).
- Date: 2026-11-11

## Context
Some agent harnesses connect to MCP servers over HTTP only (Streamable HTTP), not stdio.
CLAUDE.md §4 forbids remote or network control and keeps the core on local IPC. The owner
decided to allow an HTTP endpoint that is local only.

## Decision
- **The core does not change**: it still listens on its named pipe only, paired by token.
- **The MCP server** (`xtraos-mcp`) can additionally serve MCP over Streamable HTTP when asked:
  `xtraos-mcp --http [--port 8765]` (or `XTRAOS_HTTP_PORT`). Off by default.
- **Loopback only**: it binds to `127.0.0.1`. Nothing else is accepted, there is no option to
  bind another address.
- **Token**: every request needs `Authorization: Bearer <token>`. The token is generated on first
  use into `<data>\http.token` (32 random bytes, hex), separate from the core's pairing token.
  Wrong or missing → 401.
- **DNS rebinding and browsers**: the `Host` header must be `127.0.0.1:<port>` or
  `localhost:<port>` (else 403). A request with an `Origin` header (a web page) is refused
  (403): the endpoint is for local programs, not websites.
- Requests are stateless; one path, `/mcp`; bodies ≤ 4 MB; the same tools and policy as stdio.
  The MCP server pairs with the core as `mcp-http` (profiles in policy.json can limit it).
- CLAUDE.md §4 is amended: the core stays IPC-only; the MCP server may listen on loopback with a
  token, opt-in (this ADR).

## Tests
- No token → 401; wrong token → 401; `Host: evil.example:port` → 403; any `Origin` → 403;
  valid token → `initialize` and `tools/list` work; the listener's address is 127.0.0.1.
