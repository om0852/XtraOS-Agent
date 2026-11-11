# ADR 0062 — P7-M1 Office pack, slice 3: PowerPoint read and add slide

- Status: accepted (owner: "Keep building", 2026-11-11; plan row P7-M1).
- Date: 2026-11-11

## Context
Slices 1–2 (ADR 0055/0056) cover Excel, Word and PDF export (including .pptx). PowerPoint
reading and editing were missing. Outlook stays open (no classic Outlook mail profile here).

## Decision
Two built-in skills on the same machinery (`office.rs`: own STA thread, private hidden
instance, macros off, empty password so protected files fail, 120 s limit, our PID reaped):
- `office.ppt_read {path, notes?, max_chars?}` — Observe. Per slide: number, title, the text
  of every other text shape in order, and (with `notes: true`) the speaker notes. Output
  capped (default 20,000 characters).
- `office.ppt_add_slide {path, title, bullets?[], at?, save_as?}` — FileChange (asks the
  user). Adds a "Title and Content" slide (title only when there are no bullets) at the end
  or at position `at`; a path that does not exist creates the presentation. `save_as` writes
  a copy and never overwrites; a deck open elsewhere (read-only) is refused unless `save_as`
  is given. ≤ 50 bullets, ≤ 2,000 characters each.
- PowerPoint is never shown: presentations are opened and created without a window.

## Tests
- Unit: slide text formatting (paragraph `\r`, line break `\x0b`), argument checks.
- Interactive (`p7m1_e2e`): create a deck in a temp folder with two slides, read it back
  (titles, bullets, order, `at`), export it to PDF; refusal to overwrite on `save_as`.
