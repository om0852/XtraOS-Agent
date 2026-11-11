# ADR 0047 — P6-M4 Dialog awareness: `observe` and `status` report a dialog first

- Status: accepted (owner: "Yes, dialog awareness", 2026-11-10).
- Date: 2026-11-10

## Context
In a real session (docs/09-next-phases-plan.md §2, item 5) a career site raised a JavaScript
`alert` after an upload, and a "Leave site?" prompt blocked navigation. The page stopped
responding. The agent only found the dialog by reading a long `observe full:true`: it was buried
among the browser's other elements. Classic Windows message boxes ("Save changes?") are worse:
they are separate owned windows, so `list_windows` (which shows unowned windows only) and
`observe` of the app window never show them at all.

Spike, 2026-11-10 (Edge 14x, Windows 11 26200, `spikes`-style UIA dump):
- A JavaScript `alert` in Edge/Chrome is **inside the browser window's UIA tree**: an element
  with ControlType `Window`, `IsDialog = true` (UIA property 30174, Windows 10 1809+),
  name "This page says", with a `Text` child holding the message and `Button` children. It has
  no window handle of its own.
- A Win32 message box (`MessageBox`, class `#32770`) is a separate top-level window **owned**
  by the app window; the owner is disabled while it is open.

## Decision
1. **In-tree dialogs.** `IsDialog` is added to the cached properties (optional: if the property
   cannot be added on an old Windows build, nothing else changes). In `observe`, the first
   visible element (after the root) with `IsDialog = true` is the window's dialog. Its subtree
   is always kept (it is never dropped by `max_elements`).
2. **Owned dialogs.** For the observed window `h`, visible top-level windows whose owner chain
   reaches `h` count when their class is `#32770` or `h` is disabled (a modal is open).
   Tooltips and menus do not count (they never disable the owner and are not `#32770`). The
   front-most one is reported; its elements get refs like any other element (their window is
   the dialog's handle, so `click {ref}` works unchanged).
3. **Report it first.** `observe` prints a line right after the header, in both the full list
   and the diff:
   `!! dialog e45 "This page says": "XtraOS test alert" — buttons: e46 "OK", e47 "Cancel". Answer it first; the window may not respond until it closes.`
   (For an owned dialog the ref is its window `wN`, and the line says `(blocking)` when the
   owner is disabled.) The JSON result gets `dialog: {ref, title, text, buttons:[{ref,name}],
   kind: "in_page"|"window", blocking}`.
   When the observed window is itself an owned dialog, the header ends with `dialog of wN`.
4. **`status`** gets `dialog`: the foreground window's owned dialog (rule 2) or the foreground
   window itself when it is an owned dialog, as `{window, title, process, owner, blocking}`;
   `null` otherwise. It does not walk UIA trees (status must stay cheap), so in-page browser
   dialogs show in `observe` only. This limit is documented.
5. **Safety.** XtraOS's own windows (confirmations, ask_human) are never reported or given refs
   (`safety::is_own_window`). An elevated dialog is reported by title only, with no element
   refs, and the elevated-helper hint. Nothing is answered automatically: the agent decides,
   and the normal policy applies to the click (a "Leave"/"Delete" button is still an action).

## Not decided here
- A `wait_for {dialog: ...}` condition and dialog notices in action results (P6-M7 area).
- Firefox/Electron in-page dialogs that do not set `IsDialog` (verify when met; until then
  "planned").

## Tests
- Unit: dialog line formatting, owned-dialog rule (pure function over (class, owner disabled)).
- Interactive (testbed): new `modal` command opens a real owned `MessageBox`; `observe` of the
  testbed reports it first with its buttons, `status.dialog` names it, clicking its button ref
  closes it and the testbed records the answer.
- Interactive (browser): Edge with a throw-away profile shows an `alert`; `observe` reports
  `!! dialog` with the message and the OK button.
