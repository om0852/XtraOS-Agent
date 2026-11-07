# ADR 0031 — P2-M5 Elevated admin helper: findings and gate result

- Status: accepted (built, automated tests pass). **P2-M5 closes after the human F5 run**
  (real UAC prompt, `XTRAOS_TEST_UAC=1`, `tests/phase2/p2m5-gate.ps1 -Interactive`).
- Date: 2026-11-07

## What was built (ADR 0030)
- `helper.rs`: core side (start through `runas`, connect, check the server PID, `hello`,
  forward, stop, kill-switch hook) and helper side (`xtraos-core.exe --elevated-helper`).
  - Strict arguments.
  - Opens the core process and checks the core's image is its own; test mode skips that
    check and is refused when elevated.
  - First-instance pipe with only the core's user SID, network denied, medium label.
  - Accepts only the core's PID, once.
  - Its own approval dialog: *Allow once* / *Allow this window for 10 minutes* / *Deny*,
    60 s timeout.
  - Hard deny list; acts only on windows above the core's integrity; never password
    fields; 120 actions/min.
  - Exits with the core, when the pipe closes, after 30 s without a connection, after
    15 idle minutes, or from its tray Stop.
- Routing (`exec.rs`):
  - Refs ≥ e50000000 go to the helper.
  - On ELEVATION_REQUIRED, the core forwards UI methods while the helper runs.
  - `dispatch_local` runs inside the helper, so requests are never forwarded twice.
- Safety UI:
  - Helper mode: its own tray text, no global hotkey.
  - `gather_targets` is shared with the helper's approval.
  - The kill switch ends the helper.
- `status` shows `admin_helper`. MCP tool `elevated_helper` (28 tools).
- Uninstall waits for an elevated helper to exit.
- Gate: `tests/phase2/p2m5-gate.ps1`:
  - `-Automated`: no human.
  - `-Interactive`: hands off, plus UAC.

## Results
- Automated P2-EH-01..07: 6/6. Five ran; F5 skips without `XTRAOS_TEST_UAC`.
  - Squatted pipe → exit 6.
  - Foreign client → exit 8.
  - Bad arguments → exit 2.
  - Helper refs are e50000001+; one dialog for the window, then none.
  - Deny → BLOCKED, and the dialog shows the value.
  - Kill switch → the helper process ends.
- Gate (`-Automated`): **11/11 PASS** — fmt, clippy -D warnings, unit 61 + IPC, MCP 10/10,
  release bundle, install test 12/12 with 28 tools, helper e2e.

## Not yet verified (needs the human)
- The real elevated path:
  - Can the medium core open the helper's pipe, which an elevated process created with a
    medium label?
  - Does UIA input reach an admin Command Prompt?
  - Which publisher name does the UAC prompt show?
  - All of this is the F5 test.
- The full interactive regression (P2-M1..M4, safety, flows, bench) since the
  approval-foreground change (ADR 0029).

## Residual risk (from ADR 0030)
- Malware injected into the medium core could use an active 10-minute window grant.
- The per-user install folder is writable, so a replaced binary is what UAC would elevate.
  Code signing (owner decision) is the fix.
