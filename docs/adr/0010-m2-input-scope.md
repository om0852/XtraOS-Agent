# ADR 0010 — M2 Input: scope, targeting and new error codes

- Status: accepted
- Date: 2026-11-07

## Context
M2 adds real input (docs/07 Phase 1). Element refs come from `observe` (M4) and window refs
from `list_windows` (M3), which do not exist yet. Input must still be safe (ADR 0005/0006).

## Decision
1. **Targets in M2 are points**: `x, y` in physical screen pixels (per-monitor DPI aware v2).
   `ref` targeting arrives in M4; window-relative coordinates in M3.
2. **`window` = native window handle (integer)** until M3 adds window refs.
   - Mouse tools: `window` optional; when given, the point's root window must be that window
     immediately before every button event (else `TARGET_OBSCURED`).
   - Keyboard tools (`type`, `press_keys`): `window` **required** and must be the foreground
     window before every key (else `NEEDS_HUMAN`). Never type into an unknown window.
3. **Guards on every action** (from S1/S9): abort with `USER_ACTIVE` if a mouse button or
   modifier key is physically held at the start, or the user moves the cursor during the
   action. Pressed modifiers/buttons are always released on any exit path (no stuck input).
4. **New error codes** added to the contract: `USER_ACTIVE`, `TARGET_OBSCURED`.
5. **Moved out of M2**: `set_value` (needs UIA element refs) → M4; drag from Explorer to
   another app (P1-IN-07) → M4 (needs perception to locate the file item). M2 proves real
   OLE drag-and-drop inside the testbed instead.
6. **Testbed** (`testbed/`): a WinForms app compiled with the .NET Framework `csc.exe` that
   ships with Windows (no install), PMv2-aware, top-most, reports control rectangles and
   state over stdin/stdout. Interactive tests only ever act on it.
7. **Logging**: typed text and all params are never logged (P1-LOG-02).

## Consequences
The contract (docs/04) and test plan (docs/08) are updated: M2 tools list point targets
and `window` handles; P1-IN-07 and `set_value` move to M4.
