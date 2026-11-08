# 04 — MCP Tool Contract (v0)

This is the **only** public surface. Adding, removing, or changing a tool requires an ADR and
an update here first. Status: `P1` = Phase 1, `P2` = Phase 2, `P3` = Phase 3.

## Conventions

- Targets: `ref` (from `observe`) is preferred. Otherwise `window` (title substring or `w3`
  ref) + `x,y` with `space: "window" | "screen"` (physical pixels, per-monitor aware).
- Outputs are compact text, not pretty JSON. Every result ends with a one-line status.
- Every tool accepts optional `timeout_ms`.
- Errors: `{ code, message, hint }` with codes from the list below.

## Error codes

`NOT_PAIRED, STOPPED, BLOCKED_BY_POLICY, NEEDS_HUMAN, ELEVATION_REQUIRED, WINDOW_NOT_FOUND,
ELEMENT_NOT_FOUND, STALE_REF, TIMEOUT, INVALID_ARGS, NOT_IMPLEMENTED, INTERNAL,
USER_ACTIVE` (user held a button/modifier or moved the mouse — action aborted, ADR 0010),
`TARGET_OBSCURED` (the point is not inside the given window — nothing clicked, ADR 0010),
`STALE_REF` (the element of a ref is gone or its window closed — observe again, ADR 0015).
`NEEDS_HUMAN` also when the screen is locked or a UAC/secure prompt is showing (ADR 0025).

