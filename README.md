# XtraOS Agent

A local **AI action runtime**: the bridge that lets any AI model see and operate your computer
(apps, browsers, terminals, files, games) safely, fast, and token-efficiently.
The LLM is the brain; XtraOS Agent is the hands, eyes, and seatbelt.

> Status: **Phase 1 — M1, M2, M3 closed** (secure IPC, MCP, real input, windows & apps;
> ADRs 0009–0013). **Next: M4 Perception.** Gates: `tests/phase0/run-tests.ps1`,
> `tests/phase1/m1-gate.ps1`, `m2-gate.ps1`, `m3-gate.ps1` (`-Interactive` moves the mouse and
> types into the testbed; `-Uac` needs a human UAC click; `-Browsers` opens Chrome/Firefox).

## Documents

| Doc | Purpose |
|---|---|
| [CLAUDE.md](CLAUDE.md) | Development rules, boundaries, definition of done |
| [docs/01-product-spec.md](docs/01-product-spec.md) | What we build and what we never build |
| [docs/02-architecture.md](docs/02-architecture.md) | Components, IPC, pipelines, repo layout |
| [docs/03-functional-flows.md](docs/03-functional-flows.md) | Step-by-step flows (browser, admin, WhatsApp, games, kill switch) |
| [docs/04-mcp-tool-contract.md](docs/04-mcp-tool-contract.md) | The public MCP tool surface |
| [docs/05-security-model.md](docs/05-security-model.md) | Threats, policy engine, hard boundaries |
| [docs/06-performance-budget.md](docs/06-performance-budget.md) | Token and latency budgets |
| [docs/07-development-plan.md](docs/07-development-plan.md) | Phases, milestones, exit criteria |
| [docs/08-test-plan.md](docs/08-test-plan.md) | Test levels, test cases per phase, phase gates |
| [docs/adr/](docs/adr/) | Architecture decision records |

## Capabilities (verified only)

| Capability | Status |
|---|---|
| Secure local core ↔ MCP connection (user-only pipe, pairing token, reconnect) | verified — M1 gate (ADR 0009) |
| MCP tool `status` | verified — M1 gate |
| Action log (no secrets) | verified — M1 gate |
| Real mouse input: click (1–3, any button, modifiers), scroll, drag-and-drop, paths | verified — M2 gate (ADR 0011) |
| Real keyboard input: unicode typing (incl. Hindi, emoji), key chords | verified — M2 gate |
| Clipboard get/set | verified — M2 gate |
| Safety guards: user activity → abort, wrong window → refuse, no stuck keys/buttons | verified — M2 gate |
| Window control: list, focus, launch (incl. admin via UAC), graceful close, move/resize/min/max | verified — M3 gate (ADR 0013) |
| Elevation boundary: admin windows refused untouched (`ELEVATION_REQUIRED`) | verified — M3 gate |
| Browser flow without extensions (Chrome, Firefox: search → close) | verified — M3 gate |
| Reading the screen (UI tree, OCR, screenshots) | not yet (M4) |
