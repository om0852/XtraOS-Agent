# XtraOS Agent

A local **AI action runtime**: the bridge that lets any AI model see and operate your computer
(apps, browsers, terminals, files, games) safely, fast, and token-efficiently.
The LLM is the brain; XtraOS Agent is the hands, eyes, and seatbelt.

> Status: **Phase 1 — M1–M5 closed** (secure IPC, MCP, real input, windows & apps, perception,
> background-first actions, safety; ADRs 0009–0018). **Next: M6 Release 0.1.** Gates:
> `tests/phase0/run-tests.ps1`, `tests/phase1/m1-gate.ps1` … `m5-gate.ps1` (`-Interactive` moves
> the mouse and types into the testbed; `-Uac` needs a human UAC click; `-Browsers` opens Chrome/Firefox).

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
| Reading windows: `observe` (element refs, diffs, opaque flag), `find`, `read_text`; passwords always masked | verified — M4 gate (ADR 0016) |
| Screenshots of a window or region with exact screen mapping | verified — M4 gate |
| Background actions without the user's mouse/keyboard: `click(ref)`, `set_value`, `scroll(ref)`; focus kept with the user | verified — M4 gate (Calculator, WinForms) |
| Polite real input: waits for the user to be idle, puts the cursor back; drag by element ref (Explorer → app) | verified — M4 gate |
| Kill switch: Ctrl+Alt+Esc or tray Stop halts input (15 ms measured), only the user resumes | verified — M5 gate (ADR 0018) |
| Policy engine: Send/Reply, delete, settings, admin launch need your OK; buying and password entry always blocked; password managers off limits; editable `policy.json` | verified — M5 gate |
| Overlay banner while the agent acts (click-through, hidden from screenshots); agent can never use XtraOS's own windows | verified — M5 gate |
| Ask the user: `ask_human` (yes/no, choice, info, task) + `wait_for_human` | verified — M5 gate |
| Fast observe on windows with hundreds of list items | not yet (Phase 2; 222 ms measured vs 120 ms budget) |
| OCR | not yet (Phase 2) |
