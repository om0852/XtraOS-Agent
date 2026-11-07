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

## Optional extras
- **Web pages (the `browser` tool).** This uses the XtraDevPilot Chrome extension and its
  bridge.
  - You need an XtraDevPilot build that has the pairing token (`%USERPROFILE%\.xtradevpilot\bridge-token`).
    Older builds without a token are not used.
  - The XtraDevPilot MCP server must be running (it hosts the bridge on 127.0.0.1:42819),
    with the extension loaded in Chrome.
  - Without these, `browser` answers that you need to start them.
- **Admin windows (the `elevated_helper` tool).** You approve a Windows administrator prompt
  for each session. Nothing elevated is installed. The helper asks you before it touches
  each admin window.
- **Packaging.** This zip and `install.ps1` are the package; there is no MSI. Code signing
  is still pending (see Requirements).

## Safety you can rely on
- **Ctrl+Alt+Esc** (or the XtraOS tray icon → Stop) halts the agent instantly; only you can
  resume, from the tray icon.
- Sending messages, deleting, system settings and admin launches ask you first; payments and
  password entry are always refused; password managers are off limits.
- A banner at the top of the screen shows what the agent is doing.
- Tray icon → **Rules and activity…** shows your rules and what the agent did recently (read only).
- Your rules: `%LOCALAPPDATA%\XtraOS\policy.json`. Action log: `%LOCALAPPDATA%\XtraOS\logs`.

## Uninstall
`powershell -ExecutionPolicy Bypass -File "%LOCALAPPDATA%\Programs\XtraOS\uninstall.ps1"`
(`-Purge` also removes your XtraOS data: pairing token, logs, policy).
