# ADR 0053 — P7-M3 Developer pack, slice 2: "reproduce this bug" report

- Status: accepted (owner: "Build the bug report", 2026-11-11).
- Date: 2026-11-11

## Context
A developer asks an agent "reproduce this bug and give me a report". `record_session`
(ADR 0036) already records the agent's successful actions as a replayable macro skill with
locators. A bug report also needs: what the screen looked like after each step, the step that
**failed** (the recorder keeps only successes — but the failure is the point of the report), the
agent's own observations ("expected 42, got 41"), and one file the developer can attach.

## Decision
1. **`record_session {op: "start", name, report: true}`** also starts a report folder
   `Documents\XtraOS reports\<name>-<YYYYMMDD-HHMMSS>\` (the user's Documents known folder;
   `%TEMP%` if it is not available).
2. While it records, every recordable action that runs (`click`, `type`, `press_keys`,
   `launch_app`, … — the recorder's list) becomes a report entry: a one-line description
   (`click Button "Save" in "report.txt - Notepad"`, `type 12 chars "hello wor…"`), ok or
   **failed with the error code and message**, and a PNG of the action's window ~300 ms after
   it (`step-01.png`, scale 0.6). Failed actions are reported but, as before, never added to the
   replay skill. A step (or note) without a window of its own is pictured with the last window
   this recording used, else not at all — **never the foreground window**: the first live run
   pictured the owner's Claude app for a failed step (fixed, asserted in the test).
3. **`record_session {op: "note", text}`** adds the agent's observation to the report (≤ 1000
   characters), with a screenshot of the foreground window.
4. **`stop`** writes `report.md`: title, local time, duration, XtraOS and Windows versions, the
   apps involved, numbered "Steps to reproduce" with ok/FAILED and screenshot links,
   "Observations", and how to replay (`run_skill {name}`, when a skill was saved). It returns
   `report` (the path). A recording with only failed steps still gets its report.
5. **Privacy:** the report stays on the user's PC. Typed text appears clipped to 40 characters;
   password fields are never recorded (as before) and set_value/type on a password field is
   refused anyway; screenshots show what was on screen, so the report says that before sharing
   the user should look at the pictures.

## Tests
- Unit: step descriptions (pure), report Markdown (pure, from entries).
- Interactive: start with `report: true` on the testbed, a click, a type, a failing step
  (unknown ref), a note, stop → `report.md` exists, lists 3 steps with one FAILED, has the note,
  and the PNGs exist.
