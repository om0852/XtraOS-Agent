# ADR 0006 — Operating a real third-party app: findings from spike S9

- Status: accepted (single monitor); multi-monitor still unverified on hardware
- Date: 2026-11-07

## Context
S1 proved input on our own window. The product goal is operating **any** user app. S9
launches a fresh Windows Calculator (UWP, separate process), finds controls via UI
Automation from outside, clicks them and types with real input, verifies, and closes it.
Safety: only the Calculator window the spike launched; click point must be inside it; keys
only while it is foreground; abort on user mouse movement/held buttons; restore cursor and
previous foreground window.

## Evidence (owner-approved runs, 2026-11-07, 1 monitor 1920×1080 @ 144 DPI)

Run 1 — abort: `clearButton` not found. Cause: UWP frame window appears before its content
tree; a fixed sleep is not enough. Fix: poll for the control (`WaitFor`, up to 5 s).

Run 2 — abort before click: guard found the target point was over **Firefox**, not
Calculator. Read-only diagnosis:
- UIA and Win32 rects agree exactly (physical px) for a PMv2 thread → no DPI mismatch.
- Calculator had opened **maximized but behind Firefox**: Windows' foreground lock
  (SetForegroundWindow restrictions) kept it in the background. The guard prevented a click
  into the user's Firefox. This is the guard working as designed.

Run 3 — pass, after restore + resize + focus via `AttachThreadInput` + `SetForegroundWindow`
(no synthetic Alt key, which would toggle the user's app menu bar):

| Step | Result |
|---|---|
| Launch → window found | 1,177 ms |
| Brought to front | true |
| 5 UIA-located clicks (C, 7, +, 8, =) | each cursor error (0,0); ~170–184 ms incl. waits |
| Display after clicks | "Display is 15" → PASS |
| Real keyboard: Esc 1 2 × 3 Enter | "Display is 36" → PASS |
| Cleanup | 0 Calculator windows / processes left |

## Decision
1. Element-targeted input on foreign apps works pixel-exactly with UIA rects + PMv2 + SendInput.
2. `focus_window` is a core primitive: restore if minimized/maximized as needed, bring to
   front via AttachThreadInput + SetForegroundWindow, **verify foreground**; if it fails,
   return `NEEDS_HUMAN` ("click the window once") rather than forcing it.
3. Every element action re-checks "target point belongs to the target window's root"
   immediately before input (it caught a real misfire in run 2). Keyboard input requires the
   target to be foreground at each step.
4. Element lookup polls (event-driven in Rust via UIA events) instead of fixed sleeps.

## Still open
- Multi-monitor and mixed-DPI on real hardware (only one display attached). The spike already
  loops over every monitor and moves the app there; re-run when a second screen is connected.
- Elevated windows (S7), drag-and-drop between apps (S4).
