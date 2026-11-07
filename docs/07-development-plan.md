# 07 — Development Plan

**Current phase: Phase 1 — Foundation (Windows).** Phase 0 closed by ADR 0007 (gate 10 PASS /
0 FAIL / 1 SKIP). Toolchain installed and pinned (ADR 0008).
**M1 Core + IPC: closed** (ADR 0009) — gate 6/6 PASS twice from a clean build
(`tests/phase1/m1-gate.ps1`). Smart App Control turned off by the owner on this dev machine.
**M2 Input: closed** (ADRs 0010, 0011) — gate 8/8 PASS incl. 11/11 interactive, hands-off run;
`set_value` and Explorer→app drag
(P1-IN-07) moved to M4.
**M3 Windows & apps: closed** (ADRs 0012, 0013) — gate 9/9 PASS, 0 SKIP incl. UAC + browser
flow F3. **M4 Perception + background-first actions: closed** (ADRs 0014–0016) — gate 10/10
PASS, 0 SKIP; large-list observe latency is a stated miss (Phase 2). **M5 Safety: closed**
(ADRs 0017, 0018) — kill switch 15 ms, policy + approvals, overlay; safety e2e 6/6.
**Next: M6 Release 0.1.** Only work on the current phase (see CLAUDE.md §3).
Each milestone has exit criteria; a milestone is done only when all are met and evidenced.

## Phase 0 — Spikes & skeleton (≈ 1 week)

Goal: prove every risky OS assumption before building on it. Each spike → one ADR.

| ID | Spike | Exit criteria |
|---|---|---|
| S1 | `SendInput` absolute click on each monitor with mixed DPI (100%/150%) | Clicks land within 2 px on testbed buttons on all monitors. |
| S2 | UI Automation tree walk of Notepad, Explorer, Chrome, Firefox, Windows Terminal | Element list with names/roles; time and element counts recorded. |
| S3 | Windows.Graphics.Capture window screenshot + mapping to screen coords | Saved PNG; mapping verified by clicking a detected point. |
| S4 | Real drag-and-drop Explorer → browser upload zone | File received by testbed page. |
| S5 | Global hotkey + releasing held input | Hotkey stops a running drag within 100 ms, no stuck keys. |
| S6 | Named pipe with user-only ACL + token from Node | Node connects; another user/process without token is refused. |
| S7 | UIPI check: input to elevated window from normal process | Documented failure + elevated helper path confirmed. |
| S8 | Tauri click-through top-most overlay | Overlay visible, does not intercept clicks. |

Progress (2026-11-07): S1, S2, S3, S6, S9 (real app: Calculator) done (ADRs 0003–0006;
single monitor only — multi-monitor needs a second display to verify).
S4, S5, S7, S8 not started. Spikes use built-in tooling (ADR 0002); Rust install deferred to Phase 1.

Deliverables: repo skeleton (`core/`, `mcp/`, `tray/`, `testbed/`, `bench/`), CI building on
Windows, ADRs 0002–0009 with results.

## Phase 1 — Foundation, Windows (≈ 4–5 weeks)

| Milestone | Scope | Exit criteria |
|---|---|---|
Gate: every Phase 1 test case in `docs/08-test-plan.md` PASSes (incl. S4/S5/S7/S8 carried over).

| M1 Core + IPC | Pipe server, pairing, sessions, error codes, action log | MCP `status` works end to end; unpaired client refused (test). |
| M2 Input | click/double/right, type (unicode + keys), press_keys, scroll, drag, mouse path, clipboard | All input tools pass e2e on testbed; latency budgets met. |
| M3 Windows & apps | list/focus/launch/close/bounds, elevated launch (UAC by human) | F3 flow passes (Chrome → close → Firefox search). |
| M4 Perception | observe (refs, caps, diffs), find, read_text, screenshot | Token budgets met on 5 bench apps; opaque detection works. |
| M5 Safety | policy engine (defaults), HITL `ask_human`/`wait_for_human`, kill switch, overlay | Kill switch ≤ 100 ms; confirm dialog blocks a `message_send`-class action. |
| M6 Release 0.1 | MCP packaging, install script, README capability table (verified only), **code-signed binaries** (Smart App Control, ADR 0009) | Fresh-machine install incl. a machine with Smart App Control on; flows F2, F3, F4, F6, F10 recorded. |

## Phase 2 — Smart & cheap (≈ 4–6 weeks)

Element-ref hardening, trusted `batch`, local OCR, app map (DPAPI), skills engine + first
skills (WhatsApp desktop/web send file, Telegram send, Explorer file ops), elevated helper,
browser driver bridge to XtraDevPilot, MSI packaging.
Exit: F5 and F7 pass; routine task ≤ 3 calls / ≤ 2,000 tokens in bench.

## Phase 3 — Reach

Game/canvas helpers (frame diff, detector hooks), touch & pen injection (`InjectSyntheticPointerInput`),
multi-finger gestures, recording/replay, macOS platform (CGEvent, AX, ScreenCaptureKit).

## Phase 4 — Platform

Linux (AT-SPI, libei/portals), SDKs (Node/Python), team policies + audit export, skill registry.

## Working agreement

- Each task: issue → branch → small commits → PR with evidence (test output / recording).
- Spec/contract changes land before code (with ADR).
- Weekly: run `bench/`, update the budget table, review backlog.
