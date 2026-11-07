# ADR 0005 — Real input via SendInput: findings from spike S1

- Status: accepted (single monitor); open for multi-monitor / mixed DPI
- Date: 2026-11-07

## Context
Phase 1 input relies on `SendInput` with absolute coordinates over the virtual desktop
(`MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK`, normalised to 0..65535) from a
per-monitor-DPI-aware-v2 thread. S1 verifies accuracy and delivery on our own
non-activating test window, guarded as described in `spikes/README.md`.

## Evidence (run approved by owner, 2026-11-07; 1920×1080 @ 144 DPI / 150%)

| Button | Target (physical px) | Cursor error | Click delivered | Wrong-button clicks | Time incl. deliberate waits |
|---|---|---|---|---|---|
| B0 | (146,145) | (0,0) | yes | 0 | 212 ms |
| B1 | (386,155) | (0,0) | yes | 0 | 153 ms |
| B2 | (276,295) | (0,0) | yes | 0 | 126 ms |

- All guards passed; the user's cursor was restored; the test window never took focus and
  clicks were still delivered to it (WS_EX_NOACTIVATE + real input works).
- Times include ~105 ms of deliberate message pumping (30 ms after move, 15 ms between
  down/up, 60 ms after click), so raw injection cost is well under the 80 ms p50 budget.
- Spike note: the "stray" column in the raw output counted earlier buttons' legitimate
  clicks (1, 2); there were no wrong-button clicks.

## Decision
1. Core input uses `SendInput` absolute + VIRTUALDESK normalisation from a PMv2 thread;
   pixel-exact on this machine.
2. Keep the three safety guards (target-under-point check, no held buttons, user-moved-mouse
   detection) as **core features**, not just spike code: they become the basis of the
   "don't fight the user" rule in the action pipeline.
3. Tune waits in Rust and measure p50/p95 against the 80 ms budget in `bench/`.

## Still open
Multi-monitor and mixed-DPI accuracy (only one monitor available), elevated target windows
(S7), drag-and-drop (S4).
