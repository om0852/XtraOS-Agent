# ADR 0036 — P2-M8 policy editor + hard rule; Phase 3 order and P3-M1/M2/M3 scope

- Status: accepted (scope). Findings → ADR 0037.
- Date: 2026-11-07

## Context
The owner asked to implement every remaining item now and to test what needs a human later.

What is left:
- In Phase 2: a policy editor (the viewer is read-only) and faster observe on huge lists.
- Phase 3 (docs/07): canvas/game helpers, touch & pen, multi-finger gestures,
  recording/replay, Agent Workspace / sandbox, macOS.

## Decision
### P2-M8
1. **Policy editor in the viewer** (`--viewer`).
   - Controls: a list box for each class (allow / ask first / refuse), app rules as text
     lines `process.exe = allow|confirm|deny`, and the rate limit.
   - Save writes `policy.json`. Keys it does not know are kept.
   - The core applies the file **only after the user approves the listed changes in the
     core's own dialog** (ADR 0026). Hard rules still win: credential entry is always
     refused; purchase is at most "ask first".
2. **Hard rule: XtraOS's own windows are off limits to the agent.**
   - Today only the core's own process windows are protected. The viewer/editor, the admin
     helper and other cores are separate `xtraos-core.exe` processes.
   - Every `xtraos-core.exe` window is now `deny`, and the user cannot loosen this. So the
     agent cannot drive the editor to loosen its own rules.
3. **Not done: an event-driven observe cache.**
   - Caching "no changes" until a UIA event arrives makes a missed event a silent lie.
     Providers do not raise events reliably.
   - Correctness wins; `scope_ref` and `find` remain the fast paths (ADR 0035).

### Phase 3 order
| Milestone | Content | Needs a human to test |
|---|---|---|
| P3-M1 | `frame_diff`: changed regions of a window between calls / until a change (canvas, games, video); background capture | no |
| P3-M2 | `record_session`: record the agent's successful actions as a replayable macro skill with stable locators instead of refs | no (testbed) |
| P3-M3 | `touch` / `pen` via InjectSyntheticPointerInput: tap, long-press, drag, pinch, multi-finger swipe; pen with pressure | yes (real input) |
| P3-M4 | Agent Workspace / Windows Sandbox spike | owner: needs Windows features enabled (a system setting) |
| P3-M5 | macOS | needs a Mac |

### P3-M1 `frame_diff {window, region?, wait_ms?, min_pct?}`
- Captures the window with the existing background capture (works when covered, not when
  minimised).
- Compares a 16 px grid with the previous frame of the same window. The first call sets
  the baseline.
- Returns the changed cells merged into rectangles (screen coordinates, at most 16) and
  `changed_pct`.
- `wait_ms` polls every 50 ms until the change is at least `min_pct` (default 0.5 %) or the
  time is up.
- No pixels are returned (use `screenshot` or `ocr` for that).

### P3-M2 `record_session {op: start|stop|status|discard, name?}`
- While recording, every successful action is stored: click, type, press_keys, scroll,
  drag, mouse, set_value, focus_window, launch_app, close_window, window_bounds, run_skill,
  browser input actions.
- Refs become **locators**: `{"locate": {"process", "title", "role", "name"}}`.
  Windows become `{"locate_window": {"process", "title"}}`. A point becomes `{"at":
  {window locator, dx, dy}}`, relative to that window.
- `stop` saves a macro skill `<name>.json` in the skills folder. Replay is `run_skill name`:
  - the existing macro engine runs it;
  - every step passes the policy again;
  - locators are resolved at run time (window by process + title, element by role + exact
    name). If a locator does not resolve, the step fails with ELEMENT_NOT_FOUND; it never
    guesses.
- **Confidentiality:**
  - Text typed into password fields is never recorded (those actions are refused anyway).
  - Typed text becomes a skill argument with the recorded text as its default, so it can be
    changed at replay.
  - Recording is visible on the overlay banner ("recording").
  - At most 500 steps; recording stops by itself after 30 minutes.

### P3-M3 `touch` / `pen`
- `CreateSyntheticPointerDevice` + `InjectSyntheticPointerInput` (Windows 10 1809+).
  - touch: up to 5 contacts;
  - actions: tap, long_press, drag, pinch (scale), swipe (fingers 1–4);
  - pen: tap and draw a path with pressure 0–1024.
- Same rules as mouse input: polite (waits for the user to be idle), the kill switch stops
  it between frames, the target must not be an XtraOS window, policy classes as for click.
- Multi-finger OS gestures (3–4 finger swipes switch desktops / open Task View) are class
  `system_settings`, which means "ask first".
- Tests are written and run in the hands-off session.

## Tests
- **P2-ED-01..03:** the editor writes `policy.json` keeping unknown keys; the core then
  asks; the agent's input to an `xtraos-core.exe` window is refused.
- **P3-FD-01..03:** frame diff baseline, a change found, `wait_ms`.
- **P3-RR-01..04:** record and replay on the testbed:
  - locators resolve after the window moves;
  - a missing element fails cleanly;
  - password text is never stored;
  - stop at the limits.
- **P3-TP-*:** hands off.
