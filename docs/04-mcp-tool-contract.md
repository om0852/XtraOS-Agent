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
`STALE_REF` (the element of a ref is gone or its window closed — observe again, ADR 0015)

## Perception

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `observe` | `window?` (default foreground), `full?`, `max_elements?` (default 120) | element lines with refs, or a diff (`+`/`-`/`~`) on repeat; `opaque` flag | P1 (M4) |
| `screenshot` | `window?` or `region?` `{x,y,w,h}`, `scale?` (default 0.5, 0.1–1) | PNG + mapping `screen = origin + px/scale` | P1 (M4) |
| `read_text` | `window` or `ref`, `last_lines?` | text (UIA TextPattern / Value / Name); never password values | P1 (M4) |
| `find` | `query` (name/role/automation-id substring), `window?` | matching refs (≤ 20) | P1 (M4) |
| `ocr` | `window?`, `region?` | text boxes with coordinates | P2 |

## Windows & apps

M3 (ADR 0012): a window is referenced as `w<handle>` (e.g. `w460650`) or the integer handle.

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `list_windows` | `filter?` (substring of title or process) | `w460650 chrome.exe "Google - Chrome" 0,0 1920x1040 focused` lines | P1 (M3) |
| `focus_window` | `window` | ok (else `NEEDS_HUMAN`) | P1 (M3) |
| `launch_app` | `app` (name/path/AUMID), `args?`, `elevated?`, `timeout_ms?` | ref of the new window | P1 (M3) |
| `close_window` | `window`, `timeout_ms?` | ok (graceful WM_CLOSE; else `NEEDS_HUMAN`) | P1 (M3) |
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
| `batch` | `steps[]` of the above (trusted input), `stop_on_error?` | per-step results | P2 |

## Flow control & human in the loop

M5 (ADR 0017): every OS action passes the core's policy engine first (`BLOCKED_BY_POLICY`;
`confirm` classes show a core dialog to the user). After the kill switch (Ctrl+Alt+Esc or tray
Stop) every call returns `STOPPED` until the user resumes from the tray; there is no resume tool.

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `wait_for` | `window?`, `ref_gone?`, `text_contains?`, `window_title?`, `idle_ms?` | ok / TIMEOUT | P1 |
| `ask_human` | `message`, `kind` (confirm/choice/info/task), `options?` (choice: 2–5), `timeout_ms?` (default 120 s) | answer (`yes`/`no`/option/`ok`); `task` → `request_id` at once | P1 (M5) |
| `wait_for_human` | `request_id`, `timeout_ms?` | `done` / `denied` (else `TIMEOUT`) | P1 (M5) |
| `status` | — | running/stopped, paired client, kill hotkey, policy file (M5) | P1 |

## Skills & memory (P2)

| Tool | Args | Returns |
|---|---|---|
| `list_skills` | `app?` | skill signatures |
| `run_skill` | `name`, `args` | result (one call, many steps) |
| `app_map` | `get(service)` / `set(service, surface, location)` | entry |

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
