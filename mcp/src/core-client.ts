// Pipe client for xtraos-core: line-delimited JSON-RPC 2.0, pairs with the token on connect,
// reconnects lazily on the next call if the core restarted (P1-IPC-05). Holds no other state.
import net from 'node:net';
import fs from 'node:fs';
import path from 'node:path';
import { CoreError, HINT_START_CORE } from './errors.ts';

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
      throw new CoreError('NOT_PAIRED', 'XtraOS core connection info not found', HINT_START_CORE);
    }
  }
  let token = env.XTRAOS_TOKEN;
  if (!token) {
    try {
      token = fs.readFileSync(path.join(home, 'pairing.token'), 'utf8').trim();
    } catch {
      throw new CoreError('NOT_PAIRED', 'XtraOS pairing token not found', HINT_START_CORE);
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
    return this.send(method, params);
  }

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

  private async connect(): Promise<void> {
    const { pipe, token } = this.getConnection();
    const sock = await new Promise<net.Socket>((resolve, reject) => {
      const s = net.connect(pipe);
      s.once('connect', () => resolve(s));
      s.once('error', (e: NodeJS.ErrnoException) => {
        reject(new CoreError('NOT_PAIRED', `XtraOS core is not reachable (${e.code ?? e.message})`, HINT_START_CORE));
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

  private send(method: string, params?: Record<string, unknown>): Promise<unknown> {
    const sock = this.sock;
    if (!sock) return Promise.reject(new CoreError('NOT_PAIRED', 'not connected to XtraOS core', HINT_START_CORE));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new CoreError('TIMEOUT', `core did not answer '${method}' within ${this.timeoutMs} ms`, 'Retry; check the core is responsive.'));
      }, this.timeoutMs);
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
