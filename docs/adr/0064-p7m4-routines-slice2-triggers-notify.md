# ADR 0064 — P7-M4 Routines, slice 2: new-file trigger, notifications, viewer section

- Status: accepted (owner: "build moreee", 2026-11-11; plan row P7-M4, open item "triggers
  other than time").
- Date: 2026-11-11

## Context
Routines (ADR 0063) run only on a timer. Common wishes are "when a file lands in Downloads, do
X", and "tell me when my routine fails". The user also cannot see routines anywhere except by
asking an agent.

## Decision
1. **Trigger `new_file_in {folder, pattern? (*)}`** — a third kind of `when`, next to
   `every_minutes` and `daily_at`. The scheduler tick (20 s) looks at the folder (not its
   subfolders) for files created or changed after the routine's cursor. A file counts once it
   has not changed for 10 s, is not an unfinished download (`.crdownload`, `.part`, …) and
   matches the pattern. At most 10 files per tick, oldest first; each is one run. In the
   routine's arguments, `{file}` becomes the file's full path and `{name}` its file name.
   The cursor starts when the routine is added, so files already there are never processed.
   The folder must exist and pass the protected-places check. The add dialog shows the folder
   and pattern.
2. **`notify: "failure"` (default) | `"always"` | `"never"`** — a Windows notification from the
   XtraOS tray icon (ADR 0051). With `failure`, only when a routine starts failing and when it
   works again (no repeat while it keeps failing).
3. **Viewer.** "Rules and activity" shows a ROUTINES section: name, when, last run and result,
   read from the routines files.

Unchanged: adding asks the user; each run is the agent `routine:<name>` under the normal
rules; a routines file edited outside XtraOS waits for the user.

## Tests
- Unit: trigger parsing and validation, new-file selection (cursor, settle time, unfinished
  downloads, pattern, limit, oldest first), `{file}`/`{name}` substitution, notification
  decisions, viewer text.
- Interactive (`p7m4_e2e`): a new-file routine on a temp folder: an old file is ignored, a new
  file is processed once with `{file}` substituted, a `.crdownload` file is not.
