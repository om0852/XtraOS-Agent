# ADR 0017 — M5 scope: kill switch, policy engine, human-in-the-loop, overlay

- Status: accepted (scope). Findings and gate result → ADR 0018.
- Date: 2026-11-07

## Context
M5 = Safety (docs/07): policy engine with defaults, HITL `ask_human`/`wait_for_human`, kill
switch, overlay. Exit: kill switch ≤ 100 ms; a confirm dialog blocks a `message_send`-class
action. docs/02 planned a separate Tauri tray app.

## Decision
1. **Safety UI lives inside the core, native Win32 — no Tauri in Phase 1.** One UI thread in
   `xtraos-core` owns the global hotkey, the tray icon, the overlay and the approval dialogs.
   Why: (a) security — approvals never travel over the pipe, so no paired client can fake a
   "Yes"; (b) the kill switch works even if every client hangs; (c) no WebView2, no new
   toolchain, ~0 MB extra, instant start (cost/perf goals). A richer tray (log viewer, policy
   editor) can still come later as a separate app. CLAUDE.md §5 and docs/02 are updated.
2. **Hard rule: no safety UI, no actions.** If the UI thread is not running, every OS action
   returns `INTERNAL` ("safety UI unavailable"). The hotkey is `Ctrl+Alt+Esc`; if another app
   owns it, `status` says so and the tray Stop remains (never "no kill switch at all").
3. **Kill switch** (flow F10): hotkey or tray "Stop" sets a process-wide stop flag. Every
   injected event checks it (`SendInput` choke point), loops check it between steps, held
   buttons/modifiers are released through the existing Drop guards (release events bypass the
   flag). New and queued actions → `STOPPED`, also `observe` and `ask_human` ("stop entirely").
   **Resume only from the tray menu** — there is no MCP tool for it.
4. **The agent can never touch XtraOS's own windows** (overlay, dialogs, tray): input at a
   point over them, refs in them, observe/close/focus of them → `BLOCKED_BY_POLICY`.
5. **Overlay**: top-centre banner, top-most, layered, click-through (`WS_EX_TRANSPARENT`),
   non-activating, excluded from screen capture (`WDA_EXCLUDEFROMCAPTURE`, so the agent's
   screenshots never show it). Shown while actions run (+2.5 s): "XtraOS Agent · <action> ·
   Ctrl+Alt+Esc stops". Typed text is never shown (only "typing N chars"). Stopped: red banner
   "stopped — resume from the tray icon" until resumed.
6. **Policy engine** (`policy.rs`, pure + unit-tested), evaluated in the core before every OS
   action; the agent cannot skip it:
   - Classes per docs/05: `observe, input, launch, close` allow; `message_send, file_send,
     delete, system_settings, elevated` **confirm**; `purchase, credential_entry` **deny**.
   - Recognition (English names for now, documented as heuristics — defence in depth, not the
     only control): element names like Send / Reply / Post / Submit → `message_send`; Buy /
     Pay / Place order / Checkout → `purchase`; Delete / Remove / Move to trash → `delete`;
     Enter / `submit` / Ctrl+Enter in a messaging app (process or title: WhatsApp, Telegram,
     Slack, Teams, Discord, Signal, Outlook, Gmail, …) → `message_send`; Delete keys in
     Explorer → `delete`; drag from Explorer into a messaging window → `file_send`; input in
     Settings / Control Panel / regedit / MMC → `system_settings`; `launch_app elevated` →
     `elevated`; typing or set_value into a password field, or any action on a Windows
     credential prompt → `credential_entry`. Real clicks at a point use the UIA element under
     the point for its name.
   - App rules: default **deny** for password managers (1Password, KeePass/KeePassXC,
     Bitwarden, LastPass, Dashlane, Enpass, RoboForm, NordPass, Keeper), Windows Security,
     Credential Manager and the Windows credential UI.
   - User file `%LOCALAPPDATA%\XtraOS\policy.json` (`classes`, `apps`, `rate_per_minute`),
     reloaded when it changes; each (re)load is written to the action log with a hash.
     Not overridable: `credential_entry` = deny; `purchase` at most confirm; own windows deny.
   - Rate limit: 600 OS actions/minute (configurable) → `BLOCKED_BY_POLICY` (runaway loop).
   - Confirm = a core dialog "Allow once / Deny" naming the action, app and class (no typed
     text); **Deny is the default button**, Esc/close = Deny, 60 s without answer = Deny →
     `BLOCKED_BY_POLICY`. Kill switch while it is open = Deny + `STOPPED`.
   - Every action-log line records the decision (`allow`, `confirmed`, `denied`, `blocked`).
7. **HITL tools**: `ask_human(message, kind, options?, timeout_ms?)` — `confirm` (yes/no),
   `choice` (2–5 options), `info` (ok) block until answered (default 120 s, max 600 s →
   `TIMEOUT`); `task` (e.g. "type your password in Chrome, then click Done") returns a
   `request_id` at once; `wait_for_human(request_id, timeout_ms?)` → `done` / `denied` /
   `TIMEOUT`. The dialog is labelled as the agent's question, so it cannot pass for a policy
   approval, and its answer never approves a policy action.
8. `status` reports `stopped`, `kill_hotkey`, `policy` (file + profile).

## Tests (docs/08)
P1-SAF-01..06 as planned, plus: own windows refused; ask_human confirm/choice/timeout;
task + wait_for_human; rate limit and policy classification (unit); password typing → deny.
Tests act as the human through direct Win32 messages to the dialog/tray window (the agent
path cannot reach them by design).

## Consequences
- docs/04: `ask_human` (+ `task` kind, `timeout_ms`), `wait_for_human`; `status` fields.
- docs/02 + CLAUDE.md §5: tray/overlay = core-hosted native UI in Phase 1 (this ADR).
- Not in M5: tray policy editor and log viewer (Phase 2), localisation of the name heuristics
  (Phase 2), hash-chained log (Phase 2, docs/05).
