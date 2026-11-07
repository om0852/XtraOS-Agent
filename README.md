# XtraOS Agent

A local **AI action runtime**: the bridge that lets any AI model see and operate your computer
(apps, browsers, terminals, files, games) safely, fast, and token-efficiently.
The LLM is the brain; XtraOS Agent is the hands, eyes, and seatbelt.

> Status: **Phase 1 — M1 closed** (core + secure IPC + MCP `status`, ADR 0009). **Next: M2 Input.**
> Gates: `tests/phase0/run-tests.ps1` (`-Interactive` moves the mouse / opens Calculator) and
> `tests/phase1/m1-gate.ps1`.

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
| Clicking, typing, reading the screen | not yet (M2–M4) |
