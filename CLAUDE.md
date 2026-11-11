# XtraOS Agent — Project Rules (read before every task)

XtraOS Agent is a **local bridge** that lets an AI model (the "brain") perceive and operate a
computer (the "hands and eyes"). We build the bridge. We never put an LLM inside it.

These rules exist so that development stays on track, nothing is invented, and every claim is
backed by evidence. If a rule conflicts with a request, stop and ask the owner.

---

## 1. Sources of truth (in this order)

1. `docs/01-product-spec.md` — what we build and what we do NOT build.
2. `docs/02-architecture.md` — components, boundaries, IPC, tech stack.
3. `docs/04-mcp-tool-contract.md` — the exact public tool surface.
4. `docs/05-security-model.md` and `docs/06-performance-budget.md` — non-negotiable constraints.
5. `docs/07-development-plan.md` — the current phase and its exit criteria.
6. `docs/adr/` — every decision that changed any of the above.

Code that disagrees with these documents is a bug, unless an ADR changes the document first.

## 2. Anti-hallucination rules

- **No invented APIs.** Every OS API, crate, npm package, or protocol message used must exist
  and be verified (docs link, crate docs, or a compiling spike). Cite the doc URL in a comment
  next to non-obvious OS calls (`SendInput`, UI Automation patterns, capture APIs).
- **No invented capabilities.** Never write "supports X" in docs, README, tool descriptions,
  or commit messages unless a test or recorded manual check proves it. Use "planned" otherwise.
- **No silent stubs.** A function that is not implemented returns an explicit
  `NOT_IMPLEMENTED` error. Never return fake success.
- **Verify before reporting.** "Done" means: builds, tests pass, and the behaviour was observed
  (test output, log, or screenshot). Report failures honestly with the output.
- **Unknown = ask or spike.** If unsure how an OS behaves (DPI, focus rules, UIPI, secure
  desktop), write a small spike in `spikes/` and record the result in an ADR before building on it.
- **Version pinning.** Pin exact dependency versions. Record why each new dependency is needed.

## 3. Scope boundaries

- Work only on the **current phase** in `docs/07-development-plan.md`. Items from later phases
  are recorded in `docs/backlog.md`, not implemented.
- New feature, new tool, or changed tool signature → update the spec/contract and add an ADR
  **before** writing code.
- Platform order is fixed: **Windows first**, then macOS, then Linux. No cross-platform
  abstractions beyond the `Platform` trait until the second platform starts.
- One responsibility per component (see architecture). Do not let the MCP server touch OS APIs
  directly; only the core does.

## 4. Hard safety boundaries (never implement, never accept as a task)

- No CAPTCHA solving, no bot-detection evasion, no "humanize to avoid detection" features.
- No automation of the Windows secure desktop / UAC consent, lock screen, or credential prompts.
  Elevation always requires a human click.
- No typing of passwords, payment data, or government IDs by the agent. Credential fields are
  detected and handed to the human.
- No remote/network control: the core listens on local IPC only, paired by token. The single
  exception (owner decision, ADR 0072): the MCP server may serve MCP over HTTP when started
  with `--http`, bound to 127.0.0.1 only, with its own bearer token, Host checked and browser
  Origins refused. Never another address, never on by default.
- No hidden operation: while automation runs, the overlay is visible and the kill switch works.
- No disabling of the kill switch, policy engine, or action log, even for tests in release builds.

## 5. Engineering rules

- **Languages:** core in Rust (stable, pinned toolchain). MCP server in TypeScript (Node LTS).
  Tray/overlay/approval UI: native Win32 inside the core in Phase 1 (ADR 0017; a separate
  Tauri app may follow later). No other languages without an ADR.
- **Every public tool** has: schema, contract entry, unit test, and one end-to-end test
  against the test harness app (`testbed/`).
- **Errors** use the shared error codes in the tool contract (`ELEMENT_NOT_FOUND`,
  `WINDOW_NOT_FOUND`, `NEEDS_HUMAN`, `BLOCKED_BY_POLICY`, `STOPPED`, `TIMEOUT`,
  `ELEVATION_REQUIRED`, `NOT_IMPLEMENTED`, `INTERNAL`). Never throw raw strings across IPC.
- **Coordinates:** all public coordinates are physical screen pixels with an explicit
  `monitor` id. The core is per-monitor DPI aware. Screenshot→screen mapping is done in the
  core, never by the agent.
- **Performance budgets** in `docs/06-performance-budget.md` are tests, not wishes. A PR that
  regresses a budget must say so and justify it.
- **Logging:** every action is written to the action log (what, where, result, duration).
  Never log typed secrets; mask fields detected as password inputs.
- **Commits:** small, one concern each, conventional prefix (`feat:`, `fix:`, `docs:`,
  `test:`, `spike:`, `chore:`). Do not commit or push unless the owner asks.

## 6. Definition of Done (per task)

1. Matches the spec/contract (or the spec/contract was updated with an ADR).
2. Builds on Windows with no warnings in the touched crates/packages.
3. Unit tests + relevant e2e test pass; output pasted in the PR/summary.
4. Budgets checked where relevant (tokens per observation, latency).
5. Safety: action log entry, policy check, and kill switch path covered.
6. Docs updated (contract, README capability table) — only with verified claims.

## 7. When in doubt

Stop, state what is unknown, and propose a spike or a question. Guessing is a defect.
