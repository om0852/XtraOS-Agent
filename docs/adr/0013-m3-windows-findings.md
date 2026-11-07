# ADR 0013 — M3 Windows & apps: findings and gate status

- Status: accepted. **M3 closed** 2026-11-07 (human-assisted gate run, see Resolution).
- Date: 2026-11-07

## What was built
- `core/xtraos-core/src/winmgr.rs`: list_windows (Alt+Tab filter, DWM frame bounds, process,
  state, focused, elevated), focus_window (restore + AttachThreadInput focus, verified),
  launch_app (ShellExecuteExW open/runas, new-window detection, MSVCRT argument quoting,
  AUMID support), close_window (graceful WM_CLOSE only), window_bounds (visible-frame
  x/y/w/h, minimize/maximize/restore). `SHELLEXECUTEINFOW` layout asserted (112 bytes).
- Elevation boundary: integrity-level check; input/close/bounds on elevated windows (or at a
  point over one) → `ELEVATION_REQUIRED` before anything is sent.
- `window` args accept `w<handle>` refs everywhere (input too). MCP: 5 new tools (13 total).
- Tests: 6 new unit tests; `tests/windows_e2e.rs` (5 interactive; UAC + browsers opt-in);
  shared helpers `tests/common`; gate `tests/phase1/m3-gate.ps1 [-Interactive] [-Uac] [-Browsers]`.

## Bugs found and fixed
1. **Testbed closed itself when started by launch_app**: in a windowless .NET app the missing
   stdin handle counts as "redirected" (`Console.IsInputRedirected` = true), so the control
   loop read EOF and closed the window; window tests then hit `SetWindowPos` on a dead handle.
   The control channel is now an explicit `--control` argument. SetWindowPos errors now carry
   the Win32 error code. Window tests clean up test-launched processes on failure.
2. **Gate summary hid internal SKIPs** (tests that print SKIP and return count as "ok"): the
   gate now prints every SKIP line. A SKIP is never a PASS (docs/08).

## Gate evidence so far (`m3-gate.ps1 -Interactive`)
fmt, clippy (-D warnings), 24 unit + 9 IPC, build, tsc, 7 MCP, testbed build, 11/11 input
e2e, window e2e — **9/9 steps PASS**. Within window e2e, win01 (launch → list → exact bounds
→ maximize/restore/minimize → focus from minimised → close) and win02 PASS; win03 (UAC),
win04 (elevated refusal) and win05 (browser flow F3) SKIPPED — they need the human run.

## Decision
M3 closes after `m3-gate.ps1 -Interactive -Uac -Browsers` passes with win03–win05 actually
executed (owner clicks Yes on UAC, then closes the admin Command Prompt).

## Resolution (2026-11-07)

`tests/phase1/m3-gate.ps1 -Interactive -Uac -Browsers`, owner clicked Yes on UAC:

| Step | Result |
|---|---|
| fmt / clippy -D warnings / 24 unit + 9 IPC / build / tsc / 7 MCP / testbed build | PASS |
| Input e2e (testbed) | PASS 11/11 |
| win01 launch → list → exact bounds → max/restore/min → focus → close | PASS |
| win02 unknown app / bad refs | PASS |
| win03 (P1-WIN-07a) `launch_app cmd elevated` via UAC → window reported `elevated`; `type` → ELEVATION_REQUIRED | PASS |
| win04 (P1-WIN-07b) elevated `cmd.exe` w3344384: click, type, close_window, window_bounds → all ELEVATION_REQUIRED, nothing sent | PASS |
| win05 (P1-WIN-06, flow F3) Chrome → "xtraos agent test - google search - google chrome", closed; Firefox (no extension) → "xtraos agent test - google search — mozilla firefox", closed | PASS |
| **Gate** | **9/9 PASS, 0 SKIP** |

M3 is closed. The admin Command Prompt opened by win03 is closed by the owner (the core cannot,
by design).
