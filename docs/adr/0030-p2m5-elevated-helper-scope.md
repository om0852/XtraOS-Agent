# ADR 0030 — P2-M5 scope: the elevated (admin) helper

- Status: accepted (scope). Findings and gate → ADR 0031.
- Date: 2026-11-07

## Context
Windows blocks a normal (medium-integrity) program from sending input to, or reading the UI
of, an administrator window (UIPI). The core therefore returns `ELEVATION_REQUIRED` for admin
windows (M3). Flow F5 (admin Command Prompt → `ipconfig`) needs a helper that runs elevated
(ADR 0021: opt-in, one UAC approval, accepts only the core).

An elevated helper that takes orders from a normal process is a privilege-escalation path:
malware running as the same user could try to drive it, or could inject code into the core
and drive it from there. The design below closes the first path and contains the second.

## Decision
### Shape
- **Same binary, its own mode:** `xtraos-core.exe --elevated-helper --pipe <name> --core-pid <pid>`.
  It runs only for the current session. **Nothing elevated is installed**: no service, no
  scheduled task, no auto-elevation. Every start is a fresh Windows UAC prompt that only the
  human can answer.
- New tool **`elevated_helper {op: start|stop|status}`**.
  - `start` is class `elevated`, so the core first shows a confirm dialog explaining what the
    UAC prompt is for, and then Windows shows UAC.
  - A declined UAC prompt returns `NEEDS_HUMAN`.

### Channel: one connection, only the core
1. The core picks a random pipe name (`\\.\pipe\xtraos-elev-<128-bit hex>`) and starts the
   helper with `runas`, keeping the process handle.
2. The helper:
   - opens the core process and keeps that handle (so the PID cannot be reused);
   - checks that the core's image path is exactly its own;
   - creates the pipe as the first and only instance (a squatter makes it exit). The pipe
     allows only the core's user SID, denies network logons, and carries a medium mandatory
     label so the core can open it.
3. On connect, the helper requires the client PID to be the core PID; otherwise it drops the
   client and exits. It never listens again.
4. The core requires the server PID to be the helper it started, opens the pipe with
   identification-level QoS, and exchanges `hello`.

### Routing
- The core runs every action as before. When the result is `ELEVATION_REQUIRED`, the method
  is a UI method (observe, find, read_text, wait_for, set_value, click, type, press_keys,
  scroll, drag, mouse, focus_window, close_window, window_bounds) and the helper is
  connected, the core forwards the same request to the helper.
- The helper's element refs start at **e50000000**. Any request with such a ref goes
  straight to the helper.
- Everything else stays in the core: launch (UAC), screenshots, OCR, the clipboard, skills,
  the app map.
- The core's policy, overlay, kill switch and action log still apply to forwarded actions.

### Approval inside the helper
The helper runs elevated, so other normal programs cannot click its dialogs. That makes this
approval the real gate, even against a compromised core.
- The helper asks before the **first action on each admin window**: *Allow once* /
  *Allow this window for 10 minutes* / *Deny*. No answer within 60 s means deny.
- A grant is tied to that window and ends when the window closes.
- Hard rules that do not come from user-writable files:
  - Targets must be windows that need elevation: higher integrity than the core, not the
    core's own process.
  - Never Windows Security, the UAC/credential/logon UIs, or XtraOS windows.
  - Never type into password fields.
  - At most 120 actions per minute.
  - Keyboard input needs a target window.
- The helper is visible as its own tray icon ("XtraOS admin helper"); its menu stops it.

### Lifetime: the helper exits when
- the pipe closes (the core quits, `elevated_helper stop`, or the core's kill switch);
- the core process ends;
- nobody connects within 30 s;
- it has been idle for 15 minutes;
- the user chooses Stop in its tray menu.

After a kill switch, the agent must start the helper again, which means a new UAC prompt.

### Known residual risk (documented, not solved)
Malware already running as the user can inject into the medium core. While a 10-minute
window grant is active, it could act on that one admin window. Against that: grants are
per window, short and visible (tray), and *Allow once* is the default button. A per-user
install folder is writable by the user, so a replaced binary would be what UAC elevates;
code signing (owner decision, ADR 0019) is the fix that UAC can show.

## Tests (docs/08 P2-EH-*)
- **Automated, no human:**
  - unit tests: arguments, pipe SDDL, ref routing, deny list, grant expiry;
  - helper started **unelevated** in test mode (`--test-same-integrity`, refused when the
    process is elevated): handshake; a squatted pipe name; a foreign client is refused; it
    exits when the core disconnects; forwarded observe and click on the testbed with
    helper-range refs; the test answers the helper's dialog; Deny blocks; the kill switch
    ends the helper.
- **Human (later):** F5 with a real UAC prompt, opt-in via `XTRAOS_TEST_UAC=1`.

## Consequences
- docs/04: `elevated_helper` tool, 28 tools.
- docs/05: helper threat model.
- docs/08: P2-EH cases.
- Uninstall: stopping the core ends the helper; uninstall waits for it.
