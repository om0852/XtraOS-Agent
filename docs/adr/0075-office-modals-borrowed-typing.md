# ADR 0075 — Office modal dialogs, borrowed typing into Electron, transient tree reads

- Status: accepted (owner: "Yes, fix them", 2026-11-11)
- Date: 2026-11-11

## Context
Three failures seen on 2026-11-11 (Antigravity live run and the demo recording):
1. **Excel's "Create Table" dialog was invisible to XtraOS.** `observe` gave no `!! dialog`
   line, `find` and the window screenshot missed it, and `press_keys` went to the workbook,
   which was left in "Point" mode. Measured: the dialog is a visible top-level window, class
   `NUIDialog`, owned by a *hidden, disabled* second `XLMAIN` window of the same process; the
   visible workbook window stays enabled. `owned_dialog` only followed owner chains to the
   visible window, so it never matched.
2. **Borrowed typing into Antigravity (Electron) lost the end of a 182-character prompt.** The
   settle check (idle + the focused field's value stops changing, ADR 0049) was fooled: the
   input reports only its first 17 characters through UI Automation, so the value looked stable
   at once and the window was given back while the renderer process was still taking keys.
3. **`find` failed once with STALE_REF while reading a whole window** (Paint, mid-drawing). A
   window that still exists cannot be stale; UI Automation failed one read during a redraw.

## Decision
1. `owned_dialog(h)` also matches a visible window of the same process whose owner is hidden
   and disabled (pure rule `hidden_owner_modal`); such a dialog blocks `h`. `observe` then
   shows its `!! dialog` line with refs, as for any owned dialog (ADR 0047).
2. `press_keys` and `type` without a `ref`: while a blocking dialog is open, the keys go to the
   dialog (`key_window`), not to the window behind it.
3. Borrowed typing into a Chromium/Electron window keeps the foreground
   `300 ms + 6 ms × characters` (max 2.5 s) after the settle check, before giving it back.
4. The whole-window tree read retries a STALE_REF twice (120 ms apart) while the window exists.

Not changed: `list_windows` still lists only unowned windows; a window screenshot still captures
only that window (observe names the dialog).

## Tests
- Unit `hidden_owner_modal_rule`, `chromium_drain_grows_with_text_and_is_capped`.
- Live (owner's PC): Excel Ctrl+T → observe shows the dialog, Enter reaches it; Antigravity
  long prompt typed while another window is in front arrives whole. Recorded in the plan.
