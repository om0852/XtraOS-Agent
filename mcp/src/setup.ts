// P5-M3 (ADR 0045): `xtraos setup` writes the XtraOS MCP entry into each installed client's
// config (dry run unless --apply); `xtraos doctor` reports what is missing. No OS automation
// here: only reading and writing the user's own config files, each backed up first.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { CoreClient, coreExe, resolveConnection } from './core-client.ts';

export const SERVER_NAME = 'xtraos';

/** Folders the client configs live under; injected so tests never touch real configs. */
export type Dirs = { home: string; appData: string; localAppData?: string; hasCommand?: (name: string) => boolean };

/** Is `name` a command on PATH (name, name.exe, name.cmd)? */
export function onPath(name: string, env: NodeJS.ProcessEnv = process.env): boolean {
  for (const dir of (env.PATH ?? env.Path ?? '').split(path.delimiter)) {
    if (!dir) continue;
    for (const ext of ['', '.exe', '.cmd']) if (fs.existsSync(path.join(dir, name + ext))) return true;
  }
  return false;
}

export const realDirs = (): Dirs => ({
  home: os.homedir(),
  appData: process.env.APPDATA ?? path.join(os.homedir(), 'AppData', 'Roaming'),
  localAppData: process.env.LOCALAPPDATA ?? path.join(os.homedir(), 'AppData', 'Local'),
  hasCommand: (name) => onPath(name),
});

/** What every client starts: the bundled Node directly (no shell, no .cmd). */
export type Entry = { command: string; args: string[]; env?: Record<string, string> };

export function serverEntry(tools?: string): Entry {
  // Installed bundle: <prefix>/mcp/xtraos-mcp.mjs with <prefix>/node/node.exe; source tree:
  // the running Node and this entry script.
  const bundledNode = path.resolve(import.meta.dirname, '..', 'node', 'node.exe');
  const command = fs.existsSync(bundledNode) ? bundledNode : process.execPath;
  const script = path.resolve(process.argv[1] ?? path.join(import.meta.dirname, 'index.ts'));
  const entry: Entry = { command, args: [script] };
  if (tools === 'core') entry.env = { XTRAOS_TOOLS: 'core' };
  return entry;
}

type Kind = 'mcpServers' | 'servers' | 'toml' | 'claude-cli' | 'hermes-cli';
export type Client = { id: string; name: string; file: string; kind: Kind; installed: boolean };
const isCli = (c: Client) => c.kind === 'claude-cli' || c.kind === 'hermes-cli';

/**
 * Supported clients (ADR 0045). A desktop app counts as installed when its config folder
 * exists. A command-line client counts only when its command is on PATH or its own config file
 * exists, because other tools create the same folders (Antigravity creates ~/.gemini).
 */
export function clients(d: Dirs): Client[] {
  const app = (id: string, name: string, file: string, kind: Kind): Client =>
    ({ id, name, file, kind, installed: fs.existsSync(path.dirname(file)) });
  const cli = (id: string, name: string, file: string, kind: Kind, bin: string): Client =>
    ({ id, name, file, kind, installed: (d.hasCommand?.(bin) ?? false) || fs.existsSync(file) });
  const code = path.join(d.appData, 'Code', 'User');
  // Hermes Agent keeps config.yaml under %LOCALAPPDATA%/hermes on native Windows, else ~/.hermes.
  const hermesNative = path.join(d.localAppData ?? path.join(d.home, 'AppData', 'Local'), 'hermes', 'config.yaml');
  const hermesFile = fs.existsSync(hermesNative) ? hermesNative : path.join(d.home, '.hermes', 'config.yaml');
  return [
    cli('claude-code', 'Claude Code', path.join(d.home, '.claude.json'), 'claude-cli', 'claude'),
    app('claude-desktop', 'Claude Desktop', path.join(d.appData, 'Claude', 'claude_desktop_config.json'), 'mcpServers'),
    app('cursor', 'Cursor', path.join(d.home, '.cursor', 'mcp.json'), 'mcpServers'),
    app('vscode', 'VS Code', path.join(code, 'mcp.json'), 'servers'),
    app('windsurf', 'Windsurf', path.join(d.home, '.codeium', 'windsurf', 'mcp_config.json'), 'mcpServers'),
    app('cline', 'Cline', path.join(code, 'globalStorage', 'saoudrizwan.claude-dev', 'settings', 'cline_mcp_settings.json'), 'mcpServers'),
    cli('codex', 'Codex CLI', path.join(d.home, '.codex', 'config.toml'), 'toml', 'codex'),
    cli('gemini', 'Gemini CLI', path.join(d.home, '.gemini', 'settings.json'), 'mcpServers', 'gemini'),
    cli('hermes', 'Hermes Agent', hermesFile, 'hermes-cli', 'hermes'),
  ];
}

