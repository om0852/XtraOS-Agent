XtraOS Agent lets AI agents (Claude Code, Codex, Cursor, Claude Desktop, any MCP client or shell
agent) see and operate a Windows PC safely: UI Automation instead of screenshots, approval
dialogs for risky actions, a kill switch (Ctrl+Alt+Esc) and a tamper-evident action log.

## Install
- **npx** (Node.js 20+): `npx xtraos-install`
- **Zip**: download `xtraos-<version>-win-x64.zip`, check it against `SHA256SUMS.txt`, unzip, run
  `powershell -ExecutionPolicy Bypass -File install.ps1`.
- **winget**: manifests are attached (`winget-manifests-<version>.zip`).

Then connect your AI tools: `%LOCALAPPDATA%\Programs\XtraOS\xtraos.cmd setup --apply`.

## Signing
If this release is not code-signed, Windows Smart App Control blocks it; turn Smart App Control
off or wait for a signed release. Signed releases show the publisher in the file properties.

Full documentation: README.md and docs/10-integration-guide.md in the repository.
