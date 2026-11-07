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
ELEMENT_NOT_FOUND, STALE_REF, TIMEOUT, INVALID_ARGS, NOT_IMPLEMENTED, INTERNAL`

## Perception

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `observe` | `window?`, `scope_ref?`, `full?`, `max_elements?` (default 120) | compact tree or diff; `opaque` flag | P1 |
| `screenshot` | `window?`, `region?`, `scale?` (default 0.5), `grayscale?` | image + mapping info | P1 |
| `read_text` | `window` or `ref`, `last_lines?` | text (UIA TextPattern / Value) | P1 |
| `find` | `query` (name/role substring), `window?` | matching refs (≤ 20) | P1 |
| `ocr` | `window?`, `region?` | text boxes with coordinates | P2 |

## Windows & apps

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `list_windows` | `filter?` | `w1 chrome.exe "Google - Chrome" focused` lines | P1 |
| `focus_window` | `window` | ok | P1 |
| `launch_app` | `app` (name/path/AUMID), `args?`, `elevated?` | window ref when it appears | P1 |
| `close_window` | `window` | ok | P1 |
| `window_bounds` | `window`, `x?,y?,w?,h?`, `state?` (min/max/restore) | bounds | P1 |

## Input

| Tool | Args | Returns | Phase |
|---|---|---|---|
| `click` | `ref` or point; `button?`, `clicks?` (1–3), `modifiers?` | ok + diff | P1 |
| `type` | `ref?` (focus first), `text`, `submit?`, `mode?` (unicode/keys) | ok | P1 |
| `press_keys` | `keys[]` e.g. `["Ctrl+S"]`, `["Alt+Tab"]` | ok | P1 |
| `scroll` | `ref` or point, `dx?`, `dy?` | ok | P1 |
| `drag` | `from_ref`/point, `to_ref`/point, `steps?`, `hold_ms?` | ok | P1 |
| `mouse` | `action` (move/hover/down/up/path), `points?` | ok | P1 |
| `set_value` | `ref`, `value` (UIA ValuePattern, no typing) | ok | P1 |
| `clipboard` | `get` / `set(text)` | text | P1 |
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
