# ADR 0037 — P2-M8, P3-M1..M3: findings and gate result

- Status: accepted. The automated gate passed. Touch/pen real-input tests (P3-TP-01..03)
  wait for the hands-off run.
- Date: 2026-11-07

## What was built (ADR 0036)
- **P2-M8, the rules editor** (in `--viewer`): a list per class, app-rule lines, rate, Save.
  - Save keeps keys it does not know.
  - It refuses to loosen hard rules (credential entry, purchase) and to overwrite a broken
    `policy.json`.
  - The core still asks before applying.
- **P2-M8, the hard rule:** every `xtraos-core.exe` window (editor, admin helper, other
  cores) is `deny` for the agent, and the user cannot loosen it. `ocr` and `frame_diff` on
  the foreground window now also pass the app rules.
- **P3-M1 `frame_diff`:**
  - a 16 px grid of average colours with a ±6 threshold;
  - 8-connected regions, at most 16;
  - `wait_ms` polls every 50 ms;
  - a moved or resized window counts as fully changed;
  - no pixels returned.
- **P3-M2 `record_session`:**
  - refs, windows and points become locators;
  - typed text becomes arguments;
  - password targets are never recorded;
  - 50 steps (the batch limit), 30 min;
  - replay through `run_skill`, where the batch resolves locators before the policy check
    of each step;
  - the overlay shows "● REC".
- **P3-M3 `touch` / `pen`:**
  - InjectSyntheticPointerInput with up to 4 touch contacts (tap, long_press, drag, pinch,
    swipe), or a pen with pressure; pen paths are filled to ≤ 8 px per frame;
  - polite, kill switch per frame, contacts always lifted;
  - 3–4 finger swipes are `system_settings`, so they ask first.
- MCP: 33 tools. Gate `tests/phase3/p3-gate.ps1` (`-Automated` / `-Interactive`).

## Findings
1. **The testbed control channel dropped arguments.** It kept only a–z, as a guard against a
   stray BOM, so the new `settext` was silently ignored. Frame diff looked broken (0.0 %
   changed) until a screenshot comparison showed the text never changed. The command word is
   still guarded; `settext` / `latertext` keep their arguments.
2. **Two stray real touch taps were injected during development.**
   - A draft test aimed a tap at the XtraOS banner to prove XtraOS windows are refused.
     Between actions the banner is hidden, so the point (top-centre of the screen) hit
     whatever window was there.
   - It ran twice: once as written, and once more because a failed edit left it unchanged.
   - The check was removed. A tap can only be aimed safely at a window that is certainly on
     top, and the tool refuses XtraOS and admin windows in its point check before injecting.
   - Lesson kept for all real-input tests: never aim real input at a point unless the test
     owns the window there and it is in front. Tests that inject real input run only in the
     hands-off gate.
3. **`record_session` was a reserved (`NOT_IMPLEMENTED`) method.** The protocol and IPC
   tests used it as their example of a planned method. They now use `agent_workspace`
   (P3-M4, owner decision).
4. **Recordings are capped at 50 steps, not the 500 in ADR 0036,** so that every recording
   can replay as one batch.

## Results
- Unit tests: 75.
- Automated end-to-end tests:
  - editor ED-01/03;
  - frame diff FD-01..03: region over the text field; `wait_ms` returned 614–752 ms after a
    change scheduled at 600 ms;
  - record/replay RR-01..04;
  - touch TP-04.
- Gate `tests/phase3/p3-gate.ps1 -Automated`: **14/14 PASS**.

## Open
- Hands off: P3-TP-01..03 (touch tap, pen stroke, kill switch during a long press), plus
  the earlier interactive suites.
- P3-M4 Agent Workspace / Windows Sandbox: needs a Windows feature enabled (owner).
- P3-M5 macOS: needs a Mac.
