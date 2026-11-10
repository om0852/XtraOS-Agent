# ADR 0045 — P5-M3: `xtraos setup` and `xtraos doctor`

- Status: accepted, built (automated tests; live check of each client pending).
- Date: 2026-11-10

## Context
Today the installer can register XtraOS only with Claude Code. Every other MCP client needs a
hand-written config entry, and users get the Windows details wrong (a `.cmd` launcher that the
client starts without a shell, unescaped backslashes). Phase 5 (ADR 0044) asks for a command
that does this for the common clients, and one that tells the user what is wrong.

## Decision
Two subcommands of the MCP launcher (TypeScript, `mcp/src/setup.ts`; no OS automation, so it
does not belong in the core). The installer adds `xtraos.cmd`, so users type `xtraos setup`.

### `xtraos setup [--apply] [--remove] [--client a,b] [--tools core|full]`
- **Dry run by default.** It lists each supported client, whether it is installed, and the
  exact entry it would write. Nothing changes without `--apply`.
- **The entry** starts the bundled Node directly (no shell, no `.cmd`):
  `command: <prefix>\node\node.exe`, `args: [<prefix>\mcp\xtraos-mcp.mjs]`, and
  `env: { XTRAOS_TOOLS: "core" }` when `--tools core` is given.
- **Clients and files** (formats checked against the vendors' docs or widely used guides on
  2026-11-10; each stays "unverified live" in the compatibility matrix until a recorded run):

  | Client | File | Shape |
  |---|---|---|
  | Claude Code | via `claude mcp add --scope user` (its own CLI owns `~/.claude.json`) | — |
  | Claude Desktop | `%APPDATA%\Claude\claude_desktop_config.json` | `mcpServers` |
  | Cursor | `~\.cursor\mcp.json` | `mcpServers` |
  | VS Code | `%APPDATA%\Code\User\mcp.json` | `servers`, `type: "stdio"` |
  | Windsurf | `~\.codeium\windsurf\mcp_config.json` | `mcpServers` |
  | Cline | `%APPDATA%\Code\User\globalStorage\saoudrizwan.claude-dev\settings\cline_mcp_settings.json` | `mcpServers` |
  | Codex CLI | `~\.codex\config.toml` | `[mcp_servers.xtraos]` |
  | Gemini CLI | `~\.gemini\settings.json` | `mcpServers` |
  | Hermes Agent | via `hermes mcp add xtraos --command … --args …` (its CLI owns `config.yaml`; native Windows: `%LOCALAPPDATA%\hermes\config.yaml`, else `~/.hermes/config.yaml`). Seen on the owner's PC: Hermes Agent v0.22.2 native, `hermes mcp add / remove / list / test`. | — |

  OpenClaw is printed as a manual hint until it is verified (P5-M5).
- **What counts as installed** (corrected after the first live run, 2026-11-10): a desktop
  app when its config folder exists; a command-line client (Claude Code, Codex, Gemini,
  Hermes) only when its command is on PATH or its own config file exists. Reason: on the
  owner's PC `~/.gemini` exists because Antigravity creates it, but Gemini CLI is not installed,
  and the folder rule wrongly offered to configure it (test SU-11).
- **Client CLIs** run without stdin and with a 90 s limit, so a prompt can never hang setup.
- **Safe writes.** Before the
  first change, the file is copied to `<file>.xtraos-backup` (never overwritten). Only the
  `xtraos` entry is touched. A JSON file that does not parse (comments, a syntax error) is left
  alone and the snippet is printed instead. Writing is idempotent.
- `--remove` deletes only the `xtraos` entry.

### `xtraos doctor`
Read-only. Reports: bundle files present; whether the core is running and paired (it does not
start the core); and for each installed client whether the `xtraos` entry exists and points at
files that exist. Exit code 0 when everything needed is in place, 1 otherwise.

## Consequences
- No change to the tool contract; no new capability for agents.
- `install.ps1` prints `xtraos setup` as the next step; `-ConfigureClaudeCode` stays.
- Tests use temporary home folders; no real client config is touched by the test suite.

## Live runs
| Client | Date | Result |
|---|---|---|
| Codex CLI 0.163.0 | 2026-11-10 | `xtraos setup --apply --client codex` appended only the `[mcp_servers.xtraos]` table to `~/.codex/config.toml` (rest of the file byte-identical, backup equal to the original, second run "already configured"). `codex mcp list` / `codex mcp get xtraos` show it enabled over stdio. `codex exec` discovered the tools and started `xtraos/status`, then Codex itself refused the call: "MCP tool call requires approval, but approval policy is never" (non-interactive mode). A tool call approved in interactive `codex` is **still open**. |
| Gemini CLI | 2026-11-10 | Not installed on the owner's PC (only Antigravity's `~/.gemini`); found the detection bug above. |

Other clients: not run live yet (P5-M9).

Findings from the live run, fixed: CRLF line endings of `config.toml` are kept (SU-13); and a
release-process note: piping `install.ps1` into `Select-Object -First N` stops the installer
before it copies files, so its output must not be truncated that way.
