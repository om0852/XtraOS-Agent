// Pipe client for xtraos-core: line-delimited JSON-RPC 2.0, pairs with the token on connect,
// reconnects lazily on the next call if the core restarted (P1-IPC-05). Holds no other state.
import net from 'node:net';
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { CoreError, HINT_START_CORE } from './errors.ts';

/** The core is not running (no connection info, or nothing listens on the pipe). */
class Unreachable extends CoreError {}

/**
 * Where the core binary is when installed (ADR 0019): <prefix>/bin/xtraos-core.exe next to
 * <prefix>/mcp. XTRAOS_CORE_EXE overrides; XTRAOS_NO_AUTOSTART=1 disables starting it.
 */
export function coreExe(env: NodeJS.ProcessEnv = process.env): string | null {
  if (env.XTRAOS_NO_AUTOSTART === '1') return null;
  if (env.XTRAOS_CORE_EXE) return fs.existsSync(env.XTRAOS_CORE_EXE) ? env.XTRAOS_CORE_EXE : null;
  // Installed bundle: <prefix>/mcp/xtraos-mcp.mjs; source tree: <repo>/mcp/src/core-client.ts.
  for (const up of [['..'], ['..', '..']]) {
    const exe = path.resolve(import.meta.dirname, ...up, 'bin', 'xtraos-core.exe');
    if (fs.existsSync(exe)) return exe;
  }
  return null;
}

type Pending = { resolve: (v: unknown) => void; reject: (e: CoreError) => void; timer: NodeJS.Timeout };

export type Connection = { pipe: string; token: string };

/** XTRAOS_PIPE / XTRAOS_TOKEN override the files the core writes under %LOCALAPPDATA%\XtraOS. */
export function resolveConnection(env: NodeJS.ProcessEnv = process.env): Connection {
  const home = env.XTRAOS_HOME ?? path.join(env.LOCALAPPDATA ?? '.', 'XtraOS');
  let pipe = env.XTRAOS_PIPE;
  if (!pipe) {
    try {
      pipe = JSON.parse(fs.readFileSync(path.join(home, 'connection.json'), 'utf8')).pipe as string;
    } catch {
      throw new Unreachable('NOT_PAIRED', 'XtraOS core connection info not found', HINT_START_CORE);
    }
  }
  let token = env.XTRAOS_TOKEN;
  if (!token) {
    try {
      token = fs.readFileSync(path.join(home, 'pairing.token'), 'utf8').trim();
    } catch {
      throw new Unreachable('NOT_PAIRED', 'XtraOS pairing token not found', HINT_START_CORE);
    }
  }
  return { pipe, token };
}

export class CoreClient {
  private sock: net.Socket | null = null;
  private connecting: Promise<void> | null = null;
  private buf = '';
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private readonly clientName: string;
  private readonly getConnection: () => Connection;
  private readonly timeoutMs: number;

  constructor(opts: { clientName?: string; getConnection?: () => Connection; timeoutMs?: number } = {}) {
    this.clientName = opts.clientName ?? 'xtraos-mcp';
    this.getConnection = opts.getConnection ?? (() => resolveConnection());
    this.timeoutMs = opts.timeoutMs ?? 10_000;
  }

  async call(method: string, params?: Record<string, unknown>): Promise<unknown> {
    await this.ensureConnected();
    // A call may legitimately take up to its own timeout_ms (waiting for the user to be idle,
    // waiting for a launched window); allow that plus a margin.
    const t = typeof params?.timeout_ms === 'number' ? params.timeout_ms + 5_000 : 0;
    const r = await this.send(method, params, Math.max(this.timeoutMs, t));
    // ADR 0050: the core adds `health_warning` while the PC is under strain.
    const w = (r as { health_warning?: unknown } | null)?.health_warning;
    if (typeof w === 'string') this.healthWarning = w;
    return r;
  }

  /** The last health warning the core sent during the current tool call (ADR 0050). */
  healthWarning: string | null = null;

