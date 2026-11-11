# ADR 0069 — Fixes from the D1 demo: `files.open {app}` and readable Excel output

- Status: accepted (owner: "Fix the 2 issues first", 2026-11-11).
- Date: 2026-11-11

## Context
The recorded demo D1 (docs/flows/D1.md) found two problems:
1. The agent called `files.open` to open a report; it only showed the file selected in
   Explorer. The name and description invited the wrong call.
2. `office.excel_write` wrote values only: dates showed as `########` in narrow columns,
   labels were cut, headers were plain.

## Decision
1. `files.open {path, app?}` — the description now says what it does first ("show a file
   selected in its folder in Explorer"). `app: true` opens a document in its default app
   (ShellExecute "open") and returns the app window when one with the file's name appears within
   15 s. Anything that runs code is refused with `BLOCKED_BY_POLICY` (programs, scripts,
   installers, shortcuts, `.url`, `.reg`, `.hta`, `.dll`, …): running a program stays the explicit
   `launch_app`. Class stays `launch` (the policy applies as before). Protected places are
   refused as before. Default behaviour (no `app`) is unchanged.
2. `office.excel_write {…, autofit? (true), bold? ["A1:D1", …] (≤ 20)}` — after writing, the
   used columns are auto-fitted, capped at 60 characters wide (long file names would otherwise
   make a column wider than the screen); `bold` ranges get a bold font. Range text is checked
   (letters, digits, `:` and `$` only).

## Results
- Unit: `runs_code` (11 code types refused, documents allowed), `format_args`.
- Live: the D1 report rewritten with `bold` and autofit — headers bold, dates readable, labels
  whole; `files.open {app: true}` opened it in Excel (2.1 s); a fake `.exe` refused. Autofit
  costs nothing measurable (1.6 s with, 1.8 s without; Excel start-up varies 1.6–24 s with
  PC load).