## Perception

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `observe` | `window?` (default foreground), `scope_ref?` (one element's subtree, P2-M1), `full?`, `max_elements?` (default 120) | element lines with refs, or a diff (`+`/`-`/`~`) on repeat; `opaque` flag | P1 (M4) |
| `screenshot` | `window?` or `region?` `{x,y,w,h}`, `scale?` (default 0.5, 0.1–1) | PNG + mapping `screen = origin + px/scale` | P1 (M4) |
| `read_text` | `window` or `ref`, `last_lines?` | text (UIA TextPattern / Value / Name); never password values | P1 (M4) |
| `find` | `query` (name/role/automation-id substring), `window?`, `deep?` (virtualised list rows via ItemContainerPattern; exact name fast, partial scan ≤ `timeout_ms`, default 3000, max 15000) | matching refs (≤ 20); off-screen rows marked `(offscreen)`, virtualised ones `(virtualized: realized when used)`; `click(ref)` on a row selects (never opens) | P1 (M4), deep P2 (M7) |
| `ocr` | `window?` or `region?` `{x,y,w,h}`, `query?`, `max_lines?` (80), `language?` | lines `"text" @x,y wxh` in screen pixels (click-ready); Windows built-in OCR, offline | P2 (M2) |

## Windows & apps

M3 (ADR 0012): a window is referenced as `w<handle>` (e.g. `w460650`) or the integer handle.

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `list_windows` | `filter?` (substring of title or process) | `w460650 chrome.exe "Google - Chrome" 0,0 1920x1040 focused` lines; `agent` = opened by the agent (P2-M4) | P1 (M3) |
| `focus_window` | `window` | ok (else `NEEDS_HUMAN`) | P1 (M3) |
| `launch_app` | `app` (name/path/AUMID or app-map service), `args?`, `elevated?`, `reuse?` (default true: same app+args still open from the agent → that window), `timeout_ms?` | ref of the new (or reused) window | P1 (M3; reuse P2-M4) |
| `close_window` | `window`, `timeout_ms?` | ok (graceful WM_CLOSE; else `NEEDS_HUMAN`); a window the agent did not open asks the user first (P2-M4) | P1 (M3) |
| (CLI) | `xtraos-core call <tool> [json \| -] [--json] [--out f.png]` | the same tools from any shell / LLM agent without MCP; pairs with the running core (starts it if needed); same policy, dialogs, kill switch, log | P3 (ADR 0039) |
| `cleanup` | — | closes every still-open window the agent opened (only those); lists what is left | P2 (M4) |
| `audit_export` | `from?`, `to?` (YYYY-MM-DD, local, inclusive), `format?` csv\|json, `out?` | the action log for the range with seq/time/session/client/method/result/policy/hash; chain check + head hash in the result; never overwrites; protected paths refused | P4 (ADR 0042) |
| `sandbox` | `op`: status \| start (`network?`) \| call (`tool`, `args`, `timeout_ms?`) \| stop | Windows Sandbox desktop; a second XtraOS inside; host ↔ inside via numbered request files in `%USERPROFILE%\.xtraos-sandbox` (token, size limits, untrusted replies); `start` without the feature, or with a Sandbox the user opened → `NEEDS_HUMAN`; first start can take minutes (`start` again keeps waiting); inside, text via `set_value` and buttons via `click` ref (UIA), `type` needs the Sandbox window in front → `NEEDS_HUMAN` (ADR 0040) | P3 (M4) |
| `system` | `op`: battery \| brightness (`value?` 0–100) \| wifi / bluetooth (`state?` on\|off) \| lock \| sleep | OS APIs, background; radio switches, lock and sleep are `system_settings` (ask); shutdown/restart/sign-out, firewall, antivirus, accounts, network config, registry are deliberately not offered | P3 |
| `volume` | `op`: get \| set (`value` 0–100) \| change (`by` −100..100) \| mute \| unmute | Windows master volume of the default speakers via Core Audio, background; `{volume, muted, before}` | P3 |
| `touch` | `action`: tap \| long_press \| drag \| pinch \| swipe; `x`,`y`, `to_x`,`to_y`, `fingers` (1–4), `scale`, `distance`, `duration_ms`, `idle_ms` | real synthetic touch (InjectSyntheticPointerInput), polite; 3–4 finger swipes = `system_settings` (ask); never on XtraOS/admin windows | P3 (M3) |
| `pen` | `action`: tap (`x`,`y`) \| draw (`points` [[x,y,pressure?]…] ≤ 2000); `pressure` 0–1024 | real synthetic pen with pressure, path filled ≤ 8 px per frame | P3 (M3) |
| `record_session` | `op`: start (`name`) \| stop (`overwrite?`) \| status \| discard | records successful actions as macro skill `skills/<name>.json` with locators (process + title, role + exact name, window-relative points); typed text → arguments; password targets never recorded; ≤ 50 steps / 30 min; replay = `run_skill` | P3 (M2) |
| `frame_diff` | `window?` \| `region?`, `wait_ms?` (≤ 60000), `min_pct?` (default 0.5), `reset?` | changed regions (screen px, ≤ 16, merged 16 px cells) + `changed_pct` since the previous call on the same window/region; first call = baseline; no pixels | P3 (M1) |
| `browser` | action: list_browsers \| list_tabs \| tab_info \| dom \| ax_tree \| wait_for \| click \| type \| press_key \| scroll \| navigate \| open_tab \| switch_browser; selector, text, key, url, tab_id, … | web pages through the XtraDevPilot bridge (loopback + pairing token); same policy as desktop input (messaging send asks, purchase/credential fields refused); scripts, cookies, storage, uploads not exposed; bridge/extension missing → `NEEDS_HUMAN` | P2 (M6) |
| `elevated_helper` | op: start \| stop \| status | start: core dialog (class `elevated`) then the Windows UAC prompt; afterwards tools on admin windows are forwarded to the helper (`via: "admin_helper"`, refs ≥ e50000000), each admin window needs the user's OK in the helper's own dialog; declined UAC → `NEEDS_HUMAN` | P2 (M5) |
| `window_bounds` | `window`, `x?,y?,w?,h?` (visible frame), `state?` (minimize/maximize/restore) | bounds + state | P1 (M3) |

Input, close and bounds on an elevated window return `ELEVATION_REQUIRED` (ADR 0012).

## Input

M2 (ADR 0010): targets are `x, y` physical screen pixels. M3: `window` = `w<handle>`.
M4 (ADRs 0014, 0015): `ref` targets act **in the background** first (UI Automation pattern, no
cursor, no keys); results end with `mode=background` or `mode=input`. Real input waits until
the user is idle ≥ `idle_ms` (default 800, 0 = off) and puts the cursor back afterwards
(`restore_cursor`, default true; `mouse` defaults to false because gestures span calls);
all real-input tools accept `idle_ms`. `drag` ends can be refs: `from_ref`, `to_ref`.

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `click` | `x, y` or `ref`; `window?`; `button?` (left/right/middle), `clicks?` (1–3), `modifiers?` (ctrl/shift/alt/win) | ok | P1 (M2) |
| `type` | `window` (required, must be foreground), `text` (≤ 10,000 chars, unicode), `submit?` | ok; returns once the app has the text (M6: `settled`) | P1 (M2) |
| `press_keys` | `window` (required, must be foreground), `keys[]` e.g. `["Ctrl+S"]` | ok | P1 (M2) |
| `scroll` | `x, y` or `ref`, `window?`, `dx?`, `dy?` (notches; dy > 0 scrolls down) | ok | P1 (M2; ref M4) |
| `drag` | `from_ref` or `from_x, from_y`; `to_ref` or `to_x, to_y`; `window?`, `to_window?`, `steps?`, `step_ms?`, `hold_ms?` | ok | P1 (M2; refs M4) |
| `mouse` | `action` (move/down/up/path), `x, y` or `points[[x,y],...]`, `button?`, `window?` | ok | P1 (M2) |
| `set_value` | `ref`, `value` (UIA ValuePattern, no typing; password field → `NEEDS_HUMAN`) | ok `mode=background` | P1 (M4) |
| `clipboard` | `op` (get/set), `text?` | text | P1 (M2) |
| `batch` | `steps[]` (≤ 50) of `{tool, args}`: input, window and read tools, ocr, app_map, built-in `run_skill`; each step passes the policy; `stop_on_error?` (default true) | one line per step | P2 (M1; M3) |

## Flow control & human in the loop

M5 (ADR 0017): every OS action passes the core's policy engine first (`BLOCKED_BY_POLICY`;
`confirm` classes show a core dialog to the user). After the kill switch (Ctrl+Alt+Esc or tray
Stop) every call returns `STOPPED` until the user resumes from the tray; there is no resume tool.

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `wait_for` | `window?`, `window_title?`, `text_contains?`, `ref_gone?`, `window_gone?`, `idle_ms?`, `timeout_ms?` (default 10 s, max 120 s); all given conditions at once | met conditions (+ window ref) / `TIMEOUT` | P2 (M1; was P1, not built in Phase 1) |
| `ask_human` | `message`, `kind` (confirm/choice/info/task), `options?` (choice: 2–5), `timeout_ms?` (default 120 s) | answer (`yes`/`no`/option/`ok`); `task` → `request_id` at once | P1 (M5) |
| `wait_for_human` | `request_id`, `timeout_ms?` | `done` / `denied` (else `TIMEOUT`) | P1 (M5) |
| `status` | — | running/stopped, paired client, kill hotkey, policy file (M5) | P1 |

## Skills & memory (P2)

| Tool | Args | Returns |
|---|---|---|
| `list_skills` | `app?` (name prefix, e.g. `files`) | one line per skill: name(args) — description (P2-M3) |
| `run_skill` | `name`, `args` | result (one call, many steps); built-in skills + JSON macro skills; policy per action/step (P2-M3) |
| `app_map` | `op`: `scan(query?)` / `list` / `get(service)` / `set(service, surface, location)` / `remove(service)` | installed apps or entries; stored DPAPI-encrypted; `launch_app` accepts mapped services (P2-M3) |

## Output example (`observe`)

```text
w460650 "Untitled - Notepad" notepad.exe 1280x720
e1 MenuItem "File"
e2 MenuItem "Edit"
e5 Document "Text editor" = ""
e9 Button "Close"
-- 5 elements

(repeat call on the same window: diff only)
~ e5 Document "Text editor" = "hello"
+ e12 Button "Save"
-- diff: 1 changed, 1 added, 0 removed
```
