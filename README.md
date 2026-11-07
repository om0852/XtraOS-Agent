# XtraOS Agent

A local **AI action runtime**: the bridge that lets any AI model see and operate your computer
(apps, browsers, terminals, files, games) safely, fast, and token-efficiently.
The LLM is the brain; XtraOS Agent is the hands, eyes, and seatbelt.

> Status: **Phase 1 complete — Release 0.1 (unsigned developer preview)**, M1–M6 closed
> (ADRs 0009–0020); **Phase 2: P2-M1 closed** (wait_for, batch, scoped observe; ADR 0023),
> **P2-M2 closed** (OCR; ADR 0025), **P2-M3 closed** (app map, skills; ADR 0027), **P2-M4 built** (WhatsApp skill, resource discipline; ADR 0029), **P2-M5 built** (elevated admin helper; ADR 0031), **P2-M6 built** (browser bridge to XtraDevPilot; ADR 0033), **P2-M7 built** (Phase 2 leftovers; ADR 0035); **Phase 3: P2-M8 + P3-M1..M3 built** (rules editor, frame diff, session recording, touch/pen; ADR 0037). The hands-off and UAC test runs are pending. Open: code signing and a fresh-machine test with Smart App Control on.
> Gates: `tests/phase0/run-tests.ps1`, `tests/phase1/m1-gate.ps1` … `m6-gate.ps1`, `tests/phase2/p2m1-gate.ps1` … `p2m7-gate.ps1`, `tests/phase3/p3-gate.ps1`
> (`-Interactive` moves the mouse and types; keep hands off).

## Install (Release 0.1)

1. Build the bundle: `powershell -ExecutionPolicy Bypass -File release/build-release.ps1`
   → `dist/xtraos-0.1.0-win-x64.zip` (or use a provided zip; check `SHA256SUMS.txt`).
2. Extract, then `powershell -ExecutionPolicy Bypass -File install.ps1` (per user, no admin;
   `-ConfigureClaudeCode`, `-StartAtLogin` optional). Needs Windows 10 2004+ x64 and, for this
   unsigned preview, Smart App Control off.
3. Add the printed MCP config to your client, e.g.
   `claude mcp add --scope user xtraos -- "%LOCALAPPDATA%\Programs\XtraOS\xtraos-mcp.cmd"`.
   The core starts on first use (tray icon); **Ctrl+Alt+Esc** stops the agent at any time.

Recorded flows: [docs/flows](docs/flows/README.md) · Bench: [docs/bench/release-0.1.md](docs/bench/release-0.1.md)

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
| One-zip per-user install, MCP starts the core on demand, clean uninstall (install → working in 6–13 s) | verified — M6 gate (ADR 0020) |
| Flows F2, F3, F4, F6, F10 driven through MCP on the release build | verified — M6 gate ([docs/flows](docs/flows/README.md)) |
| `wait_for` (title, text, element/window gone, idle) in the core; `batch` of up to 50 steps, each step still policy-checked; `observe(scope_ref)` | verified — P2-M1 gate (ADR 0023) |
| Routine task in ≤ 3 calls / ≤ 2,000 tokens (Calculator 7 + 5: 3 calls, ~335 tokens) | verified — P2-M1 gate ([R1](docs/flows/R1.md)) |
| `ocr`: text + click-ready screen coordinates from opaque windows (games, canvas), Windows' built-in OCR, offline (~54 tokens vs ~2,225 for a screenshot) | verified — P2-M2 gate (ADR 0025, [F8](docs/flows/F8.md)) |
| Locked screen / UAC prompt detected: actions return `NEEDS_HUMAN` instead of acting blind | verified — P2-M2 gate run |
| App map: installed apps with launch ids; remembered services (DPAPI-encrypted); `launch_app("service")` | verified — P2-M3 gate (ADR 0027) |
| Skills: file skills (copy/move/rename/mkdir/list/open; delete → Recycle Bin after your OK; system + XtraOS folders protected) and JSON macro skills (policy per step) — routine file task in one call | verified — P2-M3 gate ([S1](docs/flows/S1.md)) |
| Policy-file changes apply only after you approve them; activity detection ignores the agent's own input and mouse jitter | verified — P2-M3 gate |
| `whatsapp.send` skill: text or file to a number, after your OK. It checks the chat is that number, restores your clipboard and closes what it opened | verified: P2-M4 e2e, real sends to the owner's own chat (ADR 0029) |
| Web pages via the XtraDevPilot bridge (`browser`): selectors/text, same policy (messaging send asks, buy and password fields refused); no scripts or cookies | automated tests pass (fake bridge, ADR 0033); live check needs the token-enabled XtraDevPilot |
| Long and virtualised lists: off-screen rows found, `find deep` reaches Explorer rows never drawn, a row click selects (never opens) | verified: P2-M7 automated tests (ADR 0035) |
| Send/Buy/Delete safety words in Hindi, Marathi, Spanish, French, German, Portuguese; read-only "Rules and activity" viewer (tray) | verified: unit + viewer test (ADR 0035) |
| `sandbox`: the agent works in a disposable Windows Sandbox desktop with its own cursor, in parallel with you (network off by default) | channel verified; live run needs Windows Sandbox enabled (ADR 0040) |
| Command-line bridge `xtraos-core call <tool> [json]` for any LLM/agent with a shell; `volume` (exact system volume), media keys, `system` (battery, brightness, Wi-Fi/Bluetooth, lock, sleep; switching asks first) | verified live on this PC (ADR 0039) |
| Tamper-evident action log (hash chain; `xtraos-core --verify-log`) and a max-actions-per-task guard that asks you to continue | verified: LG-01, TL-01 (ADR 0038) |
| Rules editor in the tray viewer (the core still asks before applying); every XtraOS window is off limits to the agent | verified: P2-ED tests (ADR 0037) |
| `frame_diff`: what changed in a canvas/game window, or wait for a change | verified: P3-FD tests |
| `record_session`: the agent's actions saved as a replayable skill with locators (works after windows move; typed text editable; passwords never recorded) | verified: P3-RR tests |
| `touch` / `pen`: tap, long press, drag, pinch, multi-finger swipe, pen with pressure | built; Windows-gesture policy verified; real input tests pending (hands off) |
| Admin windows (UIPI): opt-in `elevated_helper`, started through UAC for one session. It accepts only the core and asks you in its own elevated dialog for each admin window | automated tests pass (helper unelevated, ADR 0031); real UAC run pending |
| Resource discipline: the agent marks the windows it opened, reuses them instead of opening duplicates, and `cleanup` closes only those; closing your windows asks first | verified: P2-M4 e2e (ADR 0029) |
| Code-signed binaries (Smart App Control on) | not yet (owner decision, ADR 0019) |
| Fast observe on windows with thousands of list items | budget missed and provider-bound (167–328 ms vs 120; ADR 0035); use `scope_ref` or `find` there |
| OCR | verified — P2-M2 (see above) |
