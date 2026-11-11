# ADR 0055 — P7-M1 Office pack, slice 1: Excel read/write and PDF export through Office automation

- Status: accepted (owner: "Keep building", 2026-11-11; plan row P7-M1).
- Date: 2026-11-11

## Context
Everyday Office jobs ("read the totals from report.xlsx", "fill this table", "make a PDF of the
contract") are slow and fragile through the UI (cells are canvas-drawn, dialogs differ by
version). Office desktop has its own automation interface (COM, `Excel.Application`, …), which
reads and writes the document model directly.

Spike, 2026-11-11 (owner's PC: Microsoft 365 desktop, Office 16; temp files only, nothing of the
owner's opened): an invisible `Excel.Application` created a workbook, saved it, reopened it
read-only and read `A1:B2` correctly; `ExportAsFixedFormat` wrote a PDF; Word did the same.
Cold start ≈ 9 s on this loaded PC, a read ≈ 0.8 s after that. From PowerShell one hidden Excel
outlived `Quit()` until its references were collected (PowerShell's reference counting).

## Decision
Built-in skills (no new MCP tools), each in **its own hidden Office instance** — never the
user's open Excel/Word — which is quit and released when the skill ends (a hang beyond the time
limit ends only that instance, identified by being the process XtraOS started):

1. **`office.excel_read {path, range, sheet?}`** — read-only (policy class observe). Opens the
   file read-only, returns the cells as rows (numbers stay numbers; dates as Excel serial
   numbers; errors as `#ERR`), at most 2,000 cells.
2. **`office.excel_write {path, range, values: [[...]], sheet?, save_as?}`** — a file change (the
   user approves; the text shows the file, sheet, range and the first values). `range` is the
   top-left cell; at most 1,000 cells. Saves the file (or `save_as`, never overwriting). If the
   file is open elsewhere (Excel opens it read-only) → `NEEDS_HUMAN` "close it first".
3. **`office.export_pdf {path, out?}`** — Word (`.docx .doc .rtf .odt`), Excel (`.xlsx .xls
   .xlsm .csv .ods`), PowerPoint (`.pptx .ppt .odp`) to PDF next to it (or `out`); never
   overwrites; class file change (a new file is created; asked).

Safety, for every file: paths through the protected-places check (ADR 0026/0050); **macros
forced off** (`AutomationSecurity = 3`, msoAutomationSecurityForceDisable) before opening;
alerts off; links not updated; an **empty password** is passed so a password-protected file
fails at once (`NEEDS_HUMAN`) instead of showing a password dialog — the agent never types
passwords. Outlook (drafts only, sending asks) and PowerPoint editing are later slices.

## Build findings (2026-11-11)
- **Optional COM arguments** must be passed as VT_ERROR / `DISP_E_PARAMNOTFOUND`; VT_EMPTY for
  `Workbooks.Open`'s `Format` made every open fail. Open failures now include Office's message.
- **Office lingers after `Quit`.** Even with every reference released, a hidden Excel often took
  5–10 s to exit on this busy PC, and one did not exit at all. Each call records the process id
  of the one instance it created (the single new `EXCEL.EXE`/`WINWORD.EXE`/`POWERPNT.EXE` right
  after `CoCreateInstance`; with more than one new process nothing is ever ended by force), the
  result returns at once, and a background thread ends that process if it is still there after
  10 s. Waiting in the call itself made the test take 86 s instead of 43 s.
- Excel's `IgnoreRemoteRequests` keeps files the user double-clicks out of the hidden instance,
  so ending it cannot close a document of the user's.
- Live: cold start 2–7 s per call; reading `A1:C3` back gave `Item Qty Price / Pens 12 1.5 /
  Paper 3 4.25` (numbers stay numbers); Excel → PDF and Word (RTF) → PDF worked; no Office
  process was left.

## Tests
- Unit: extension → app mapping, PDF target naming, cell value formatting.
- Interactive (Office installed): create a temp workbook through `office.excel_write` with
  `save_as` (policy bypassed in the test by calling the skill function directly), read it back
  with `office.excel_read`, export it and a temp Word document to PDF; no Excel/Word process left
  that XtraOS started.
