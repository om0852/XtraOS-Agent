# ADR 0067 — `screen.find`: find text in every open window

- Status: accepted (owner: "Keep building", 2026-11-11).
- Date: 2026-11-11

## Context
`find` needs a window. An agent asked "where is the Unsubscribe button" or "which window shows
invoice 4711" lists the windows and searches them one by one: many calls and tokens.

## Decision
Built-in skill `screen.find {query, timeout_ms? (5000, ≤ 15000), max_windows? (10)}`, class
observe. It runs the existing `find` (UI Automation names, roles, automation ids) on every
listable window, front first, skipping XtraOS's own windows and admin windows, until the time
budget is spent. A window that fails (closing, not responding) is skipped and counted. Result:
per window with matches, a header line (`w123 chrome.exe "title": N match(es)`) and its first
5 matching elements with refs usable at once (`click {ref}`), plus `searched`, `skipped` and
whether the budget ran out. Text drawn as pixels (games, canvas) is not found; the result says
to use `ocr` on a window then.

## Tests
- Unit: result formatting (per-window cap, headers, budget note).
- Interactive (`screen_e2e`): the testbed among the user's real windows (only read); its
  button found with a working ref (clicked in the background), an absent word → 0 matches.
- Measured on the owner's PC (4 windows: Chrome, Firefox, Claude, Notepad): 2.1 s cold,
  0.4 s warm.
