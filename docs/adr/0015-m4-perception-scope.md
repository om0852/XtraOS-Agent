# ADR 0015 — M4 scope: perception, element refs, background-first actions

- Status: accepted (scope). Findings and gate result → ADR 0016.
- Date: 2026-11-07

## Context
M4 = Perception (docs/07): `observe`, `find`, `read_text`, `screenshot`, refs, diffs, opaque
detection. The owner accepted ADR 0014 (2026-11-07: "Yes, start M4"), so M4 also delivers the
background-first action ladder, because background actions need element refs. `set_value` and
P1-IN-07 were moved here by ADR 0010.

## Decision
1. **Native UI Automation via COM**, MTA per blocking worker (`CoInitializeEx` per call,
   idempotent). Spike S10 showed the *managed* .NET client sees WinForms controls as bare
   panes; the native client uses the MSAA proxy. One `ElementFromHandleBuildCache` call with a
   `CacheRequest` (TreeScope subtree, control-view filter, ~14 properties) fetches the whole
   tree in one cross-process round trip (ADR 0003: naive walks were 183–794 ms).
2. **Bindings: `windows = "=0.62.2"`** (Microsoft's official crate, free, MIT/Apache) for COM/UIA
   only. Reason: UIA interfaces have 50–100 vtable slots each; hand-declared slots that are off
   by one corrupt memory silently. Existing hand-declared Win32 FFI (input, winmgr) stays.
   **`png = "=0.18.1"`** (pure Rust) encodes screenshots.
3. **Refs** `e<n>`: a process-wide table maps ref → (window, UIA element, RuntimeId, role, name,
   bounds). A re-`observe` of the same window reuses refs of elements with the same RuntimeId,
   so the agent's refs stay valid and a **diff** is possible. A ref whose element is gone, or
   whose window has closed → `STALE_REF` (hint: observe again). The table is capped (4,096
   refs, oldest window evicted).
4. **`observe(window?, full?, max_elements? = 120)`**: header line + one line per kept element
   (`e5 Button "Seven"`; edits/values `= "text"`, toggles `[x]`/`[ ]`, disabled `(off)`).
   Kept: interactive control types, plus named text/headings. Offscreen elements dropped.
   Second call on the same window returns only the diff (`+`, `-`, `~` lines) unless `full`.
   `opaque: true` when a window larger than 200×200 px yields < 5 kept elements → hint to use
   `screenshot`. Password fields never return a value (`= ***`).
5. **`find(query, window?)`**: case-insensitive substring over name / role / automation id in
   the cached tree; ≤ 20 refs. **`read_text(window|ref, last_lines?)`**: TextPattern, else
   Value, else Name; passwords refused.
6. **`screenshot(window?, region?, scale? = 0.5)`**: `PrintWindow(PW_RENDERFULLCONTENT)`
   cropped to the visible frame for a window (works when covered), screen `BitBlt` for a
   region; box-filter downscale; PNG. Returns the mapping `screen = origin + px / scale`
   (ADR 0004: physical pixels throughout). Default target: foreground window, never the
   whole virtual desktop.
7. **Background-first actions (ADR 0014)**, result line says `mode=background|input`:
   - `click(ref)` (left, single, no modifiers): Invoke → Toggle → SelectionItem.Select →
     ExpandCollapse → LegacyIAccessible.DoDefaultAction; else real click at the element centre.
   - `set_value(ref, value)`: ValuePattern only; password field → `NEEDS_HUMAN`; read-only or
     no pattern → `INVALID_ARGS` (hint: click + type).
   - `scroll(ref, dy?)`: ScrollItem.ScrollIntoView, or Scroll on a container; else real wheel.
   - If a background action makes its provider take the foreground (S10: Calculator), the core
     gives the foreground back to the window that had it.
8. **Polite real input** (all M2 tools and real-input fallbacks): wait until the *user* has
   been idle ≥ `idle_ms` (default 800 ms; 0 disables) within `timeout_ms`, else `USER_ACTIVE`.
   The core's own injections update `GetLastInputInfo` too, so input that arrives inside the
   core's own injection window is not counted as the user. After mouse actions the cursor is
   put back where the user left it (`restore_cursor`, default true). Keyboard focus is **not**
   restored after a real click: the click is meant to move focus (click → type flows). The
   M2 guards (abort on user movement) remain.
9. **Moved out of M4:** none. P1-IN-07 (Explorer file → testbed drop) is in the gate; the
   testbed drop zone now also accepts files.

## Tests (docs/08)
P1-PER-01..07 as planned, P1-IN-01 via `click(ref)`, P1-IN-07, P1-IN-08 (`set_value`), plus
P1-BG-01..06 (no cursor move / no injected input for background clicks, foreground restore,
idle wait, cursor restore, background scroll).

## Consequences
docs/04 updated (ref targets, `mode`, `idle_ms`, `restore_cursor`, `STALE_REF`, observe format);
docs/08 adds P1-BG-*; docs/02 `perceive`/`refs` match this ADR.
