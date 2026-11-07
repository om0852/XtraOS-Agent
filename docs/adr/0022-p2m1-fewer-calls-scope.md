# ADR 0022 — P2-M1 scope: `wait_for`, scoped observe, trusted `batch`

- Status: accepted (scope). Findings and gate → ADR 0023.
- Date: 2026-11-07

## Context
Cost lever #1–#7 (docs/06): agents waste calls polling ("is it loaded yet?") and doing one
action per round trip. `wait_for` was in the P1 contract but not built (ADR 0021).

## Decision
1. **`wait_for(window?, window_title?, text_contains?, ref_gone?, window_gone?, idle_ms?,
   timeout_ms?)`** — all given conditions must hold at the same time; at least one is required.
   - `window_title`: substring (case-insensitive) of the window's title; without `window`, any
     top-level window → its ref is returned (e.g. a page that finished loading).
   - `text_contains`: visible names/values of the window's tree, else the text of its main
     text element (terminals, editors) — e.g. a prompt appearing.
   - `ref_gone`: the element is gone (STALE) or no longer on screen. `window_gone`: closed.
   - `idle_ms`: the visible tree has not changed for that long (page/app settled).
   - Polls in the core (250 ms; title/gone checks are cheap Win32, tree checks one cached UIA
     fetch) — no agent round trips. Default timeout 10 s, max 120 s → `TIMEOUT` naming the
     unmet conditions. Read-only: policy class `observe`; stops on the kill switch.
   Event-driven UIA subscriptions are a later optimisation, not needed for the budget.
2. **`observe(scope_ref)`**: observe only one element's subtree (a list, a dialog, a pane):
   one cached fetch from that element; always the full list for the scope (no diff); refs join
   the window's ref table. Cuts tokens and time on large windows (ADR 0016).
3. **`batch(steps[], stop_on_error? = true)`**: up to 50 steps `{tool, args}` executed in the
   core without AI round trips. Allowed tools: click, type, press_keys, scroll, drag, mouse,
   set_value, clipboard, focus_window, window_bounds, launch_app, close_window, wait_for,
   observe, find, read_text. Not allowed (→ `INVALID_ARGS` before anything runs): batch,
   ask_human, wait_for_human, screenshot, status, unknown tools.
   **Every step passes the policy engine on its own** (a confirm step still shows the dialog;
   a denied step is `BLOCKED_BY_POLICY`), and the kill switch stops the batch between and
   within steps. Each step is written to the action log as `batch:<tool>`. Result: one compact
   line per step; on the first error the rest is skipped (unless `stop_on_error: false`).
   "Trusted" = the sequence runs without the model in the loop, never without the policy.

## Tests (docs/08, P2-M1)
P2-WF-01..05, P2-OBS-01, P2-BA-01..04, P2-BENCH-01 (a routine task in ≤ 3 calls /
≤ 2,000 tokens through MCP on the release build).

## Consequences
docs/04: `wait_for` row precise (P2-M1), `observe` gains `scope_ref`, `batch` row precise.
