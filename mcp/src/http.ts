// Optional local HTTP endpoint for the MCP server (ADR 0072): Streamable HTTP on 127.0.0.1 only,
// a bearer token, Host checked (DNS rebinding), any browser Origin refused. Off by default; the
// core itself still listens on its named pipe only (CLAUDE.md §4).
import crypto from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import type { AddressInfo } from 'node:net';
import path from 'node:path';
import type { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/streamableHttp.js';

export const HTTP_PATH = '/mcp';
export const DEFAULT_PORT = 8765;
export const MAX_BODY = 4 * 1024 * 1024;

export function httpTokenPath(env: NodeJS.ProcessEnv = process.env): string {
  const home = env.XTRAOS_HOME ?? path.join(env.LOCALAPPDATA ?? '.', 'XtraOS');
  return path.join(home, 'http.token');
}

/** The endpoint's own token (not the core's pairing token): 32 random bytes, hex, made once. */
export function loadOrCreateToken(file: string): string {
  try {
    const t = fs.readFileSync(file, 'utf8').trim();
    if (/^[0-9a-f]{64}$/.test(t)) return t;
  } catch { /* first use */ }
  const t = crypto.randomBytes(32).toString('hex');
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, t, { encoding: 'utf8', mode: 0o600 });
  return t;
}

function sameSecret(a: string, b: string): boolean {
  const x = Buffer.from(a), y = Buffer.from(b);
  return x.length === y.length && crypto.timingSafeEqual(x, y);
}

export type Refusal = { status: number; message: string };

/** What a request must satisfy before anything else happens (pure). */
export function checkRequest(
  req: { url?: string; headers: Record<string, string | string[] | undefined> },
  token: string,
  port: number,
): Refusal | null {
  if (req.headers.origin !== undefined) return { status: 403, message: 'requests from web pages (Origin header) are not accepted' };
  const host = String(req.headers.host ?? '').toLowerCase();
  if (host !== `127.0.0.1:${port}` && host !== `localhost:${port}`) return { status: 403, message: `Host must be 127.0.0.1:${port}` };
  if ((req.url ?? '').split('?')[0] !== HTTP_PATH) return { status: 404, message: `the endpoint is ${HTTP_PATH}` };
  const m = /^Bearer\s+(\S+)$/i.exec(String(req.headers.authorization ?? ''));
  if (!m || !sameSecret(m[1], token)) return { status: 401, message: 'missing or wrong token (Authorization: Bearer <token from http.token>)' };
  return null;
}

/** `--http [--port N]` / XTRAOS_HTTP_PORT → port, or null for stdio (pure). */
export function httpPortFrom(argv: string[], env: NodeJS.ProcessEnv): number | null {
  const i = argv.indexOf('--http');
  const fromEnv = env.XTRAOS_HTTP_PORT;
  if (i < 0 && !fromEnv) return null;
  const pi = argv.indexOf('--port');
  const raw = pi >= 0 ? argv[pi + 1] : fromEnv && i < 0 ? fromEnv : undefined;
  const port = raw === undefined ? DEFAULT_PORT : Number(raw);
  if (!Number.isInteger(port) || port < 0 || port > 65535) throw new Error(`bad port '${raw}'`);
  return port;
}

function reply(res: http.ServerResponse, status: number, message: string) {
  res.writeHead(status, { 'content-type': 'application/json' });
  res.end(JSON.stringify({ error: message }));
}

/** Serves MCP on 127.0.0.1:port (0 = any free port); a new MCP server per request (stateless). */
export async function serveHttp(makeServer: () => Server, port: number, token: string): Promise<http.Server> {
  let actual = port;
  const srv = http.createServer(async (req, res) => {
    try {
      const bad = checkRequest(req, token, actual);
      if (bad) return reply(res, bad.status, bad.message);
      if (req.method !== 'POST') {
        res.setHeader('allow', 'POST');
        return reply(res, 405, 'POST JSON-RPC to /mcp (stateless endpoint)');
      }
      const chunks: Buffer[] = [];
      let n = 0;
      for await (const c of req) {
        n += (c as Buffer).length;
        if (n > MAX_BODY) return reply(res, 413, 'request body over 4 MB');
        chunks.push(c as Buffer);
      }
      let body: unknown;
      try {
        body = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      } catch {
        return reply(res, 400, 'body is not JSON');
      }
      const server = makeServer();
      const transport = new StreamableHTTPServerTransport({ sessionIdGenerator: undefined, enableJsonResponse: true });
      res.on('close', () => { void transport.close(); void server.close(); });
      await server.connect(transport);
      await transport.handleRequest(req, res, body);
    } catch (e) {
      if (!res.headersSent) reply(res, 500, e instanceof Error ? e.message : String(e));
    }
  });
  await new Promise<void>((ok, fail) => {
    srv.once('error', fail);
    srv.listen(port, '127.0.0.1', () => ok());
  });
  actual = (srv.address() as AddressInfo).port;
  return srv;
}
