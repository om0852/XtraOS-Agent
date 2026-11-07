# ADR 0019 — M6 scope: Release 0.1 (unsigned developer preview)

- Status: accepted (scope). Findings and gate result → ADR 0020.
- Date: 2026-11-07

## Context
M6 (docs/07): MCP packaging, install script, README capability table (verified only),
code-signed binaries; exit = fresh-machine install incl. Smart App Control (SAC) on, flows
F2, F3, F4, F6, F10 recorded. Constraint: no paid services. The repo is public with no
licence, so free open-source signing (SignPath Foundation) is not available yet; paid signing
is excluded.

## Decision (owner, 2026-11-07)
1. **Release 0.1 is an unsigned developer preview.** It runs where SAC is off (SmartScreen may
   warn once). The release script signs when given a certificate (`-CertThumbprint`, uses the
   Windows SDK `signtool`), so signing is one parameter later. **The "SAC on" exit criterion
   stays open** and is listed as such — not claimed.
2. **Fresh-machine test = clean-profile test on this PC** (owner's choice): install into an
   empty folder with its own data directory and pipe name, then `status` through MCP within
   2 minutes. Not a truly fresh machine; recorded as such.
3. **Bundle** `dist/xtraos-0.1.0-win-x64.zip`, per-user, no admin, no installer framework:
   - `bin/xtraos-core.exe` (release build);
   - `mcp/` (server source + production `node_modules` only) and `node/node.exe` — the
     official Node 24 runtime (signed by the OpenJS Foundation, version and SHA-256 recorded),
     so users need no Node install; `xtraos-mcp.cmd` starts it;
   - `install.ps1`, `uninstall.ps1`, `INSTALL.md`, `THIRD-PARTY.md` (Node, MCP SDK, Rust crates).
4. **The MCP server starts the core on demand.** If the pipe is not there, it launches
   `bin/xtraos-core.exe` (detached, in the user's session) and waits ≤ 10 s. So no autostart
   entry is required; the core (with its tray icon and kill switch) runs while it is used.
5. **install.ps1**: copies to `%LOCALAPPDATA%\Programs\XtraOS` (or `-Prefix`), checks Windows
   10 2004+ x64, prints ready-to-paste MCP configs (Claude Code, Claude Desktop, generic).
   Opt-in only: `-StartAtLogin` (HKCU Run entry), `-ConfigureClaudeCode` (runs
   `claude mcp add --scope user`). It never changes security settings (SAC, Defender, UAC).
   **uninstall.ps1** stops the installed core, removes the Run entry and the install folder;
   user data (`%LOCALAPPDATA%\XtraOS`: token, logs, policy) only with `-Purge`.
6. **Recorded flows**: `tests/flows/run-flows.mjs` drives the *installed* bundle through MCP
   exactly as an agent would and writes transcripts (each call, result, token estimate,
   latency) to `docs/flows/`: F2 (observe → act, Calculator), F3 (Chrome search → close →
   Firefox search), F4 (Explorer file → drop zone page in Chrome via refs), F6 (Windows
   Terminal: `claude --version`, read the output — no paid model call), F10 (kill switch during
   a drag, then STOPPED, then tray resume).
7. **Bench report** `docs/bench/release-0.1.md` on the release build via MCP (pipe round trip,
   observe, click(ref), type 50 chars, screenshot, kill switch) against docs/06; deviations
   stated (P1-REL-04).
8. Version 0.1.0 everywhere; a git tag / GitHub release only when the owner asks.

## Consequences
- docs/02 repo layout: `release/` (scripts), `dist/` (ignored), `docs/flows/`, `docs/bench/`.
- README: install section + capability table re-reviewed (P1-REL-03).
- Open after M6: signing (licence + SignPath, or the owner's certificate) and a truly fresh
  machine / SAC-on test.
