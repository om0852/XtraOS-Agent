# ADR 0074 — Skills that say "asks the user" always ask

- Status: accepted (owner: "Yes, fix them", 2026-11-11)
- Date: 2026-11-11

## Context
During the demo recording (2026-11-11) `files.organize {apply: true}` moved 30 files without
an approval dialog, although its description says "apply asks the user". Its policy class is
`file_change`, which is allowed by default (ADR 0026), and nothing else asked. The same gap
existed for every `file_change` skill whose description promises a question:
`office.excel_write`, `office.word_append`, `office.ppt_add_slide`, `office.export_pdf`,
`files.organize`, `files.bulk_rename` and `skill.export`. An agent (and the user reading
`list_skills`) was told something the code did not do.

## Decision
1. `skills::ASKING_SKILLS` lists those skills; `plan()` sets `asks` for them (for
   `files.organize` / `files.bulk_rename` only with `apply`, a preview changes nothing).
2. In `safety::authorize`, after the user's policy and the team floor: an `asks` skill whose
   decision is `allow` becomes `confirm` (the usual approval dialog, with the plan as its text).
   A `deny` rule still wins; a user cannot silently turn the question off with `policy.json`.
3. Routines are exempt: the user approves a routine in its own dialog when it is added
   (`routine.add`), and its runs are unattended (client `routine:<name>`).
4. `file_change` itself stays `allow` (mkdir/copy/move/rename of single items, ADR 0026).

## Tests
- Unit `skills_that_say_they_ask_do_ask`: every built-in whose description says it asks the
  user either has a class that asks by default, shows its own dialog (`routine.add`), or is in
  `ASKING_SKILLS`. A new skill with a false promise fails this test.
- Unit `housekeeping_on_a_temp_folder`: an organize preview has `asks = false`, apply `true`.
- Live: see the P6-M9 note in the plan.
