# ADR 0066 — Reminders: `notify.show` and one-time routines

- Status: accepted (owner: "Keep building", 2026-11-11; Phase 7 everyday packs).
- Date: 2026-11-11

## Context
"Remind me at 5 pm to call the bank" is a common ask. Agents end their turn, so they cannot
wait until 5 pm. Routines (ADR 0063/0064) run on their own but only repeat, and no skill
shows a message.

## Decision
1. **`notify.show {text, title?}`** — a Windows notification from the XtraOS tray icon
   (ADR 0051). Class observe (it changes nothing). Text ≤ 250 characters, title ≤ 60. The
   10-second spacing between notifications stays; this skill waits for it (≤ 11 s) instead of
   dropping the message, so two reminders at the same minute both show.
2. **`once_at`** — a fourth kind of routine `when`: `"HH:MM"` (today, or tomorrow if that time
   has passed) or `"YYYY-MM-DD HH:MM"` (local time, ≤ 1 year ahead; a past date is refused).
   It runs once and the routine is then removed. If the core was off at that time, it runs
   once when the core is back (a late reminder beats a lost one); the summary keeps its run.
3. A reminder is `routine.add {name, skill: "notify.show", args: {text}, once_at}` — the user
   still approves it in the routine dialog. No separate reminder tool.

## Tests
- Unit: `once_at` parsing (today / tomorrow / date, past refused, > 1 year refused), due logic
  (before, at, late), removal after the run; `notify.show` argument checks.
- Interactive: a once routine with `notify.show` runs at its time through the scheduler, shows
  the notification, and is gone from `routine.list` afterwards; the summary has its run.