export type State = 'not-installed' | 'configured' | 'missing' | 'different' | 'unparseable';
export type Check = { client: Client; state: State; current?: Entry; note?: string };

const readText = (file: string): string | null => {
  try { return fs.readFileSync(file, 'utf8').replace(/^﻿/, ''); } catch { return null; }
};

const sameEntry = (a: Entry | undefined, b: Entry): boolean =>
  !!a && a.command === b.command && JSON.stringify(a.args ?? []) === JSON.stringify(b.args) &&
  JSON.stringify(a.env ?? {}) === JSON.stringify(b.env ?? {});

// ---- TOML (Codex CLI): only the [mcp_servers.xtraos] tables are read or replaced. ----

const TOML_HEAD = /^\[mcp_servers\.xtraos(\.env)?\]\s*$/;

/** Splits config.toml into our lines and everything else (kept byte for byte). */
function splitToml(text: string): { ours: string[]; rest: string[] } {
  const ours: string[] = [];
  const rest: string[] = [];
  let mine = false;
  for (const line of text.split(/\r?\n/)) {
    if (/^\s*\[/.test(line)) mine = TOML_HEAD.test(line.trim());
    (mine ? ours : rest).push(line);
  }
  return { ours, rest };
}

const tomlBlock = (e: Entry): string => {
  const lines = [`[mcp_servers.${SERVER_NAME}]`, `command = ${JSON.stringify(e.command)}`, `args = [${e.args.map((a) => JSON.stringify(a)).join(', ')}]`];
  if (e.env) lines.push('', `[mcp_servers.${SERVER_NAME}.env]`, ...Object.entries(e.env).map(([k, v]) => `${k} = ${JSON.stringify(v)}`));
  return lines.join('\n');
};

function tomlEntry(ours: string[]): Entry | undefined {
  if (!ours.length) return undefined;
  const get = (key: string) => ours.find((l) => l.trim().startsWith(`${key} =`))?.split('=').slice(1).join('=').trim();
  try {
    const entry: Entry = { command: JSON.parse(get('command') ?? '""'), args: JSON.parse(get('args') ?? '[]') };
    const tools = get('XTRAOS_TOOLS');
    if (tools) entry.env = { XTRAOS_TOOLS: JSON.parse(tools) };
    return entry;
  } catch { return { command: '(unreadable)', args: [] }; }
}

// ---- Hermes Agent (config.yaml): read-only scan; writing goes through `hermes mcp`. ----

/** Is there an `xtraos:` key under `mcp_servers:`, and does it point at our script? */
function hermesState(text: string, want: Entry): State {
  const lines = text.split(/\r?\n/);
  const start = lines.findIndex((l) => /^mcp_servers:\s*$/.test(l));
  if (start < 0) return 'missing';
  const block: string[] = [];
  let mine = false;
  for (const l of lines.slice(start + 1)) {
    if (/^\S/.test(l)) break; // the next top-level key
    const key = /^ {2}([^\s:][^:]*):/.exec(l);
    if (key) mine = key[1] === SERVER_NAME;
    if (mine) block.push(l);
  }
  if (!block.length) return 'missing';
  // YAML may write the path with single or doubled backslashes, or forward slashes.
  const norm = (v: string) => v.replace(/\\+/g, '/').toLowerCase();
  const body = norm(block.join('\n'));
  const tools = body.includes('xtraos_tools');
  return body.includes(norm(want.args[0])) && tools === !!want.env ? 'configured' : 'different';
}

// ---- Check / apply / remove ----

/** Runs a client's own CLI (`claude`, `hermes`): no stdin and a time limit, so it cannot hang on a prompt. */
export type RunCli = (bin: string, args: string[]) => { ok: boolean; out: string };
const runCli: RunCli = (bin, args) => {
  // These are .cmd shims on Windows, so they need the shell.
  const r = spawnSync(bin, args, { encoding: 'utf8', shell: true, stdio: ['ignore', 'pipe', 'pipe'], timeout: 90_000 });
  return { ok: r.status === 0, out: `${r.stdout ?? ''}${r.stderr ?? ''}`.trim() };
};
const q = (v: string) => `"${v}"`;

export function check(client: Client, want: Entry): Check {
  if (!client.installed) return { client, state: 'not-installed' };
  const text = readText(client.file);
  if (text === null || text.trim() === '') return { client, state: 'missing' };
  if (client.kind === 'hermes-cli') return { client, state: hermesState(text, want) };
  let current: Entry | undefined;
  if (client.kind === 'toml') {
    current = tomlEntry(splitToml(text).ours);
  } else {
    let json: any;
    try { json = JSON.parse(text); } catch { return { client, state: 'unparseable', note: 'the file is not plain JSON (comments or a syntax error)' }; }
    current = json?.[client.kind === 'servers' ? 'servers' : 'mcpServers']?.[SERVER_NAME];
  }
  if (!current) return { client, state: 'missing' };
  return { client, state: sameEntry(current, want) ? 'configured' : 'different', current };
}

/** The text a user can paste or run by hand (also shown in the dry run). */
export function snippet(client: Client, e: Entry): string {
  if (client.kind === 'toml') return tomlBlock(e);
  if (client.kind === 'claude-cli') {
    const env = e.env ? Object.entries(e.env).map(([k, v]) => `-e ${k}=${v} `).join('') : '';
    return `claude mcp add --scope user ${env}${SERVER_NAME} -- ${q(e.command)} ${q(e.args[0])}`;
  }
  if (client.kind === 'hermes-cli') {
    const env = e.env ? `--env ${Object.entries(e.env).map(([k, v]) => `${k}=${v}`).join(' ')} ` : '';
    return `hermes mcp add ${SERVER_NAME} --command ${q(e.command)} ${env}--args ${q(e.args[0])}`;
  }
  const body = client.kind === 'servers' ? { type: 'stdio', ...e } : e;
  return JSON.stringify({ [client.kind === 'servers' ? 'servers' : 'mcpServers']: { [SERVER_NAME]: body } }, null, 2);
}

function backup(file: string): void {
  const copy = `${file}.xtraos-backup`;
  if (fs.existsSync(file) && !fs.existsSync(copy)) fs.copyFileSync(file, copy);
}

/** Writes (or with `entry = null` removes) the xtraos entry. Returns a one-line result. */
export function write(client: Client, entry: Entry | null, run: RunCli = runCli): string {
  if (client.kind === 'claude-cli') {
    run('claude', ['mcp', 'remove', '--scope', 'user', SERVER_NAME]); // absent is fine
    if (!entry) return 'removed (claude mcp remove)';
    const env = entry.env ? Object.entries(entry.env).flatMap(([k, v]) => ['-e', `${k}=${v}`]) : [];
    const r = run('claude', ['mcp', 'add', '--scope', 'user', ...env, SERVER_NAME, '--', q(entry.command), q(entry.args[0])]);
    return r.ok ? 'added (claude mcp add)' : `FAILED: ${r.out || 'the claude command is not on PATH'}`;
  }
  if (client.kind === 'hermes-cli') {
    // Hermes owns config.yaml; its CLI keeps the file's comments and other servers.
    backup(client.file);
    const had = hermesState(readText(client.file) ?? '', entry ?? { command: '', args: [''] }) !== 'missing';
    if (had) {
      const r = run('hermes', ['mcp', 'remove', SERVER_NAME]);
      if (!r.ok) return `FAILED: hermes mcp remove: ${r.out.slice(-300)}`;
    }
    if (!entry) return had ? 'removed (hermes mcp remove)' : 'no xtraos entry';
    const env = entry.env ? ['--env', ...Object.entries(entry.env).map(([k, v]) => `${k}=${v}`)] : [];
    const r = run('hermes', ['mcp', 'add', SERVER_NAME, '--command', q(entry.command), ...env, '--args', q(entry.args[0])]);
    return r.ok ? 'added (hermes mcp add)' : `FAILED: ${r.out.slice(-300) || 'the hermes command is not on PATH'}`;
  }
  const text = readText(client.file) ?? '';
  let next: string;
  if (client.kind === 'toml') {
    const { rest } = splitToml(text);
    const kept = rest.join('\n').replace(/\s+$/, '');
    next = entry ? `${kept ? kept + '\n\n' : ''}${tomlBlock(entry)}\n` : `${kept}\n`;
    if (text.includes('\r\n')) next = next.replace(/\n/g, '\r\n'); // keep the file's line endings
  } else {
    const key = client.kind === 'servers' ? 'servers' : 'mcpServers';
    let json: any = {};
    if (text.trim()) {
      try { json = JSON.parse(text); } catch { return 'SKIPPED: the file is not plain JSON; add the entry by hand'; }
    }
    if (typeof json !== 'object' || json === null || Array.isArray(json)) return 'SKIPPED: unexpected file content';
    json[key] = typeof json[key] === 'object' && json[key] !== null ? json[key] : {};
    if (entry) json[key][SERVER_NAME] = client.kind === 'servers' ? { type: 'stdio', ...entry } : entry;
    else delete json[key][SERVER_NAME];
    next = JSON.stringify(json, null, 2) + '\n';
  }
  backup(client.file);
  fs.mkdirSync(path.dirname(client.file), { recursive: true });
  fs.writeFileSync(client.file, next, 'utf8');
  return entry ? 'written' : 'removed';
}

// ---- Command line ----

type Options = { apply: boolean; remove: boolean; only: string[] | null; tools?: string };

function parse(argv: string[]): Options | string {
  const o: Options = { apply: false, remove: false, only: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--apply') o.apply = true;
    else if (a === '--remove') o.remove = true;
    else if (a === '--client') o.only = (argv[++i] ?? '').split(',').map((s) => s.trim()).filter(Boolean);
    else if (a === '--tools') o.tools = argv[++i];
    else return `unknown option '${a}'`;
  }
  if (o.tools !== undefined && o.tools !== 'core' && o.tools !== 'full') return `--tools must be core or full`;
  return o;
}

const HELP = `Usage:
  xtraos setup [--apply] [--remove] [--client a,b] [--tools core|full]
      Shows (and with --apply writes) the XtraOS entry for each installed AI client.
      --tools core lists 15 everyday tools instead of all (saves ~5,000 tokens per session).
  xtraos doctor
      Checks the install, the core, and each client's entry. Changes nothing.
Clients: ${clients(realDirs()).map((c) => c.id).join(', ')}`;

export function setup(argv: string[], d: Dirs = realDirs(), log: (s: string) => void = console.log, run: RunCli = runCli): number {
  const o = parse(argv);
  if (typeof o === 'string') { log(`${o}\n\n${HELP}`); return 2; }
  const want = serverEntry(o.tools);
  let list = clients(d);
  if (o.only) {
    const unknown = o.only.filter((id) => !list.some((c) => c.id === id));
    if (unknown.length) { log(`unknown client: ${unknown.join(', ')}\n\n${HELP}`); return 2; }
    list = list.filter((c) => o.only!.includes(c.id));
  }
  log(`XtraOS MCP server: ${q(want.command)} ${q(want.args[0])}${want.env ? ' (tools: core)' : ''}\n`);
  let pending = 0;
  let failed = 0;
  for (const c of list) {
    const k = check(c, want);
    if (k.state === 'not-installed') { log(`- ${c.name}: not installed`); continue; }
    if (o.remove) {
      if (k.state === 'missing') { log(`- ${c.name}: no xtraos entry`); continue; }
      if (!o.apply) { pending++; log(`- ${c.name}: would remove the xtraos entry${isCli(c) ? '' : ` from ${c.file}`}`); continue; }
      log(`- ${c.name}: ${write(c, null, run)}`);
      continue;
    }
    if (k.state === 'configured') { log(`- ${c.name}: already configured`); continue; }
    if (k.state === 'unparseable') {
      failed++;
      log(`- ${c.name}: ${k.note}; left untouched. Add this by hand to ${c.file}:\n${snippet(c, want)}`);
      continue;
    }
    const verb = k.state === 'different' ? 'update' : 'add';
    if (!o.apply) {
      pending++;
      log(`- ${c.name}: would ${verb} the entry${isCli(c) ? ' by running' : ` in ${c.file}`}:\n${snippet(c, want)}`);
      continue;
    }
    const result = write(c, want, run);
    if (/^(FAILED|SKIPPED)/.test(result)) failed++;
    log(`- ${c.name}: ${result}${c.kind === 'claude-cli' ? '' : ` (${c.file}; backup: .xtraos-backup)`}`);
  }
  if (!o.only && !o.remove) {
    log(`\nOpenClaw (not verified yet): add a stdio server with "openclaw mcp", command ${q(want.command)}, argument ${q(want.args[0])}.`);
  }
  if (pending) log(`\nNothing was changed. Run again with --apply to make ${pending} change(s). Restart the client afterwards.`);
  else if (o.apply) log('\nDone. Restart the client (or reload its MCP servers) to see XtraOS.');
  return failed ? 1 : 0;
}

export async function doctor(d: Dirs = realDirs(), log: (s: string) => void = console.log): Promise<number> {
  let bad = 0;
  const line = (ok: boolean | null, what: string, hint = '') => {
    if (ok === false) bad++;
    log(`${ok === null ? '[--]' : ok ? '[ok]' : '[!!]'} ${what}${hint && !ok ? `\n      ${hint}` : ''}`);
  };
  const want = serverEntry();
  line(fs.existsSync(want.command), `Node runtime: ${want.command}`);
  line(fs.existsSync(want.args[0]), `MCP server: ${want.args[0]}`);
  const exe = coreExe();
  line(!!exe, `Core binary: ${exe ?? 'not found'}`, 'Reinstall XtraOS (install.ps1).');

  // Core: report only; doctor never starts it.
  process.env.XTRAOS_NO_AUTOSTART = '1';
  let conn = false;
  try { resolveConnection(); conn = true; } catch { /* no connection files yet */ }
  if (!conn) {
    line(null, 'Core: not running (it starts by itself when an agent first uses XtraOS)');
  } else {
    const core = new CoreClient({ clientName: 'xtraos-doctor', timeoutMs: 3000 });
    try {
      const s = (await core.call('status')) as { version: string; state: string };
      line(s.state === 'running', `Core: ${s.state}, version ${s.version}`, 'The kill switch is on. Resume from the XtraOS tray icon.');
    } catch (e) {
      const code = (e as { code?: string }).code ?? '';
      if (code === 'NOT_PAIRED') line(null, 'Core: not running (it starts by itself when an agent first uses XtraOS)');
      else line(false, `Core: ${e instanceof Error ? e.message : String(e)}`);
    } finally { core.close(); }
  }

  let configured = 0;
  for (const c of clients(d)) {
    const k = check(c, want);
    if (k.state === 'not-installed') continue;
    const fix = `Run: xtraos setup --apply --client ${c.id}`;
    if (c.kind === 'hermes-cli' && (k.state === 'configured' || k.state === 'different')) {
      if (k.state === 'configured') configured++;
      line(k.state === 'configured', `${c.name}: xtraos entry`, `It does not point at this install. ${fix}`);
    } else if (k.state === 'configured' || k.state === 'different') {
      const cur = k.current!;
      const exists = fs.existsSync(cur.command) && (cur.args ?? []).every((a) => fs.existsSync(a));
      if (exists) configured++;
      line(exists, `${c.name}: xtraos entry${cur.env?.XTRAOS_TOOLS ? ` (tools: ${cur.env.XTRAOS_TOOLS})` : ''}${k.state === 'different' ? ' (differs from this install)' : ''}`,
        `It points at a file that does not exist. ${fix}`);
    } else if (k.state === 'unparseable') {
      line(null, `${c.name}: ${c.file} is not plain JSON; cannot check`);
    } else {
      line(null, `${c.name}: installed, no xtraos entry (${fix.replace('Run: ', 'run: ')})`);
    }
  }
  if (!configured) line(null, 'No client is configured yet. Run: xtraos setup');
  log(bad ? `\n${bad} problem(s) found.` : '\nNo problems found.');
  return bad ? 1 : 0;
}

/** Entry for `xtraos-mcp <setup|doctor|help>`; returns the process exit code. */
export async function main(argv: string[]): Promise<number> {
  const [cmd, ...rest] = argv;
  if (cmd === 'setup') return setup(rest);
  if (cmd === 'doctor') return doctor();
  console.log(HELP);
  return cmd === 'help' || cmd === '--help' || cmd === '-h' ? 0 : 2;
}
