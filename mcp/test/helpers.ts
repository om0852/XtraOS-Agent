// Test helpers: spawn the real core binary and the real MCP server, talk MCP over stdio.
import { spawn, type ChildProcess } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
export const ROOT = path.resolve(here, '..', '..');
export const CORE_EXE = path.join(ROOT, 'core', 'target', 'debug', 'xtraos-core.exe');
export const MCP_ENTRY = path.join(ROOT, 'mcp', 'src', 'index.ts');

export type Env = { XTRAOS_PIPE: string; XTRAOS_TOKEN: string; XTRAOS_HOME: string };

export function testEnv(): Env {
  return {
    XTRAOS_PIPE: `\\\\.\\pipe\\xtraos-e2e-${crypto.randomBytes(6).toString('hex')}`,
    XTRAOS_TOKEN: crypto.randomBytes(32).toString('hex'),
    XTRAOS_HOME: fs.mkdtempSync(path.join(os.tmpdir(), 'xtraos-e2e-')),
  };
}

/** Starts xtraos-core and resolves once it prints its READY line. */
export function startCore(env: Env): Promise<ChildProcess> {
  if (!fs.existsSync(CORE_EXE)) throw new Error(`core binary missing: ${CORE_EXE} (run cargo build in core/)`);
  const child = spawn(CORE_EXE, [], { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
  return new Promise((resolve, reject) => {
    let out = '';
    let err = '';
    const timer = setTimeout(() => reject(new Error(`core not ready in 10 s\nstdout:${out}\nstderr:${err}`)), 10_000);
    child.stderr!.on('data', (d) => { err += d; });
    child.stdout!.on('data', (d) => {
      out += d;
      if (out.includes('READY ')) { clearTimeout(timer); resolve(child); }
    });
    child.once('exit', (code) => { clearTimeout(timer); reject(new Error(`core exited early (${code}): ${err}`)); });
  });
}

export async function stopProcess(child: ChildProcess | undefined): Promise<void> {
  if (!child || child.exitCode !== null) return;
  await new Promise<void>((r) => { child.once('exit', () => r()); child.kill(); });
}

/** Minimal MCP client over the server's stdio (newline-delimited JSON-RPC). */
export class McpClient {
  private child: ChildProcess;
  private buf = '';
  private waiters = new Map<number, (m: any) => void>();
  private id = 100;

  constructor(env: Partial<Env>) {
    this.child = spawn(process.execPath, [MCP_ENTRY], { env: { ...process.env, ...env }, stdio: ['pipe', 'pipe', 'pipe'] });
    this.child.stdout!.setEncoding('utf8');
    this.child.stdout!.on('data', (d: string) => {
      this.buf += d;
      let i: number;
      while ((i = this.buf.indexOf('\n')) >= 0) {
        const line = this.buf.slice(0, i).trim();
        this.buf = this.buf.slice(i + 1);
        if (!line) continue;
        const m = JSON.parse(line);
        const w = m.id !== undefined ? this.waiters.get(m.id) : undefined;
        if (w) { this.waiters.delete(m.id); w(m); }
      }
    });
  }

  rpc(method: string, params: unknown = {}): Promise<any> {
    const id = this.id++;
    this.child.stdin!.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`MCP timeout: ${method}`)), 15_000);
      this.waiters.set(id, (m) => { clearTimeout(t); resolve(m); });
    });
  }

  async init(): Promise<void> {
    const r = await this.rpc('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'xtraos-test', version: '1' } });
    if (!r.result) throw new Error(`initialize failed: ${JSON.stringify(r)}`);
    this.child.stdin!.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
  }

  async callTool(name: string, args: Record<string, unknown> = {}): Promise<{ isError: boolean; text: string }> {
    const r = await this.rpc('tools/call', { name, arguments: args });
    const res = r.result ?? {};
    return { isError: !!res.isError, text: (res.content?.[0]?.text as string) ?? JSON.stringify(r) };
  }

  async close(): Promise<void> { await stopProcess(this.child); }
}
