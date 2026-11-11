# 04 — MCP Tool Contract (v0)

This is the **only** public surface. Adding, removing, or changing a tool requires an ADR and
an update here first. Status: `P1` = Phase 1, `P2` = Phase 2, `P3` = Phase 3.

## Conventions

- Targets: `ref` (from `observe`) is preferred. Otherwise `window` (title substring or `w3`
  ref) + `x,y` with `space: "window" | "screen"` (physical pixels, per-monitor aware).
- Outputs are compact text, not pretty JSON. Every result ends with a one-line status.
- Every tool accepts optional `timeout_ms`.
- Errors: `{ code, message, hint }` with codes from the list below. The hint of an `INVALID_ARGS`
  is one correct example call for that tool (P6-M3).

## Profiles and instructions (P5-M7, ADR 0044)

- `initialize` returns server `instructions`: a short how-to for the model.
- `XTRAOS_TOOLS=full` (default) lists every tool. `XTRAOS_TOOLS=core` lists 15: status,
  list_windows, launch_app, focus_window, observe, find, read_text, screenshot, click, type,
  press_keys, scroll, wait_for, list_skills, run_skill. A profile only shortens the list; every
  tool stays callable and policy-checked.

## Error codes

`NOT_PAIRED, STOPPED, BLOCKED_BY_POLICY, NEEDS_HUMAN, ELEVATION_REQUIRED, WINDOW_NOT_FOUND,
ELEMENT_NOT_FOUND, STALE_REF, TIMEOUT, INVALID_ARGS, NOT_IMPLEMENTED, INTERNAL,
USER_ACTIVE` (user held a button/modifier or moved the mouse — action aborted, ADR 0010),
`TARGET_OBSCURED` (the point is not inside the given window — nothing clicked, ADR 0010),
`TARGET_OCCUPIED` (a drag would drop onto another item such as a folder or the Recycle Bin — nothing dragged),
`STALE_REF` (the element of a ref is gone or its window closed — observe again, ADR 0015).
`NEEDS_HUMAN` also when the screen is locked or a UAC/secure prompt is showing (ADR 0025).
`WINDOW_BUSY` (P8-M1, ADR 0059): another agent session is acting in that window (it holds a
lease until 20 s after its last action there, or until it disconnects); the call waited in a
first-come queue for 10 s first. Reading tools are never blocked, and neither is the user.

**Health warnings (ADR 0050).** While the PC is under strain (RAM ≥ 90 %, CPU ≥ 95 % for 30 s,
a thermal zone ≥ 90 °C, the system drive almost full, ≥ 15 windows/tabs opened by the agent in
10 minutes, or one app grew by ≥ 40 processes since the session started), every result object
carries `health_warning` (text) and the MCP server adds a `⚠` line. At the critical level
(RAM ≥ 95 %, ≥ 98 °C, < 1 GB free, ≥ 30 opened, +100 processes), `launch_app`, `browser`
open_tab and `run_skill files.open` ask the user first; no → `BLOCKED_BY_POLICY`. Reading and
closing are never braked; XtraOS never kills processes.

