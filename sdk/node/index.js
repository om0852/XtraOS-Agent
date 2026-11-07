// @xtraos/sdk — Node client for the XtraOS core (ADR 0042). No dependencies.
// Pairs with the user's XtraOS core exactly like the MCP server and the CLI, so the same
// policy, approval dialogs, kill switch and action log apply. Windows only.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';

export class XtraOSError extends Error {
  /** @param {string} code @param {string} message @param {string} [hint] */
  constructor(code, message, hint = '') {
    super(`${code}: ${message}`);
    this.name = 'XtraOSError';
    this.code = code;
    this.hint = hint;
  }
}

function home(env) {
  return env.XTRAOS_HOME ?? path.join(env.LOCALAPPDATA ?? '.', 'XtraOS');
}

function resolve(opts, env) {
  const h = home(env);
  let pipe = opts.pipe ?? env.XTRAOS_PIPE;
  if (!pipe) {
    try {
      pipe = JSON.parse(fs.readFileSync(path.join(h, 'connection.json'), 'utf8')).pipe;
    } catch { /* not running yet */ }
  }
  let token = opts.token ?? env.XTRAOS_TOKEN;
  if (!token) {
    try {
      token = fs.readFileSync(path.join(h, 'pairing.token'), 'utf8').trim();
    } catch { /* not created yet */ }
  }
  return { pipe, token };
}

function coreExe(env) {
  const p = env.XTRAOS_CORE_EXE ?? path.join(env.LOCALAPPDATA ?? '.', 'Programs', 'XtraOS', 'bin', 'xtraos-core.exe');
  return fs.existsSync(p) ? p : null;
}

export class XtraOS {
  /**
   * @param {{pipe?: string, token?: string, client?: string, autostart?: boolean, timeoutMs?: number}} [opts]
   */
  constructor(opts = {}) {
    this.opts = opts;
    this.env = process.env;
    this.sock = null;
    this.buf = '';
    this.nextId = 1;
    this.pending = new Map();
  }

  async #connectOnce() {
    const { pipe, token } = resolve(this.opts, this.env);
    if (!pipe || !token) throw new XtraOSError('NOT_PAIRED', 'XtraOS core connection info or token not found');
    await new Promise((res, rej) => {
      const s = net.connect(pipe);
      s.once('connect', () => { this.sock = s; res(); });
      s.once('error', (e) => rej(new XtraOSError('NOT_PAIRED', `XtraOS core not reachable (${e.code ?? e.message})`)));
    });
    this.sock.setEncoding('utf8');
    this.sock.on('data', (d) => {
      this.buf += d;
      let i;
      while ((i = this.buf.indexOf('\n')) >= 0) {
        const line = this.buf.slice(0, i);
        this.buf = this.buf.slice(i + 1);
        let m;
        try { m = JSON.parse(line); } catch { continue; }
        const p = this.pending.get(m.id);
        if (!p) continue;
        this.pending.delete(m.id);
        if (m.error) p.reject(new XtraOSError(m.error.code ?? 'INTERNAL', m.error.message ?? 'error', m.error.hint ?? ''));
        else p.resolve(m.result);
      }
    });
    this.sock.on('close', () => {
      for (const p of this.pending.values()) p.reject(new XtraOSError('NOT_PAIRED', 'connection to the XtraOS core closed'));
      this.pending.clear();
      this.sock = null;
    });
    await this.#rpc('pair', { token, client: this.opts.client ?? 'xtraos-sdk-node' });
  }

  async #connect() {
    if (this.sock) return;
    try {
      await this.#connectOnce();
    } catch (e) {
      const exe = this.opts.autostart === false || this.opts.pipe || this.env.XTRAOS_PIPE ? null : coreExe(this.env);
      if (!exe) throw e;
      // Detached, hidden, no inherited handles: the core lives in the user's session.
      spawn(exe, [], { detached: true, stdio: 'ignore', windowsHide: true }).unref();
      const deadline = Date.now() + 10_000;
      for (;;) {
        await new Promise((r) => setTimeout(r, 200));
        try { return await this.#connectOnce(); } catch (e2) { if (Date.now() > deadline) throw e2; }
      }
    }
  }

  #rpc(method, params) {
    const id = this.nextId++;
    const timeoutMs = this.opts.timeoutMs ?? 120_000;
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => {
        this.pending.delete(id);
        reject(new XtraOSError('TIMEOUT', `no answer from the XtraOS core within ${timeoutMs} ms`));
      }, timeoutMs);
      this.pending.set(id, {
        resolve: (v) => { clearTimeout(t); resolve(v); },
        reject: (e) => { clearTimeout(t); reject(e); },
      });
      this.sock.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
    });
  }

  /**
   * Calls one XtraOS tool (same names and arguments as the MCP tools, docs/04).
   * @param {string} tool @param {Record<string, unknown>} [params]
   * @returns {Promise<any>} the tool's result; throws XtraOSError on errors.
   */
  async call(tool, params = {}) {
    await this.#connect();
    return this.#rpc(tool, params);
  }

  close() {
    this.sock?.end();
    this.sock = null;
  }
}

export default XtraOS;
