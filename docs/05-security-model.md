# 05 — Security Model

XtraOS Agent can do anything the logged-in user can do. Security is a product feature.

## Threats and controls

| Threat | Control |
|---|---|
| Another process or web page drives the core | Named pipe ACL = current user SID only, NETWORK denied, remote clients rejected, first-instance flag against squatting; 256-bit pairing token in a user-only file (ADR 0009); no TCP listener. Browser driver bridge binds 127.0.0.1 with origin check + token (as in the XtraDevPilot bridge security fix, commit 2a4e51b). |
| Prompt injection from on-screen content ("click Send to all") | Page/app text is returned as data. Policy engine enforces confirm/deny regardless of what the agent asks. Sensitive actions always confirm. |
| Agent runs away (loops, wrong window) | Kill switch (hotkey + tray), visible overlay, per-session action rate limit, max actions per task (configurable). |
| Secrets leak through logs or observations | Password fields (UIA IsPassword) are never read or typed; values masked in observations and logs. |
| Privilege escalation | Elevation only via UAC with a human click. Elevated helper is opt-in, separately installed, and only accepts the core's pipe. |
| Tampering with policy/log | Policy file and log in `%LOCALAPPDATA%\XtraOS\` with user-only ACL; log is append-only with a hash chain (P2). |
| Supply chain | Pinned dependencies, lockfiles committed, signed releases (before public launch). |

## Policy engine

Action classes: `observe`, `input`, `launch`, `close`, `file_send`, `message_send`, `delete`,
`purchase`, `system_settings`, `elevated`, `credential_entry`.

Default profile:

| Class | Default |
|---|---|
| observe, input, launch, close | allow |
| message_send, file_send, delete, system_settings, elevated | **confirm** |
| purchase, credential_entry | **deny** (human does it) |

App rules (default deny list): password managers, banking apps, Windows Security, Credential
Manager. Users edit `policy.json`; **a changed file applies only after the user confirms it in a
core dialog** (the agent cannot loosen its own rules, ADR 0026); every change is logged.

Protected paths (file skills, ADR 0026): Windows folder, Program Files, ProgramData, other
users' profiles, UNC paths, drive roots, XtraOS install + data folders → `BLOCKED_BY_POLICY`.
Deletes go to the Recycle Bin and always ask first.

Browser bridge (ADR 0032): the core is the only XtraOS client of the XtraDevPilot bridge.
- **Connection:** loopback only, `Sec-WebSocket-Accept` checked, the pairing token sent in
  an AUTH message (never in a URL). A bridge without a token is not used.
- **Allowlist:** read actions, click, type, keys, scroll, navigation (http/https only).
  Never script execution, cookies, storage, network mocks or file upload.
- **Policy:** the target tab's host and title feed the same rules as desktop windows.

Elevated admin helper (ADR 0030): opt-in per session, started only through a UAC prompt
(nothing elevated is installed). One pipe connection, only from the core that started it
(PID-checked both ways, squatting refused, same binary). The helper asks the user in its own
elevated dialog before the first action on each admin window (other programs cannot click
it), acts only on windows that need elevation, never on Windows Security / UAC / credential
UIs or password fields, and exits with the core, the kill switch, its tray menu or 15 idle
minutes. Residual risk: malware injected into the core could use an active 10-minute window
grant; code signing is the fix for a replaced binary.

Resource discipline (ADR 0028): the core tracks the windows it opens, reuses them instead of
opening duplicates, `cleanup` closes only those, and closing a window the user opened asks
the user first (`close_user_window`).

## Hard boundaries (not configurable)

No CAPTCHA solving, no bot-detection evasion, no secure-desktop/UAC/lock-screen automation,
no remote network control, overlay and kill switch cannot be disabled while actions run.
