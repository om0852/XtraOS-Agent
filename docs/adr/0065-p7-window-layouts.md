# ADR 0065 — Window layouts: save and restore how windows are arranged

- Status: accepted (owner: "Keep building", 2026-11-11; Phase 7 everyday packs).
- Date: 2026-11-11

## Context
Daily work starts by arranging the same windows: editor left, browser right, terminal at the
bottom. Agents can already move windows one by one (`window_bounds`), but they cannot recall
an arrangement, and each restore costs many calls.

## Decision
Built-in skills (no new tool):
- `layout.save {name, filter?}` — records the visible top-level windows (process, title,
  position and size of the visible frame, normal / maximized / minimized). `filter` keeps only
  windows whose process or title contains it. Skipped: XtraOS's own windows, admin windows.
  Stored in XtraOS's data folder (`layouts\<name>.json`; ≤ 30 layouts, ≤ 40 windows each).
  No prompt (own data folder, like task checkpoints).
- `layout.restore {name}` — class `input` (moves the user's windows; the policy applies). Each
  saved window is matched to an open window of the same process: the same title first, else
  the most title parts in common (" - " separated), else the first unused one. A window is
  moved and sized like `window_bounds` (background, no focus change); maximized windows are
  moved to their monitor first, then maximized. Windows that are not open are reported, not
  launched (launching stays an explicit `launch_app`).
- `layout.list`, `layout.delete {name}`.

## Tests
- Unit: name checks, window matching (exact title, best overlap, first unused, no double use,
  process case), file round trip.
- Interactive: two testbed windows saved, moved and resized away, restored to within a few
  pixels; a minimized one comes back minimized; a missing window is reported.