## Perception

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `observe` | `window?` (default foreground), `scope_ref?` (one element's subtree, P2-M1), `full?`, `bounds?` (each line + `@cx,cy (x,y wxh)` screen place; full list), `max_elements?` (default 120), `reveal?` (P6-M5, ADR 0048: a minimized Chrome/Edge/Electron window with no page tree is shown for a moment with `SW_SHOWNOACTIVATE`, read, and minimized again; focus unchanged) | element lines with refs, or a diff (`+`/`-`/`~`) on repeat; `opaque` flag. P6-M4 (ADR 0047): a dialog of the window (in-page `IsDialog` element such as a browser alert, or an owned `#32770`/modal window) is the line after the header, in the list and in a diff: `!! dialog <ref> "title" (blocking): "message" — buttons: e46 "OK"…`; result `dialog: {ref, title, text, buttons[{ref,name}], kind: in_page\|window, blocking}` or `null`; header ends `dialog of wN` when the window is itself a dialog. P6-M5: on a Chromium window with no `Document` yet, `observe`/`find`/`read_text` read again for ≤ 1.5 s (`waited_ms`); `page: "minimized"\|"not_exposed"\|null` and a footer note say why a page is missing; `revealed: true` when `reveal` acted | P1 (M4), P6-M4, P6-M5 |
| `screenshot` | `window?` or `region?` `{x,y,w,h}`, `scale?` (default 0.5, 0.1–1) | PNG + mapping `screen = origin + px/scale` | P1 (M4) |
| `read_text` | `window` or `ref`, `last_lines?` | text (UIA TextPattern / Value / Name); never password values. Verified 2026-11-10 (ADR 0052): classic consoles, Windows Terminal and the VS Code editor are readable; the VS Code integrated terminal exposes no text (run commands to read in Windows Terminal instead) | P1 (M4) |
| `find` | `query` (name/role/automation-id substring), `window?`, `deep?` (virtualised list rows via ItemContainerPattern; exact name fast, partial scan ≤ `timeout_ms`, default 3000, max 15000) | matching refs (≤ 20); off-screen rows marked `(offscreen)`, virtualised ones `(virtualized: realized when used)`; `click(ref)` on a row selects (never opens) | P1 (M4), deep P2 (M7) |
| `ocr` | `window?` or `region?` `{x,y,w,h}`, `query?`, `max_lines?` (80), `language?` | lines `"text" @x,y wxh` in screen pixels (click-ready); Windows built-in OCR, offline | P2 (M2) |

## Windows & apps

M3 (ADR 0012): a window is referenced as `w<handle>` (e.g. `w460650`) or the integer handle.
P6-M1 (ADR 0046): `window` also accepts text: part of a title or a process name (`"chrome"`),
case-insensitive, exact title first; one match is used, none is `WINDOW_NOT_FOUND`, several are
`INVALID_ARGS` listing the candidates. A ref whose handle Windows gave to another app is `STALE_REF`.

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `list_windows` | `filter?` (substring of title or process) | `w460650 chrome.exe "Google - Chrome" 0,0 1920x1040 focused` lines; `agent` = opened by the agent (P2-M4) | P1 (M3) |
| `focus_window` | `window`, `verify_ms?` (default 300, 0 = off, max 2000) | `focused` true/false after `verify_ms`; when another window took the front: `taken_by` {ref, process, title} (else `NEEDS_HUMAN`) | P1 (M3); verify P6-M2 |
| `launch_app` | `app` (name/path/AUMID or app-map service), `args?`, `elevated?`, `reuse?` (default true: same app+args still open from the agent → that window), `timeout_ms?` | ref of the new (or reused) window. P6-M7 (ADR 0051): the first `chrome://`, `edge://`, `brave://`, `vivaldi://`, `opera://` page in `args` (not `about:` — `about:blank` works on the command line) of a Chromium browser is typed into the address bar (Ctrl+L, text, Enter; borrows like `type`) → `opened_internal`, else `internal_error` | P1 (M3; reuse P2-M4; P6-M7) |
| `close_window` | `window`, `timeout_ms?` | ok (graceful WM_CLOSE; else `NEEDS_HUMAN`); a window the agent did not open asks the user first (P2-M4) | P1 (M3) |
| (CLI) | `xtraos-core call <tool> [json \| -] [--json] [--out f.png]` | the same tools from any shell / LLM agent without MCP; pairs with the running core (starts it if needed); same policy, dialogs, kill switch, log | P3 (ADR 0039) |
| `cleanup` | — | closes every still-open window the agent opened (only those); lists what is left | P2 (M4) |
| `audit_export` | `from?`, `to?` (YYYY-MM-DD, local, inclusive), `format?` csv\|json, `out?` | the action log for the range with seq/time/session/client/method/result/policy/hash; chain check + head hash in the result; never overwrites; protected paths refused | P4 (ADR 0042) |
| `sandbox` | `op`: status \| start (`network?`) \| call (`tool`, `args`, `timeout_ms?`) \| stop | Windows Sandbox desktop; a second XtraOS inside; host ↔ inside via numbered request files in `%USERPROFILE%\.xtraos-sandbox` (token, size limits, untrusted replies); `start` without the feature, or with a Sandbox the user opened → `NEEDS_HUMAN`; first start can take minutes (`start` again keeps waiting); inside, text via `set_value` and buttons via `click` ref (UIA), `type` needs the Sandbox window in front → `NEEDS_HUMAN` (ADR 0040) | P3 (M4) |
| `system` | `op`: health \| battery \| brightness (`value?` 0–100) \| wifi / bluetooth (`state?` on\|off) \| lock \| sleep | OS APIs, background; radio switches, lock and sleep are `system_settings` (ask); shutdown/restart/sign-out, firewall, antivirus, accounts, network config, registry are deliberately not offered. `health` (ADR 0050, read-only, no admin): `cpu_pct`, `ram` {used_gb, total_gb, pct}, `gpu_pct`, `temps` [{zone, c}] (firmware thermal zones; "not available" when none), `disks` [{drive, free_gb, pct_free}], `wifi` {signal_pct} (no network name), `battery`, `top` (5 apps using most memory {app, processes, mb}), `warnings` | P3, health ADR 0050 |
| `volume` | `op`: get \| set (`value` 0–100) \| change (`by` −100..100) \| mute \| unmute | Windows master volume of the default speakers via Core Audio, background; `{volume, muted, before}` | P3 |
| `touch` | `action`: tap \| long_press \| drag \| pinch \| swipe; `x`,`y`, `to_x`,`to_y`, `fingers` (1–4), `scale`, `distance`, `duration_ms`, `idle_ms` | real synthetic touch (InjectSyntheticPointerInput), polite; 3–4 finger swipes = `system_settings` (ask); never on XtraOS/admin windows | P3 (M3) |
| `pen` | `action`: tap (`x`,`y`) \| draw (`points` [[x,y,pressure?]…] ≤ 2000); `pressure` 0–1024 | real synthetic pen with pressure, path filled ≤ 8 px per frame | P3 (M3) |
| `record_session` | `op`: start (`name`, `report?`) \| stop (`overwrite?`) \| status \| discard \| note (`text`) | records successful actions as macro skill `skills/<name>.json` with locators (process + title, role + exact name, window-relative points); typed text → arguments; password targets never recorded; ≤ 50 steps / 30 min; replay = `run_skill`. Bug report (P7-M3, ADR 0053): `report: true` also writes `Documents\XtraOS reports\<name>-<time>\report.md` — every recordable action incl. FAILED ones (error code + message) with a screenshot of its window (a step without a window uses the recording's last window, never the foreground app), `note` observations, versions, apps, replay hint; `stop` returns `report` | P3 (M2), P7-M3 |
| `frame_diff` | `window?` \| `region?`, `wait_ms?` (≤ 60000), `min_pct?` (default 0.5), `reset?` | changed regions (screen px, ≤ 16, merged 16 px cells) + `changed_pct` since the previous call on the same window/region; first call = baseline; no pixels | P3 (M1) |
| `browser` | action: list_browsers \| list_tabs \| tab_info \| dom \| ax_tree \| wait_for \| click \| type \| press_key \| scroll \| navigate \| open_tab \| switch_browser; selector, text, key, url, tab_id, … | web pages through the XtraDevPilot bridge (loopback + pairing token); same policy as desktop input (messaging send asks, purchase/credential fields refused); scripts, cookies, storage, uploads not exposed; bridge/extension missing → `NEEDS_HUMAN` | P2 (M6) |
| `elevated_helper` | op: start \| stop \| status | start: core dialog (class `elevated`) then the Windows UAC prompt; afterwards tools on admin windows are forwarded to the helper (`via: "admin_helper"`, refs ≥ e50000000), each admin window needs the user's OK in the helper's own dialog; declined UAC → `NEEDS_HUMAN` | P2 (M5) |
| `window_bounds` | `window`, `x?,y?,w?,h?` (visible frame), `state?` (minimize/maximize/restore) | bounds + state | P1 (M3) |

Input, close and bounds on an elevated window return `ELEVATION_REQUIRED` (ADR 0012).

## Input

M2 (ADR 0010): targets are `x, y` physical screen pixels. M3: `window` = `w<handle>`, or `"desktop"` for the desktop icon view (not in `list_windows`; its icons are ListItems, moved with `drag` from_ref).
M4 (ADRs 0014, 0015): `ref` targets act **in the background** first (UI Automation pattern, no
cursor, no keys); results end with `mode=background` or `mode=input`. Real input waits until
the user is idle ≥ `idle_ms` (default 800, 0 = off) and puts the cursor back afterwards
(`restore_cursor`, default true; `mouse` defaults to false because gestures span calls);
all real-input tools accept `idle_ms`. `drag` ends can be refs: `from_ref`, `to_ref`.

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `click` | `x, y` or `ref`; `window?`; `button?` (left/right/middle), `clicks?` (1–3), `modifiers?` (ctrl/shift/alt/win), `borrow?` (P6-M6) | ok; a ref on a `Text`/`Image` without an action tries its parent (`via: parent_invoke`); a real click into a window covered at that point borrows it (`borrowed_ms`) | P1 (M2), P6-M6 |
| `type` | `ref` (the field, focused via UIA; P6-M6) or `window`; `text` (≤ 10,000 chars, unicode), `submit?`, `focus?` (bring up and leave it), `borrow?` (default true) | ok; returns once the app has the text (M6: `settled`); password field → `NEEDS_HUMAN`. Borrow (ADR 0049): a window not in front is brought up after the user has been idle 1.5 s (wait ≤ 30 s, else `USER_ACTIVE`), one burst, then the user's window is given back (`borrowed_ms`); user input during the burst → `USER_ACTIVE`; bursts > 3 s ask the user first (a yes lasts 10 min per window); `borrow:false` → old `NEEDS_HUMAN` | P1 (M2), P6-M6 |
| `press_keys` | `window`, `keys[]` e.g. `["Ctrl+S"]` (one chord may be a plain string, P6-M3), `focus?`, `borrow?` (default true) | ok; borrows like `type` | P1 (M2), P6-M6 |
| `scroll` | `x, y` or `ref`, `window?`, `dx?`, `dy?` (notches; dy > 0 scrolls down) | ok | P1 (M2; ref M4) |
| `drag` | `from_ref` or `from_x, from_y`; `to_ref` or `to_x, to_y`; `window?`, `to_window?`, `steps?`, `step_ms?`, `hold_ms?`, `drop_on_item?` | ok; ends early → cancelled (Esc), nothing dropped; a drop at x,y onto another item → `TARGET_OCCUPIED` unless `drop_on_item` (a `to_ref` target is intended) | P1 (M2; refs M4) |
| `mouse` | `action` (move/down/up/path), `x, y` or `points[[x,y],...]`, `button?`, `window?` | ok | P1 (M2) |
| `set_value` | `ref`, `value` (UIA ValuePattern, no typing; password field → `NEEDS_HUMAN`) | ok `mode=background`. P6-M6 (ADR 0049): on a drop-down, selects the option with that name (exact, then case-insensitive) and reads it back (`via: select_option`); no such option → `ELEMENT_NOT_FOUND` listing the options; a browser `<select>` is opened for ≈ 0.5 s after the user is idle 1.5 s. A browser text field adds `note` (rich-text editors may not notice; use `type {ref, text}`) | P1 (M4), P6-M6 |
| `clipboard` | `op` (get/set), `text?` | text | P1 (M2) |
| `batch` | `steps[]` (≤ 50) of `{tool, args}`: input, window and read tools, ocr, app_map, built-in `run_skill`; each step passes the policy; `stop_on_error?` (default true) | one line per step | P2 (M1; M3) |

## Flow control & human in the loop

M5 (ADR 0017): every OS action passes the core's policy engine first (`BLOCKED_BY_POLICY`;
`confirm` classes show a core dialog to the user). After the kill switch (Ctrl+Alt+Esc or tray
Stop) every call returns `STOPPED` until the user resumes from the tray; there is no resume tool.
The user can also pause one agent (tray → Agents, P8-M3): that client's calls — including one
already running — return `STOPPED` ("the user paused this agent") while other agents go on.

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `wait_for` | `delay_ms?` (0–60,000; P6-M7: waited first; alone = a plain wait → `met: ["delay"]`), `window?`, `window_title?`, `text_contains?`, `ref_gone?`, `window_gone?`, `idle_ms?`, `timeout_ms?` (default 10 s, max 120 s, counted after the delay); all given conditions at once | met conditions (+ window ref) / `TIMEOUT` | P2 (M1; was P1, not built in Phase 1); P6-M7 |
| `ask_human` | `message`, `kind` (confirm/choice/info/task/notify), `options?` (choice: 2–5), `timeout_ms?` (default 120 s) | answer (`yes`/`no`/option/`ok`); `task` → `request_id` at once; `notify` (P6-M7, ADR 0051): Windows notification from the tray icon, at once `{shown}`, at most 1 per 10 s (`throttled`) | P1 (M5); P6-M7 |
| `wait_for_human` | `request_id`, `timeout_ms?` | `done` / `denied` (else `TIMEOUT`) | P1 (M5) |
| `status` | — | running/stopped, paired client, kill hotkey, policy file (M5); `dialog`: the foreground window's owned dialog (or the foreground window when it is one) `{window, title, process, owner, blocking}` or `null` — Win32 dialogs only, in-page browser dialogs show in `observe` (P6-M4, ADR 0047); `health` {cpu_pct, ram_pct, warnings} (ADR 0050); `locks` [{window, title, client, session, idle_s}] — windows other agent sessions are acting in (P8-M1, ADR 0059); `profile`: what this client may do — `full`, `background_only`, `read_only`, `blocked`, optionally "(only in …)" (P8-M2, ADR 0060); `state` `paused` when the user paused only this agent, and `agents` [{session, client, connected, paused, last_action, idle_s, calls, refused, connected_s}] — agents connected now and ended in the last 10 min, method names only (P8-M3, ADR 0061); `team`: the organisation's signed team policy — `null` (none), `{active: true, key_id, hash}`, or `{active: false, broken}` (then acting calls → `BLOCKED_BY_POLICY`, reading still works; ADR 0073) | P1, P6-M4, ADR 0050, P8-M1, P8-M2, P8-M3 |

## Skills & memory (P2)

| Tool | Args | Returns |
|---|---|---|
| `list_skills` | `app?` (name prefix, e.g. `files`) | one line per skill: name(args) — description (P2-M3) |
| `run_skill` | `name`, `args` | result (one call, many steps); built-in skills + JSON macro skills; policy per action/step (P2-M3). Built-in `dev.check` (P7-M3, ADR 0052, read-only): `{window, expect: [{text} \| {no_text} \| {element, role?, value?, enabled?} \| {title} \| {changed: true, min_pct? (0.1)}], timeout_ms? (3000, ≤ 30000), screenshot_on_fail? (true)}` (`value`: the element's value contains it; `changed`: the window visibly changed during the check — a minimized window fails only that check, with the reason) → `{pass, checks: [{check, pass, detail}], screenshot?}` + `PASS`/`FAIL` lines; retried every 250 ms until all pass or timeout; a failure saves a PNG to `%TEMP%\xtraos-checks`. Housekeeping (P7-M2, ADR 0054): `files.duplicates {path, min_kb? (100), limit?}` and `files.largest {path, limit?}` read-only (byte-for-byte identical files; time-bounded walk, `complete` flag); `files.organize {path, by? type\|month, apply?}` and `files.bulk_rename {path, find, replace, pattern?, apply?}` return a preview (read-only) unless `apply: true`, which is a `file_change` (the user approves the plan) done in one undoable shell operation, never overwriting; unfinished downloads and files changed in the last 2 min are not moved. Office (P7-M1, ADR 0055, needs Office desktop): `office.excel_read {path, range, sheet?}` read-only (≤ 2,000 cells; `values` rows, numbers stay numbers); `office.word_read {path, max_chars? (20000)}` read-only (text as lines, paragraph and word counts); `office.ppt_read {path, notes?, max_chars? (20000)}` read-only (`slides` [{slide, title, text[], notes}], ADR 0062); `office.ppt_add_slide {path, title, bullets?[] (≤ 50), at?, save_as?}` is `file_change` (a title + bullets slide, title only without bullets; a missing .pptx path creates the deck; ADR 0062); `office.excel_write {path, range (top-left), values [[...]], sheet?, save_as?, autofit? (true; columns ≤ 60 wide), bold? ["A1:D1", …] (≤ 20); ADR 0069}`, `office.word_append {path, text, save_as?}` (new paragraphs at the end; a missing path creates a .docx; ADR 0056) and `office.export_pdf {path, out?}` are `file_change` (asked); each in its own hidden Office instance with macros forced off, alerts off, an empty password (a protected file → `NEEDS_HUMAN`); never overwrites; a file open elsewhere → `NEEDS_HUMAN`; 120 s limit. Local vision (ADR 0070; computed on the PC, compact text instead of a picture): `vision.colors {window \| region \| both, grid? "8x6" (≤ 24x16), min_pct? (1)}` → named colour shares + a letter grid map with legend; `vision.regions {…, colors? [names], min_area? (0.02 %), max? (40)}` → coloured shapes with screen box, centre, area % and `round`/`box`/`thin`/`blob`, background left out; `vision.find_image {…, image (PNG ≤ 512 px), threshold? (0.9), max? (5)}` → screen boxes + scores (a flat image is refused); `vision.snip {region (+ window), path}` (`file_change`, never overwrites) saves an area as PNG; `window` + `region` together = that area of the window's own capture (works while covered). Colours are named to the nearest of 22. `files.open {path, app?}` (ADR 0069): without `app` shows a file selected in its folder in Explorer (a folder opens); `app: true` opens a document in its default app and returns its window (`ref`) when one with the file's name appears within 15 s; programs, scripts, installers and shortcuts → `BLOCKED_BY_POLICY` (use `launch_app`). Everyday pack 2 (ADR 0068): `window.snap {window, to: left\|right\|top\|bottom\|top-left\|top-right\|bottom-left\|bottom-right\|center\|maximize}` (class `input`; the work area of the window's own monitor; background like `window_bounds`); `files.recent {days? (2, ≤ 30), limit? (20, ≤ 100), pattern?}` read-only (the user's real Desktop, Documents, Downloads, newest first; unfinished downloads and Office lock files skipped; `complete`); `files.read_text {path, max_chars? (20000)}` read-only (UTF-8, UTF-16 with BOM; binary → `INVALID_ARGS`; ≤ 5 MB; protected places refused); `disk.usage` read-only (fixed drives, LOW under 10 % or 5 GB free); `routine.pause {name}` / `routine.resume {name}` (a paused routine is kept but not run; resuming does not catch up missed runs or files). `screen.find {query, timeout_ms? (5000, ≤ 15000), max_windows? (10)}` (ADR 0067, read-only): `find` on every listable window front first (not XtraOS's own or admin windows) within the time budget → per window with matches a header `wN process "title": n match(es)` and its first 5 elements with refs usable at once; `windows` [{window, matches}], `searched`, `skipped`, `complete`; pixel-only text needs `ocr`. Reminders (ADR 0066): `notify.show {text (≤ 250), title? (≤ 60, "XtraOS reminder")}` read-only, a Windows notification from the tray icon (waits for the 10 s spacing instead of dropping it); with `routine.add {…, once_at: "HH:MM" (today, or tomorrow if passed) \| "YYYY-MM-DD HH:MM" (≤ 1 year; past refused)}` it is a reminder: runs once, then the routine is removed; missed while XtraOS was off → runs once when it is back. Window layouts (ADR 0065): `layout.save {name, filter?}` (no prompt; XtraOS's data folder; visible top-level windows except XtraOS's own and admin windows; position/size of the visible frame and normal/maximized/minimized; ≤ 30 layouts × 40 windows), `layout.restore {name}` (class `input`; each saved window → an open window of the same process, same title first, else most " - " title parts in common; background move like `window_bounds`; windows not open are reported in `missing`, never launched), `layout.list`, `layout.delete {name}`. Routines (P7-M4, ADR 0063, in the core — nothing registered with Windows): `routine.add {name, skill, args?, every_minutes? (≥ 5) \| daily_at? "HH:MM" \| new_file_in? (folder) + pattern? (*), notify? failure\|always\|never}` (ADR 0064: `new_file_in` runs the skill once per new file in that folder — not subfolders — once it has not changed for 10 s, not unfinished downloads, ≤ 10 per 20 s tick, files already there never; `{file}`/`{name}` in `args` become its path/name; `notify` shows a Windows notification — `failure` (default) only when it starts failing or works again) always shows the user a dialog (no **Add routine** → `BLOCKED_BY_POLICY`; the scheduled skill is checked first: unknown skill, protected path, credential-looking keys, `routine.*` refused; ≤ 20); `routine.list`, `routine.summary {hours? (24)}` read-only; `routine.run {name}` and `routine.remove {name}` are `file_change`; each run is the agent `routine:<name>` through the normal policy, logged and pausable; a missed run runs once; a routines file changed outside XtraOS does not run until the user approves it. Task checkpoints (P7-M6, ADR 0057, no prompt — XtraOS's own data folder only): `task.save {id, state (JSON ≤ 32 KB), note?}`, `task.load {id}`, `task.list` (newest first; older than 30 days removed), `task.done {id}`; ≤ 200 checkpoints; a state with a credential-looking key (password, token, api_key, secret, otp, cvv, pin…) → `BLOCKED_BY_POLICY`. Skill sharing (P7-M5, ADR 0058): `skill.export {name, out?, include_defaults?}` (`file_change`, asked) writes `Documents\XtraOS skills\<name>.xtraos-skill.json` without typed-text defaults unless asked; `skill.import {path, name?}` checks the file (format 1, ≤ 256 KB, ≤ 50 steps, batch tools only, no nested macros, no existing name) and shows the user every step — only **Import** keeps it, else `BLOCKED_BY_POLICY`; imported skills still pass the policy per step |
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
