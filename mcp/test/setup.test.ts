// P5-M3 (ADR 0045): `xtraos setup` / `xtraos doctor`. Every test uses temporary home folders,
// so no real client config is touched.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { check, clients, doctor, serverEntry, setup, write, type Dirs } from '../src/setup.ts';

function tempDirs(): Dirs {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'xtraos-setup-'));
  return { home: path.join(root, 'home'), appData: path.join(root, 'appdata') };
}
const mk = (file: string, text?: string) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  if (text !== undefined) fs.writeFileSync(file, text, 'utf8');
};
const byId = (d: Dirs, id: string) => clients(d).find((c) => c.id === id)!;
const run = (argv: string[], d: Dirs, claude: (bin: string, args: string[]) => { ok: boolean; out: string } = () => ({ ok: true, out: '' })) => {
  const out: string[] = [];
  const code = setup(argv, d, (s) => out.push(s), claude);
  return { code, text: out.join('\n') };
};

test('SU-01: a client whose config folder is absent is "not installed" and nothing is created', () => {
  const d = tempDirs();
  const r = run(['--apply'], d);
  assert.equal(r.code, 0);
  assert.match(r.text, /Cursor: not installed/);
  assert.ok(!fs.existsSync(d.home), 'no folder may be created for a missing client');
});

test('SU-02: dry run shows the entry and changes nothing', () => {
  const d = tempDirs();
  const cursor = byId(d, 'cursor');
  mk(cursor.file, '{"mcpServers":{"other":{"command":"x"}}}');
  const before = fs.readFileSync(cursor.file, 'utf8');
  const r = run([], d);
  assert.match(r.text, /Cursor: would add the entry/);
  assert.match(r.text, /Run again with --apply/);
  assert.equal(fs.readFileSync(cursor.file, 'utf8'), before);
  assert.ok(!fs.existsSync(`${cursor.file}.xtraos-backup`));
});

test('SU-03: --apply adds the entry, keeps other servers, backs up once, and is idempotent', () => {
  const d = tempDirs();
  const cursor = byId(d, 'cursor');
  const original = '{"mcpServers":{"other":{"command":"x"}},"keep":1}';
  mk(cursor.file, original);
  assert.equal(run(['--apply', '--client', 'cursor'], d).code, 0);
  const json = JSON.parse(fs.readFileSync(cursor.file, 'utf8'));
  assert.deepEqual(json.mcpServers.other, { command: 'x' });
  assert.equal(json.keep, 1);
  assert.deepEqual(json.mcpServers.xtraos, serverEntry());
  assert.equal(fs.readFileSync(`${cursor.file}.xtraos-backup`, 'utf8'), original);
  const again = run(['--apply', '--client', 'cursor'], d);
  assert.match(again.text, /Cursor: already configured/);
  assert.equal(fs.readFileSync(`${cursor.file}.xtraos-backup`, 'utf8'), original, 'the backup is never overwritten');
});

test('SU-04: VS Code uses "servers" with type stdio; --tools core adds the env', () => {
  const d = tempDirs();
  const vscode = byId(d, 'vscode');
  mk(vscode.file); // folder exists, file absent
  assert.equal(run(['--apply', '--client', 'vscode', '--tools', 'core'], d).code, 0);
  const entry = JSON.parse(fs.readFileSync(vscode.file, 'utf8')).servers.xtraos;
  assert.equal(entry.type, 'stdio');
  assert.deepEqual(entry.env, { XTRAOS_TOOLS: 'core' });
  assert.equal(entry.command, serverEntry().command);
});

test('SU-05: a JSON file with comments is left untouched and the snippet is printed', () => {
  const d = tempDirs();
  const gemini = byId(d, 'gemini');
  const text = '{\n  // my settings\n  "theme": "dark"\n}';
  mk(gemini.file, text);
  const r = run(['--apply', '--client', 'gemini'], d);
  assert.equal(r.code, 1);
  assert.match(r.text, /left untouched/);
  assert.match(r.text, /"mcpServers"/);
  assert.equal(fs.readFileSync(gemini.file, 'utf8'), text);
});

