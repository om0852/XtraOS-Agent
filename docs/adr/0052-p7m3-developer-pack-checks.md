# ADR 0052 — P7-M3 Developer pack, slice 1: UI checks (`dev.check`) and what terminals expose

- Status: accepted (owner: "Developer pack", 2026-11-10).
- Date: 2026-11-10

## Context
The developer pack (docs/09, P7-M3) is: drive the user's own app for UI checks, read VS Code and
terminal output, emulator/installer flows, "reproduce this bug" recording. Much exists already:
`wait_for` (text, title, element gone), `read_text`, `frame_diff`, `record_session` + `run_skill`.
What is missing is a check that reports **each** expectation as PASS/FAIL with evidence: an
agent testing an app wants "these 4 things hold, this one does not, here is a screenshot",
not one TIMEOUT.

Spike, 2026-11-10 (own test windows, closed afterwards):

| Where | `read_text` | Note |
|---|---|---|
| Classic console (conhost cmd) | marker found, prompt line too | UIA TextPattern |
| Windows Terminal | marker found | UIA TextPattern |
| VS Code editor | marker found (+ object-replacement characters) | |
| VS Code integrated terminal | **not exposed** (panel `Group "Terminal"`: 17 chars, no output) | xterm.js draws it; Ctrl+` opened it (borrowed 100–150 ms), but typed input did not reliably reach the shell; OCR did not find the output either |

VS Code switched itself to "Screen Reader Optimized" while XtraOS read it (its own detection of
UI Automation clients); nothing was changed in its settings by XtraOS.

## Decision
1. **Built-in skill `dev.check`** (class observe: read-only, no prompt), through `run_skill` so the
   tool list does not grow:
   `{window, expect: [...], timeout_ms?: 3000 (≤ 30000), screenshot_on_fail?: true}` where each
   expectation is one of
   - `{"text": "Saved"}` — the window's visible or document/terminal text contains it
     (case-insensitive),
   - `{"no_text": "Error"}` — it does not,
   - `{"element": "Save", "role"?: "Button"}` — an element whose name contains it (role exact
     when given) is on screen,
   - `{"title": "report.pdf"}` — the window title contains it.
   It re-reads every 250 ms until all pass or the timeout ends, then returns
   `{pass, checks: [{check, pass, detail}], screenshot?}` and a text block of `PASS …` / `FAIL …`
   lines. A failure saves a PNG of the window to `%TEMP%\xtraos-checks\` (path in the result;
   outside XtraOS's protected data folder so the developer can attach it).
2. **Terminals:** documented as above. Agents are told (skill description and contract) to run
   commands they need to read in Windows Terminal or a console; the VS Code integrated terminal
   stays "not readable" until a reliable way is verified (planned, not claimed).
3. **Later slices** of P7-M3: a "reproduce this bug" report (recorded steps + a screenshot per
   step as one Markdown file), installer/emulator flows.

## Addendum (2026-11-11): value, enabled state, visible change
- `{"element": "Total", "value": "42"}` — the element's value contains it (password values are
  never read); `{"element": "Delete", "enabled": false}` — its state. A failure shows the actual
  value and state (`Edit "text" is = "changed-later", enabled`).
- `{"changed": true, "min_pct"?: 0.1}` — the window visibly changed while the check ran
  (frame_diff baseline at the start, latched on the first difference). The first try used
  frame_diff's default 0.5 %: a text-field update on the testbed is ≈ 0.4 % of the window, so
  the check passed only 4.7 s later on something else. The default is 0.1 % (a blinking caret is
  ≈ 0.07 %). It resets the window's frame_diff baseline.
- A window that cannot be captured (minimized — seen twice while the owner used the PC) fails
  only the `changed` check, with the reason; the baseline is retried while the check runs.

## Tests
- Unit: expectation parsing and the pure verdict over a (text, elements, title) view.
- Interactive: `dev.check` on the testbed — passing text/element/title checks, a failing one with a
  screenshot file that exists, `no_text`, and the timeout waiting for a late text (`latertext`).
