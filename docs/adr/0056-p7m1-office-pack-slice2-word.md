# ADR 0056 — P7-M1 Office pack, slice 2: Word read and append; Outlook stays planned

- Status: accepted (owner: "Keep building", 2026-11-11).
- Date: 2026-11-11

## Context
Slice 1 (ADR 0055) gave Excel read/write and PDF export through a private, hidden Office
instance. The plan row also lists Word insert and Outlook drafts.

Check, 2026-11-11, without starting Outlook: the owner's PC has **no classic Outlook mail
profile** (`HKCU\Software\Microsoft\Office\16.0\Outlook\Profiles` absent). Outlook automation
needs one; without it, starting `Outlook.Application` opens the profile wizard. Outlook also runs
as one shared instance (the user's own), unlike Excel/Word. An Outlook draft skill could not be
verified here, so it is not built (CLAUDE.md: no capability without verification).

## Decision
1. **`office.word_read {path, max_chars?: 20000}`** — read-only: the document's text (paragraphs
   as lines), its paragraph and word counts; same private hidden Word instance as slice 1, opened
   read-only, macros forced off, empty password.
2. **`office.word_append {path, text, save_as?}`** — a file change (asked; the approval text
   shows the file and the start of the text): opens (or, when `path` does not exist, creates) the
   document, adds `text` as new paragraph(s) at the end (newlines start new paragraphs) and saves
   (`.docx` for a new file; `save_as` never overwrites). Open elsewhere → `NEEDS_HUMAN`.
3. **Outlook drafts: planned.** When built, they must check for a mail profile before starting
   Outlook, never `Quit` an Outlook the user had open, and never send.

## Tests
- Unit: text normalisation (Word's `\r` / `\x07` / `\x0b` to lines).
- Interactive (Office): append to a new temp `.docx`, append again, read back both paragraphs;
  no Word process left.