test('SU-06: Codex TOML: the block is added, other tables are kept, an update replaces it', () => {
  const d = tempDirs();
  const codex = byId(d, 'codex');
  mk(codex.file, 'model = "x"\n\n[mcp_servers.other]\ncommand = "o"\n');
  assert.equal(run(['--apply', '--client', 'codex'], d).code, 0);
  let text = fs.readFileSync(codex.file, 'utf8');
  assert.match(text, /^model = "x"/);
  assert.match(text, /\[mcp_servers\.other\]\ncommand = "o"/);
  assert.equal(text.match(/\[mcp_servers\.xtraos\]/g)!.length, 1);
  assert.equal(check(byId(d, 'codex'), serverEntry()).state, 'configured');
  // Switch to the core profile: still exactly one block, now with the env table.
  assert.equal(run(['--apply', '--client', 'codex', '--tools', 'core'], d).code, 0);
  text = fs.readFileSync(codex.file, 'utf8');
  assert.equal(text.match(/\[mcp_servers\.xtraos\]/g)!.length, 1);
  assert.match(text, /\[mcp_servers\.xtraos\.env\]\nXTRAOS_TOOLS = "core"/);
  assert.match(text, /\[mcp_servers\.other\]/);
});

test('SU-13: Codex TOML: the rest of the file stays byte for byte, CRLF line endings are kept', () => {
  const d = tempDirs();
  const codex = byId(d, 'codex');
  const original = 'model = "x"\r\n\r\n[mcp_servers.other]\r\ncommand = \'C:\\a b\\o.exe\'\r\n';
  mk(codex.file, original);
  assert.equal(run(['--apply', '--client', 'codex'], d).code, 0);
  const text = fs.readFileSync(codex.file, 'utf8');
  assert.ok(text.startsWith(original.trimEnd()), 'the existing content is unchanged');
  assert.ok(!/[^\r]\n/.test(text), 'every line ending is CRLF');
  assert.equal(run(['--remove', '--apply', '--client', 'codex'], d).code, 0);
  assert.equal(fs.readFileSync(codex.file, 'utf8'), original);
});

test('SU-07: --remove deletes only the xtraos entry', () => {
  const d = tempDirs();
  const cursor = byId(d, 'cursor');
  mk(cursor.file, '{"mcpServers":{"other":{"command":"x"}}}');
  write(cursor, serverEntry());
  assert.equal(run(['--remove', '--apply', '--client', 'cursor'], d).code, 0);
  assert.deepEqual(JSON.parse(fs.readFileSync(cursor.file, 'utf8')).mcpServers, { other: { command: 'x' } });
});

test('SU-08: Claude Code goes through its own CLI (remove, then add with the env)', () => {
  const d: Dirs = { ...tempDirs(), hasCommand: (n) => n === 'claude' };
  const calls: string[][] = [];
  const r = run(['--apply', '--client', 'claude-code', '--tools', 'core'], d, (bin, args) => { calls.push([bin, ...args]); return { ok: true, out: '' }; });
  assert.equal(r.code, 0);
  assert.deepEqual(calls[0], ['claude', 'mcp', 'remove', '--scope', 'user', 'xtraos']);
  assert.deepEqual(calls[1].slice(0, 8), ['claude', 'mcp', 'add', '--scope', 'user', '-e', 'XTRAOS_TOOLS=core', 'xtraos']);
  assert.ok(!fs.existsSync(path.join(d.home, '.claude.json')), '~/.claude.json is never written by setup');
});

// Found live on the owner's PC: Antigravity creates ~/.gemini, but Gemini CLI is not installed.
test('SU-11: a command-line client needs its command or its own config file, not just a folder', () => {
  const d = tempDirs();
  mk(path.join(d.home, '.gemini', 'antigravity', 'x'), '');
  assert.match(run(['--client', 'gemini'], d).text, /Gemini CLI: not installed/);
  assert.match(run(['--client', 'gemini'], { ...d, hasCommand: (n) => n === 'gemini' }).text, /Gemini CLI: would add the entry/);
});

