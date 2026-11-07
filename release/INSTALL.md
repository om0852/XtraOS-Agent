# XtraOS Agent 0.1 — developer preview

XtraOS Agent lets an AI agent (Claude Code, Claude Desktop or any MCP client) see and operate
your Windows desktop: read windows, click, type, drag, launch and arrange apps — preferring
background actions that do not touch your mouse or keyboard, and stopping the moment you
press **Ctrl+Alt+Esc**.

## Requirements
- 64-bit Windows 10 2004 or Windows 11, normal (non-admin) user.
- **Smart App Control off.** This preview is not code-signed yet, so Windows blocks it while
  Smart App Control is on. Signed builds come later. Nothing here changes that setting.

## Install
1. Extract the zip anywhere.
2. In PowerShell, in the extracted folder:
   `powershell -ExecutionPolicy Bypass -File install.ps1`
   - `-ConfigureClaudeCode` also registers it with Claude Code (user scope).
   - `-StartAtLogin` starts the core when you sign in (otherwise it starts on first use).
3. Add the MCP server to your client with the command/JSON the installer prints.

## Safety you can rely on
- **Ctrl+Alt+Esc** (or the XtraOS tray icon → Stop) halts the agent instantly; only you can
  resume, from the tray icon.
- Sending messages, deleting, system settings and admin launches ask you first; payments and
  password entry are always refused; password managers are off limits.
- A banner at the top of the screen shows what the agent is doing.
- Your rules: `%LOCALAPPDATA%\XtraOS\policy.json`. Action log: `%LOCALAPPDATA%\XtraOS\logs`.

## Uninstall
`powershell -ExecutionPolicy Bypass -File "%LOCALAPPDATA%\Programs\XtraOS\uninstall.ps1"`
(`-Purge` also removes your XtraOS data: pairing token, logs, policy).
