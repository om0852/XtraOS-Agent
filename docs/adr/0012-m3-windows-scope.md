# ADR 0012 — M3 Windows & apps: window refs, elevation boundary, test scope

- Status: accepted
- Date: 2026-11-07

## Decisions
1. **Window refs are stateless**: `w<handle>` (e.g. `w460650`). Every `window` argument accepts
   the ref string or the raw integer handle (M2 compatibility). No title-substring targeting
   (ambiguous); `list_windows` has a `filter` (case-insensitive substring of title or process).
2. **Listing filter** (Alt+Tab-like): visible, not DWM-cloaked, non-empty title, not a tool
   window, no owner. Bounds are the DWM extended frame (no invisible borders).
3. **launch_app** uses `ShellExecuteExW` (`open`, or `runas` when `elevated`). Names without a
   path/extension get `.exe` (App Paths + PATH resolution); names with `!` are AUMIDs
   (`shell:AppsFolder\<AUMID>`). Result = the first *new* top-level window (prefer the
   launched PID; apps like Chrome hand off to an existing process). No new window within
   the timeout → `TIMEOUT`. UAC cancelled by the user → `NEEDS_HUMAN` ("declined").
   Error dialogs are suppressed (`SEE_MASK_FLAG_NO_UI`).
4. **focus_window**: restore if minimised, `AttachThreadInput` + `BringWindowToTop` +
   `SetForegroundWindow` (S9), verify; else `NEEDS_HUMAN`.
5. **close_window**: graceful `WM_CLOSE` only (apps may ask to save). Never kills processes.
   Still open after the timeout → `NEEDS_HUMAN` ("may be asking to save").
6. **window_bounds**: get, or set x/y/w/h for the *visible frame* (compensates invisible
   borders), or `state` minimize/maximize/restore.
7. **Elevation boundary (S7 / UIPI)**: a window whose process has a higher integrity level
   than the core (or whose token the core may not read) is treated as elevated. Input,
   close and bounds on it return `ELEVATION_REQUIRED` *before* anything is sent (UIPI would
   otherwise drop input silently). Focus and listing are allowed. The opt-in elevated
   helper (Phase 2) is the only way to drive such windows.
8. **Tests**: testbed-based window tests run in the interactive gate; the browser flow F3
   (Chrome → close → Firefox search) and the human-UAC test are opt-in switches because they
   open the user's real browsers / need a human click.

## Consequences
Contract updated (window ref format, M3 tool args). Testbed gains "no stdin" mode so it can be
launched by `launch_app`.
