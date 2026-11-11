# ADR 0058 — P7-M5 Skill sharing: export and import a recorded skill, with a review screen

- Status: accepted (owner: "Keep building", 2026-11-11; plan row P7-M5).
- Date: 2026-11-11

## Context
`record_session` (ADR 0036) saves a workflow as a macro skill in XtraOS's data folder
(`skills\<name>.json`), replayed by `run_skill` with every step passing the policy again.
A useful workflow should be shareable: one file to send to a teammate or another PC. An imported
skill is someone else's instructions, so the user must see what it does before it is kept.

## Decision
1. **`skill.export {name, out?, include_defaults?: false}`** — writes one file
   (`Documents\XtraOS skills\<name>.xtraos-skill.json`, or `out`; never overwrites) with a header
   (`xtraos_skill: 1`, name, description, exported date, XtraOS version), the steps and the
   arguments. **Typed text recorded as argument defaults is removed** unless
   `include_defaults: true` (it may be personal: an address, a message); the arguments become
   required. Class file change (asked; the text names the file and the step count).
2. **`skill.import {path, name?}`** — reads the file (≤ 256 KB, the shape above, ≤ 50 steps,
   every step's tool one a batch may run, `exec::BATCH_TOOLS`), refuses a name that exists, then
   shows the user a **review screen**: the skill's name, description and every step in plain words
   (`click Button "Save" in notepad.exe "…"`, `type ${text1}`, `run skill files.delete`). Only
   **Import** saves it to `skills\`; Cancel, closing or no answer saves nothing. The prompt is the
   standard XtraOS confirmation (safe answer default, real input only, 0.8 s arming).
3. An imported skill gets no extra trust: replaying it passes the policy step by step, like any
   skill (deletes, sends, purchases still ask).

## Tests
- Unit: export transform (defaults stripped / kept, header), import validation (bad version,
  unknown tool, too many steps, nested macros refused), step descriptions for the review.
- The review prompt is the existing confirmation dialog; the import's save path is tested through
  the approved branch.
