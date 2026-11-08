# ADR 0043 — Findings from the desktop demo (2026-11-08)

- Status: accepted, built.
- Date: 2026-11-08

## Context
XtraOS drove a recorded demo on the owner's PC: arrange 45 desktop icons into a circle, open
GitHub and LinkedIn side by side, find the newest resume in File Explorer. Everything went
through XtraOS. What was missing or went wrong is fixed here, within the existing scope.

## Decisions

1. **`window: "desktop"`** (commit b1d78b1): the desktop icon view (Progman, or the WorkerW
   holding SHELLDLL_DefView) has no Alt+Tab entry, so observe could not reach the icons.
2. **`observe {bounds: true}`** (90e63ea): each line also gets `@cx,cy (x,y wxh)`, the same
   format as ocr. Needed to plan drops onto empty spots; without positions, the plan had to
   assume a layout, and the owner had rearranged the icons in the meantime.
3. **A drag that ends early cancels** (90e63ea): Esc before the button-up. Seen live: the
   user moved the mouse, the drag aborted, and the release dropped a desktop shortcut into
   the user's browser.
4. **No drop onto another item** (`TARGET_OCCUPIED`): before the button goes down, `drag`
   checks the element under a drop point given as x,y. On a ListItem/DataItem/TreeItem other
   than the dragged one, it refuses: a file dropped on a folder moves into it, on the Recycle
   Bin it is deleted. `to_ref` names the target on purpose; `drop_on_item: true` allows it for
   a point. The demo only stayed safe because the agent planned every drop by hand.
5. **`focus: true` on `type` / `press_keys`**: brings the window to the front first. The
   demo's keyboard steps were refused while the agent's own window was in front; the
   foreground is still re-checked before every chunk/chord.
6. **`files.list` shows modified dates and `sort: "newest"`; new `files.find(path, pattern,
   limit?)`**: breadth-first (Desktop/Downloads before deep project trees), newest first;
   skips protected and hidden folders, links, dot-folders, `node_modules` and `AppData` (not
   always marked hidden; its Recent folder filled the first results with `.lnk` shortcuts);
   5 s budget. Live on the owner's profile: 67,915 items in 3.9 s, complete, newest resume
   first. "Find my latest resume" is one call without the GUI.

## Not a bug
The CLI rejected JSON with Windows paths. A byte dump showed the JSON reached it with single
backslashes (`"C:\Users"`, the invalid escape `\U`): the caller's quoting had collapsed them.
XtraOS was right to refuse. Guessing is unsafe (`"C:\new"` is a valid escape for a newline),
so the error now says what is wrong and how to write the path (doubled backslashes or `/`).

## Tests
- DG-01 (p2m7, automated): a drop at x,y onto a testbed list row → `TARGET_OCCUPIED`, nothing
  dragged.
- IN-09b (input, interactive): a minimised testbed, `type` without focus → `NEEDS_HUMAN`;
  with `focus: true` it comes to the front and the text arrives; same for `press_keys`.
- Unit: `files.find` finds a nested file, sorts newest first, skips a hidden folder.
