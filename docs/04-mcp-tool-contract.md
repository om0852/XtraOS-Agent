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
`TARGET_OBSCURED` (the point is not inside the given window — nothing clicked, ADR 0010)

## Perception

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `observe` | `window?`, `scope_ref?`, `full?`, `max_elements?` (default 120) | compact tree or diff; `opaque` flag | P1 |
| `screenshot` | `window?`, `region?`, `scale?` (default 0.5), `grayscale?` | image + mapping info | P1 |
| `read_text` | `window` or `ref`, `last_lines?` | text (UIA TextPattern / Value) | P1 |
| `find` | `query` (name/role substring), `window?` | matching refs (≤ 20) | P1 |
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

M2 (ADR 0010): targets are `x, y` physical screen pixels; `window` is a native window handle
(integer). `ref` targets are added in M4, window refs/titles in M3.

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `click` | `x, y` (M4: or `ref`); `window?`; `button?` (left/right/middle), `clicks?` (1–3), `modifiers?` (ctrl/shift/alt/win) | ok | P1 (M2) |
| `type` | `window` (required, must be foreground), `text` (≤ 10,000 chars, unicode), `submit?` | ok | P1 (M2) |
| `press_keys` | `window` (required, must be foreground), `keys[]` e.g. `["Ctrl+S"]` | ok | P1 (M2) |
| `scroll` | `x, y`, `window?`, `dx?`, `dy?` (notches; dy > 0 scrolls down) | ok | P1 (M2) |
| `drag` | `from_x, from_y`, `to_x, to_y`, `window?`, `to_window?`, `steps?`, `step_ms?`, `hold_ms?` | ok | P1 (M2) |
| `mouse` | `action` (move/down/up/path), `x, y` or `points[[x,y],...]`, `button?`, `window?` | ok | P1 (M2) |
| `set_value` | `ref`, `value` (UIA ValuePattern, no typing) | ok | P1 (M4) |
| `clipboard` | `op` (get/set), `text?` | text | P1 (M2) |
| `batch` | `steps[]` of the above (trusted input), `stop_on_error?` | per-step results | P2 |

## Flow control & human in the loop

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `wait_for` | `window?`, `ref_gone?`, `text_contains?`, `window_title?`, `idle_ms?` | ok / TIMEOUT | P1 |
| `ask_human` | `message`, `kind` (confirm/choice/info), `options?` | answer | P1 |
| `wait_for_human` | `request_id` | done / denied | P1 |
| `status` | — | running/stopped, paired client, policy profile | P1 |

## Skills & memory (P2)

| Tool | Args | Returns |
|---|---|---|
| `list_skills` | `app?` | skill signatures |
| `run_skill` | `name`, `args` | result (one call, many steps) |
| `app_map` | `get(service)` / `set(service, surface, location)` | entry |

## Output example (`observe`)

```text
w2 "Untitled - Notepad" notepad.exe focused 1280x720@m1
e1 MenuItem "File"
e2 MenuItem "Edit"
e5 Document "Text editor" = ""
e9 Button "Close"
-- 5 elements, ~60 tokens
```