  close(): void {
    this.sock?.destroy();
    this.sock = null;
  }

  private ensureConnected(): Promise<void> {
    if (this.sock && !this.sock.destroyed) return Promise.resolve();
    if (!this.connecting) {
      this.connecting = this.connect().finally(() => { this.connecting = null; });
    }
    return this.connecting;
  }

  private lastAutostart = 0;

  /** Connects; if the core is not running and its binary is installed, starts it once (ADR 0019). */
  private async connect(): Promise<void> {
    try {
      return await this.connectOnce();
    } catch (e) {
      const exe = coreExe();
      if (!(e instanceof Unreachable) || !exe || Date.now() - this.lastAutostart < 30_000) throw e;
      this.lastAutostart = Date.now();
      // Detached and hidden: the core lives in the user's session (tray icon, kill switch)
      // and outlives this MCP server.
      spawn(exe, [], { detached: true, stdio: 'ignore', windowsHide: true }).unref();
      const deadline = Date.now() + 10_000;
      for (;;) {
        await new Promise((r) => setTimeout(r, 200));
        try {
          return await this.connectOnce();
        } catch (e2) {
          if (!(e2 instanceof Unreachable) || Date.now() > deadline) throw e2;
        }
      }
    }
  }

  private async connectOnce(): Promise<void> {
    const { pipe, token } = this.getConnection();
    const sock = await new Promise<net.Socket>((resolve, reject) => {
      const s = net.connect(pipe);
      s.once('connect', () => resolve(s));
      s.once('error', (e: NodeJS.ErrnoException) => {
        reject(new Unreachable('NOT_PAIRED', `XtraOS core is not reachable (${e.code ?? e.message})`, HINT_START_CORE));
      });
    });
    this.sock = sock;
    this.buf = '';
    sock.setEncoding('utf8');
    sock.on('data', (chunk: string) => this.onData(chunk));
    sock.on('error', () => { /* surfaced via 'close' */ });
    sock.on('close', () => {
      if (this.sock === sock) this.sock = null;
      this.failAll(new CoreError('NOT_PAIRED', 'connection to XtraOS core closed', HINT_START_CORE));
    });
    try {
      await this.send('pair', { token, client: this.clientName });
    } catch (e) {
      this.close();
      throw e;
    }
  }

  private send(method: string, params?: Record<string, unknown>, timeoutMs = this.timeoutMs): Promise<unknown> {
    const sock = this.sock;
    if (!sock) return Promise.reject(new CoreError('NOT_PAIRED', 'not connected to XtraOS core', HINT_START_CORE));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new CoreError('TIMEOUT', `core did not answer '${method}' within ${timeoutMs} ms`, 'Retry; check the core is responsive.'));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      const msg: Record<string, unknown> = { jsonrpc: '2.0', id, method };
      if (params !== undefined) msg.params = params;
      sock.write(JSON.stringify(msg) + '\n');
    });
  }

  private onData(chunk: string): void {
    this.buf += chunk;
    let i: number;
    while ((i = this.buf.indexOf('\n')) >= 0) {
      const line = this.buf.slice(0, i);
      this.buf = this.buf.slice(i + 1);
      let msg: { id?: number; result?: unknown; error?: { code?: string; message?: string; hint?: string } };
      try { msg = JSON.parse(line); } catch { continue; }
      const p = msg.id === undefined ? undefined : this.pending.get(msg.id);
      if (!p) continue;
      this.pending.delete(msg.id as number);
      clearTimeout(p.timer);
      if (msg.error) p.reject(new CoreError(msg.error.code ?? 'INTERNAL', msg.error.message ?? 'core error', msg.error.hint ?? ''));
      else p.resolve(msg.result);
    }
  }

  private failAll(err: CoreError): void {
    for (const [, p] of this.pending) { clearTimeout(p.timer); p.reject(err); }
    this.pending.clear();
  }
}
