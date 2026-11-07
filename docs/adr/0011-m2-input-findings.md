# ADR 0011 — M2 Input: implementation findings and gate status

- Status: accepted. **M2 closed** 2026-11-07 (owner-confirmed hands-off gate run, see Resolution).
- Date: 2026-11-07

## What was built
- `core/xtraos-core/src/input.rs`: click (1–3, left/right/middle, modifiers), mouse
  (move/down/up/path), scroll (v/h wheel), drag (real OLE-compatible), type (unicode incl.
  Hindi/emoji, Enter/Tab as real keys), press_keys (chords), clipboard get/set. Guards per
  ADR 0010. All Win32 FFI declared by hand with doc links; INPUT layout asserted (40 bytes).
- Protocol: input methods dispatched as `Execute`, run on a blocking thread, serialised
  process-wide; results/errors logged without params (typed text never logged).
- MCP: tools `click, type, press_keys, scroll, drag, mouse, clipboard` (contract test enforces).
- `testbed/`: WinForms app (built-in csc), PMv2, reports control rects/state over stdio.
- Tests: 6 new unit tests; 11 interactive e2e tests (`tests/input_e2e.rs`, `#[ignore]`);
  gate `tests/phase1/m2-gate.ps1 [-Interactive]`.

## Bugs found by the tests and fixed
1. **Argument validation ran after live-input checks** → an off-screen point could return
   USER_ACTIVE instead of INVALID_ARGS depending on what the user was doing. Now every
   argument (incl. on-desktop bounds) is validated before any guard reads input state.
2. **Guard adopted user movement as its own** (re-read cursor after moving): a user move
   during that window was never detected (in10). Now the guard remembers the aimed point.
3. **Cursor lag**: the OS applies a move asynchronously; under load the cursor can lag
   > 8 ms. A too-early check gave false USER_ACTIVE, and a click sent before arrival could
   land at the old position (a lost click in in01). Now the guard waits (≤ 60 ms) for the
   cursor to arrive before any button event.
4. Testbed reported non-ASCII as "?" (console code page); fixed with a UTF-8 stdout writer
   (first attempt via Console.OutputEncoding crashed the windowless testbed — reverted).
5. in10 simulated the user as one instantaneous jump, which a following automated move can
   overwrite unseen; replaced with a continuous movement stream (how a real hand behaves).

## Gate evidence so far
- Non-interactive steps (fmt, clippy -D warnings, 18 unit + 9 IPC, build, tsc, 7 MCP,
  testbed build): PASS on every run.
- Interactive suite after fixes 1–5: **4 consecutive runs 11/11 PASS**.
- Other runs on this busy desktop stopped on safety refusals: `USER_ACTIVE` ("mouse moved",
  "a mouse button is held") and `TARGET_OBSCURED` (testbed covered). These are the guards
  working (nothing was clicked elsewhere). One earlier run had a click reported OK that did
  not register on btnB — consistent with concurrent user input, **not proven**; tracked.

## Decision
M2 closes after one interactive gate run with the owner confirming hands-off, passing 11/11.
If a reported-OK click is ever lost again during a confirmed hands-off run, it becomes a bug.

## Resolution (2026-11-07)

Owner confirmed hands-off; `tests/phase1/m2-gate.ps1 -Interactive`:

| Step | Result |
|---|---|
| rustfmt --check | PASS |
| clippy, warnings = errors | PASS |
| cargo test: 18 unit + 9 IPC | PASS |
| cargo build | PASS |
| tsc --noEmit | PASS |
| MCP: 3 contract + 4 e2e | PASS (7/7) |
| testbed build | PASS |
| Interactive input e2e (P1-IN-01..06, -09..-12, P1-LOG-02, clipboard) | **PASS 11/11** |
| **Gate** | **8/8 PASS** |

No reported-OK click was lost during the confirmed hands-off run; the earlier occurrence
stays attributed to concurrent user input. M2 is closed.