test('SU-12: Hermes Agent goes through `hermes mcp`; an existing entry is removed first; config.yaml is backed up', () => {
  const base = tempDirs();
  const d: Dirs = { ...base, localAppData: path.join(base.home, 'local'), hasCommand: (n) => n === 'hermes' };
  const file = path.join(d.localAppData!, 'hermes', 'config.yaml');
  mk(file, 'model: x\nmcp_servers:\n  other:\n    command: o\ntoolsets: []\n');
  // Dry run: shows the command, runs nothing.
  const calls: string[][] = [];
  const fake = (bin: string, args: string[]) => { calls.push([bin, ...args]); return { ok: true, out: '' }; };
  const dry = run(['--client', 'hermes'], d, fake);
  assert.match(dry.text, /Hermes Agent: would add the entry by running:\nhermes mcp add xtraos --command /);
  assert.equal(calls.length, 0);
  // Apply: no entry yet, so only `add`; --args is the last option.
  assert.equal(run(['--apply', '--client', 'hermes', '--tools', 'core'], d, fake).code, 0);
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].slice(0, 5), ['hermes', 'mcp', 'add', 'xtraos', '--command']);
  assert.deepEqual(calls[0].slice(6, 8), ['--env', 'XTRAOS_TOOLS=core']);
  assert.equal(calls[0].at(-2), '--args');
  assert.ok(fs.existsSync(`${file}.xtraos-backup`));
  // With an entry for this install already there: "already configured", nothing runs.
  const want = serverEntry();
  fs.writeFileSync(file, `mcp_servers:\n  other:\n    command: o\n  xtraos:\n    command: ${JSON.stringify(want.command)}\n    args:\n    - ${JSON.stringify(want.args[0])}\nmodel: x\n`);
  calls.length = 0;
  assert.match(run(['--apply', '--client', 'hermes'], d, fake).text, /Hermes Agent: already configured/);
  assert.equal(calls.length, 0);
  // A different entry (the core profile is wanted now): remove, then add.
  assert.equal(run(['--apply', '--client', 'hermes', '--tools', 'core'], d, fake).code, 0);
  assert.deepEqual(calls.map((c) => c.slice(0, 3).join(' ')), ['hermes mcp remove', 'hermes mcp add']);
});

test('SU-09: bad options are refused with exit code 2', () => {
  const d = tempDirs();
  assert.equal(run(['--client', 'nope'], d).code, 2);
  assert.equal(run(['--tools', 'tiny'], d).code, 2);
  assert.equal(run(['--frobnicate'], d).code, 2);
});

test('SU-10: doctor flags an entry that points at a missing file and changes nothing', async () => {
  const d = tempDirs();
  const cursor = byId(d, 'cursor');
  const text = JSON.stringify({ mcpServers: { xtraos: { command: 'C:\\nowhere\\node.exe', args: ['C:\\nowhere\\x.mjs'] } } });
  mk(cursor.file, text);
  const out: string[] = [];
  const saved = { home: process.env.XTRAOS_HOME };
  process.env.XTRAOS_HOME = path.join(d.home, 'no-core-here'); // no connection files: core "not running"
  try {
    const code = await doctor(d, (s) => out.push(s));
    assert.equal(code, 1);
    assert.match(out.join('\n'), /\[!!\] Cursor: xtraos entry/);
    assert.match(out.join('\n'), /Core: not running/);
    assert.equal(fs.readFileSync(cursor.file, 'utf8'), text);
  } finally {
    if (saved.home === undefined) delete process.env.XTRAOS_HOME; else process.env.XTRAOS_HOME = saved.home;
    delete process.env.XTRAOS_NO_AUTOSTART;
  }
});
