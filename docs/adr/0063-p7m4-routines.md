# ADR 0063 — P7-M4 Routines: run a skill on a schedule, inside the core

- Status: accepted (owner: "build this all first in single run", 2026-11-11; plan row P7-M4).
- Date: 2026-11-11

## Context
The plan row named Task Scheduler. A scheduled task is a lasting change to Windows that runs
even when XtraOS is not, and it would need the owner's explicit OK (CLAUDE.md: confirm
persistent configuration). Everything a routine can do already exists as a skill.

## Decision
1. **In-core scheduler, no Task Scheduler.** Routines run only while the XtraOS core runs
   (it starts at login). Nothing is registered with Windows. A missed run (core off at that
   time) runs once when the core is back, never more than once.
2. **Skills** (`run_skill`):
   - `routine.add {name, skill, args?, every_minutes? (≥ 5) | daily_at? "HH:MM"}` — always shows
     the user a dialog (name, what the skill does, when); only **Add routine** keeps it. The
     skill and its arguments are checked like a normal call (unknown skill, protected path,
     credential-looking keys → refused). ≤ 20 routines; `routine.*` cannot be scheduled.
   - `routine.remove {name}` (no prompt: it only takes power away), `routine.list`,
     `routine.run {name}` (now), `routine.summary {hours? (24)}` — runs, failures and the last
     result per routine.
3. **Each run is a normal agent call**: client `routine:<name>`, session `routine-<name>`; it
   passes `safety::authorize` (kill switch, per-agent pause and profiles, policy, prompts) and
   is written to the action log. The dashboard lists it, so the user can pause it from the tray.
   While the kill switch is on, nothing runs.
4. **Storage**: `<data>\routines.json`, plus `routines.ok` = hash of the content the core wrote.
   If the file was changed outside XtraOS, its routines do not run until the user approves the
   listed routines in a dialog (like policy changes, ADR 0026). This keeps well-behaved agents
   from bypassing the dialog by editing the file; it is not a boundary against a malicious
   program running as the user. Run history: `<data>\routines-runs.jsonl` (≤ 2,000 lines; method
   names, result codes and ≤ 200-character result text — no typed text).
5. Times are the PC's local time.

## Tests
- Unit: schedule parsing, due logic (every / daily, catch-up once, created after today's slot),
  name and argument checks, run-log trimming, summary.
- Interactive (`p7m4_e2e`, private XtraOS home): the dialog only counts a real click, so the
  test stores the routine through the function `routine.add` calls after approval; then a due
  tick runs it once as `routine:<name>` through the real authorize path, `routine.run`,
  per-agent pause → `STOPPED`, summary counts, refusals before the dialog (protected path,
  `routine.*`), a tampered file is untrusted, remove.
