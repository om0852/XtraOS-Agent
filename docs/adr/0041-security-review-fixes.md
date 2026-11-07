# ADR 0041 — Fixes from the Antigravity security review (2026-11-07)

- Status: accepted, built.
- Date: 2026-11-07

## Context
The owner had Antigravity (Claude Sonnet 4.6) review the whole project; XtraOS drove
Antigravity. It reported 8 findings. Each was checked against the code before fixing.

| # | Finding (reviewer's severity) | Verdict |
|---|---|---|
| 1 | Admin-helper 10-minute window grant lets any later action through without asking (critical) | **Valid.** It was a documented design choice (ADR 0030 residual risk), but too broad: a grant also covered typing commands into an admin console. |
| 2 | Any local process can post `WM_COMMAND ID_RESUME` to the hidden UI window and undo the kill switch (high) | **Valid.** `safety.rs` accepted ID_STOP / ID_RESUME / ID_QUIT / ID_VIEWER from any message. |
| 3 | Any medium-integrity process can click Allow in an approval dialog (`BM_CLICK` / `WM_COMMAND`) (high) | **Valid.** `prompt_proc` acted on any button command. Our own tests answered dialogs exactly that way. |
| 4 | The `launch_app` approval hides the command-line arguments (high) | **Valid.** The dialog said "launch cmd as administrator" without the arguments. |
| 5 | Grants survive PID/HWND reuse (high) | **Valid in principle.** The grant checked HWND + PID, not that it was the same process instance. |
| 6 | The word lists miss "Delete all", "Delete account" and similar; `mouse` / `touch` / `pen` skip the name check (high) | **Valid.** The name rules only ran for `click` / `set_value` / browser clicks. |
| 7 | Helper watchdog thread waits on a raw handle the main thread may close (medium) | **Valid.** It was a narrow race at process exit. |
| 8 | Unaligned cast of a `Vec<u8>` to `SID_AND_ATTRIBUTES` (medium) | **Valid.** Formally undefined behaviour, in `winsec.rs` and in `winmgr.rs` (integrity). |

## Decision (fixes)
1. **Helper grants cover reading only.** "Allow reading this window for 10 minutes" covers
   observe, find, read_text, wait_for, scroll and focus. **Every input to an admin window**
   (type, keys, click, set_value, drag, mouse, close, bounds) asks each time and shows the
   text or keys.
2. **Commands from window messages.** The hidden UI window accepts only **Stop** and
   **Quit** as messages; both are safe directions and the uninstaller uses Quit. **Resume**
   and **Rules and activity…** come only from the tray menu itself (TrackPopupMenu return).
3. **Approval dialogs accept "yes" only from real hardware input.**
   - A button other than the safe default counts only if the current input message comes
     from hardware (`GetCurrentInputMessageSource` origin `IMO_HARDWARE`).
   - Injected input (SendInput), `BM_CLICK` and posted messages are ignored.
   - The safe answer (Deny / Esc / close) is always accepted.
   - Tests answer dialogs through an in-process call (`safety::answer_prompt_for_tests`); a
     separate process cannot use it.
   - The unelevated test-mode admin helper accepts synthetic answers. It has no extra power,
     and an elevated helper is protected by UIPI as well.
   - Trade-off: screen-reader Invoke on these buttons is ignored. Keyboard and mouse work.
4. **`launch_app` dialogs and the log show the arguments** (up to 200 characters).
5. **Grants are tied to the process instance:** PID + process creation time + HWND.
6. **More words, and the name check for every pointer action.**
   - Delete words gain: delete all, delete account, delete everything, remove all,
     clear all, erase all, delete forever, close account, deactivate account (and the
     localised equivalents where common).
   - Purchase words gain: order now, confirm and pay, complete order, pay with…,
     place order and pay.
   - `mouse` down/up, `touch` tap/long_press and `pen` tap now look up the element under the
     point and run the same name rules as `click`.
7. **The helper watchdog opens its own handle** to the core process.
8. **Token buffers are allocated as `Vec<u64>`** (8-byte aligned) before casting.

## Tests
- Unit: grant scope; process instance; new words; message filter.
- Proof for item 3: an XtraOS **SendInput** click on the Allow button is ignored (injected
  input), while Deny is accepted. A hardware click is recognised by the same API (the origin
  reads IMO_INJECTED for SendInput, so the call does see the click's input message); the
  owner confirms the hardware path in the hands-off session.
- All existing suites, updated to answer dialogs in-process.

## Results
- Unit: 84 tests, including grant scope, process instance, new words and mouse/touch/pen
  name rules.
- SEC-01: a `BM_CLICK` on Allow from another thread was ignored and the dialog stayed open;
  Deny was accepted and the action was blocked.
- SEC-02: a posted Resume left the kill switch on; a posted Stop still worked.
- EH-05: with a 10-minute grant, a read went through without a dialog, and the click asked
  again.
- Phase 3 gate (`-Automated`): every step passed except the admin-helper suite. Its EH-07
  still resumed through the now-blocked posted message. It was switched to the in-process
  call and re-run: 5/5. fmt, clippy and every non-interactive test pass after the change.
- All suites now answer dialogs in-process.
- **Not yet verified: a real hardware click on Allow.** If `GetCurrentInputMessageSource`
  did not report hardware for it, approvals would fail closed (nothing approved), not open.
  SEC-03 (an injected click is ignored) and one real Allow click by the owner settle it in
  the hands-off session.
