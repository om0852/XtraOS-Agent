# ADR 0068 — Everyday pack 2: snap, recent files, read text, disk usage, pause a routine

- Status: accepted (owner: "build 4 to 5 feature or improvement at once, then test", 2026-11-11).
- Date: 2026-11-11

## Context
Small, frequent needs that cost agents many calls or are impossible today:
- Arranging a window side by side needs monitor math (`window_bounds` takes pixels).
- "The file I worked on yesterday" needs a search over several folders.
- MCP-only clients (Claude Desktop, Cursor) cannot read a text file at all.
- Before a big download or clean-up, the agent cannot see free disk space.
- A routine can only be removed, not paused for a while.

## Decision
Built-in skills (no new tool):
1. **`window.snap {window, to}`** — `to`: `left`, `right`, `top`, `bottom`, `top-left`,
   `top-right`, `bottom-left`, `bottom-right`, `center` (60 % × 70 %), `maximize`. The area is
   the work area (without the taskbar) of the monitor the window is on. Class `input`; moved
   in the background like `window_bounds` (admin windows refused there).
2. **`files.recent {days? (2, ≤ 30), limit? (20, ≤ 100), pattern?}`** — files changed in the
   last days under the user's Desktop, Documents and Downloads (their real places, also when
   OneDrive moved them), newest first. Read-only; the same time-bounded walk as the
   housekeeping skills (hidden folders, `AppData`, `node_modules` and protected places skipped;
   `complete` flag).
3. **`files.read_text {path, max_chars? (20000, ≤ 200000)}`** — the text of a text file
   (UTF-8 with or without BOM, UTF-16 with BOM; others refused as binary). ≤ 5 MB. Read-only;
   protected places refused like every file skill (XtraOS's own files, credentials stores,
   system folders).
4. **`disk.usage`** — each fixed drive: total, free, used %; a drive with < 10 % or < 5 GB
   free is marked LOW. Read-only.
5. **`routine.pause {name}` / `routine.resume {name}`** — a paused routine stays but does not
   run (a new-file routine skips the files that arrive meanwhile). No prompt: pausing takes
   power away; resuming restores what the user approved. `routine.list` and the viewer show
   PAUSED.

## Tests
- Unit: snap rectangles (each position, odd sizes, monitor offsets), text decoding and binary
  refusal, disk LOW rule and formatting, pause state round trip and the scheduler skipping it.
- Interactive (one file): snap a testbed window left / right / maximize on its monitor;
  `files.recent` finds a file just written in Documents; `files.read_text` reads UTF-8 and
  UTF-16 files and refuses a binary; `disk.usage` lists C:; a paused routine is not run by a
  due tick and runs again after resume.
