# ADR 0057 — P7-M6 Task checkpoints: save progress so another agent or session can resume

- Status: accepted (owner: "Keep building", 2026-11-11; plan row P7-M6).
- Date: 2026-11-11

## Context
XtraOS is meant to be used by any agent harness (Phase 5). Long desktop tasks ("process these
40 invoices", "fill the form for each row") outlive one agent session: the context runs out, a
harness restarts, or the user hands the task to another agent. Today the progress lives only in
the agent's memory. XtraOS already keeps state per user (data folder, ADR 0011); a small
checkpoint store there lets any client pick up where another stopped.

## Decision
Built-in skills (no new MCP tools), class observe (they write only XtraOS's own data folder,
never the user's files; no prompt):

- **`task.save {id, state, note?}`** — `state` is any JSON (≤ 32 KB), `note` a one-line summary
  (≤ 200 characters). Saved to `<data>\tasks\<id>.json` with the time; saving again replaces it
  and counts the saves. `id`: 1–64 letters, digits, `- _ .`.
- **`task.load {id}`** — the state, note, when it was saved and how many times.
- **`task.list`** — every checkpoint, newest first, with note and age; checkpoints older than 30
  days are removed when listing.
- **`task.done {id}`** — removes the checkpoint.

Limits and safety: at most 200 checkpoints (saving a new one beyond that is refused; finish or
remove some); **a state with a key that looks like a secret** (`password`, `passwd`, `secret`,
`token`, `api_key`/`apikey`, `otp`, `cvv`, `pin`, case-insensitive, at any depth) **is refused**
— checkpoints must never carry credentials; the folder is XtraOS's own data folder, which file
skills cannot read or change (ADR 0026).

## Tests
- Unit: id validation, secret-key detection (nested), size limit; save → load → list → done on a
  temp data folder.
