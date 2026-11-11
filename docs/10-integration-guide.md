# Integration guide — connecting AI tools to XtraOS Agent

XtraOS Agent works with any AI tool in one of two ways:

- **MCP** (stdio): the tool starts `xtraos-mcp`, which talks to the local core. 37 tools.
- **Command line**: any agent that can run a shell command calls
  `xtraos-core.exe call <tool> -`. Same tools, same rules. The skill
  [`skills/xtraos/SKILL.md`](../skills/xtraos/SKILL.md) teaches an agent how.

Both go through the same safety layer: approval dialogs, the kill switch (Ctrl+Alt+Esc),
per-agent permissions and pause (tray → Agents), and the action log.

## Compatibility

Only what was actually checked is marked verified. "Config tested" means `xtraos setup`
writes that client's config format correctly (automated tests SU-01…SU-14); "live" means a
real session of that client used XtraOS.

| Client | Connects through | `xtraos setup` config | Live run |
|---|---|---|---|
| Claude Code | MCP (`claude mcp add`) | config tested | **verified** — daily use while building XtraOS; `status` through MCP on 2026-11-11 |
| Any shell agent | command line (`xtraos-core.exe call`) | — | **verified** — ADR 0039, and every command in `SKILL.md` re-run on 2026-11-11 |
| Claude Desktop | MCP (`claude_desktop_config.json`) | config tested | pending |
| Cursor | MCP (`~/.cursor/mcp.json`) | config tested | pending |
| VS Code (Copilot agent) | MCP (`mcp.json`, `servers`) | config tested | pending |
| Windsurf | MCP | config tested | pending |
| Cline | MCP | config tested | pending |
| Codex CLI | MCP (`~/.codex/config.toml`) | config tested | pending |
| Gemini CLI | MCP (`~/.gemini/settings.json`) | config tested | pending |
| Hermes Agent | MCP (`hermes mcp`) or the skill | config tested | pending |
| OpenClaw | the skill (command line) | — | pending |
| Agents inside WSL2 | command line through Windows interop | — | pending (P5-M5) |

To move a row to verified: connect the client, ask it to call `status` and to do flow F2
(Calculator 7 + 5), and link the record here.

## Set up

1. Install XtraOS (README → Quick start).
2. Let XtraOS write the config for each AI tool it finds. Each config is backed up first:
   ```powershell
   $x = "$env:LOCALAPPDATA\Programs\XtraOS\xtraos.cmd"
   & $x setup            # dry run: what it would change
   & $x setup --apply    # write it; add --tools core for a 15-tool list (small context)
   & $x doctor           # check the install and each client
   ```
3. Restart the AI tool. Ask it: *"Call the XtraOS status tool."*

Each MCP client is named on the PC by its own MCP name, e.g. `mcp:claude-code`. You see that
name in the tray → Agents menu and in the "Rules and activity" window.

## Per client

**Claude Code.** `setup --apply` runs `claude mcp add` (user scope). Check with
`claude mcp list`. Small context: `--tools core`.

**Claude Desktop, Cursor, Windsurf, Cline.** An `mcpServers` entry named `xtraos` in the
client's JSON config, pointing at the bundled Node.js and `xtraos-mcp.mjs`. A config file with
comments is left untouched; `setup` prints the snippet to paste instead.

**VS Code.** The same entry under `servers` with `"type": "stdio"` in the user `mcp.json`.

**Codex CLI.** A `[mcp_servers.xtraos]` table in `~/.codex/config.toml`. The rest of the file
stays byte for byte.

**Gemini CLI.** An `mcpServers.xtraos` entry in `~/.gemini/settings.json`.

**Hermes Agent.** `setup --apply` runs `hermes mcp add` (an existing entry is removed first;
`config.yaml` is backed up). Hermes can also use the skill instead of MCP.

**OpenClaw and other skill-based agents.** Install the skill from Git
(`skills/xtraos/SKILL.md` in this repository). It uses the command line only.

**WSL2 agents.** Call the Windows exe through interop:
`/mnt/c/Users/<you>/AppData/Local/Programs/XtraOS/bin/xtraos-core.exe call status`. Not
verified yet.

## Limit what an agent may do

In `policy.json` (tray → Rules and activity → Open policy.json). XtraOS asks you to approve
every rule change.

```json
"clients": {
  "mcp:codex-mcp-client": "background_only",
  "mcp:hermes": {"profile": "full", "apps": ["chrome.exe"]},
  "*": "full"
},
"new_clients": "ask"
```

- `full`: like today.
- `background_only`: no real mouse or keyboard.
- `read_only`: it may look, not change.
- `blocked`: nothing but `status`.

With `new_clients: "ask"`, XtraOS asks you the first time an unknown agent wants to change
something. Details: ADR 0060.

## Several agents at once

Two agents never act in the same window at once: the second waits up to 10 s, then gets
`WINDOW_BUSY`. Reading is never blocked (ADR 0059). Pause one agent from the tray → Agents;
the others keep working (ADR 0061).
