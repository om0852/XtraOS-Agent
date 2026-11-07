# ADR 0038 — Tamper-evident action log + max actions per task

- Status: accepted, built and tested.
- Date: 2026-11-07

## Context
docs/05 promised two controls that were never built (found 2026-11-07):
- "log is append-only with a **hash chain**";
- "**max actions per task** (configurable)".

Only append-only files and a per-minute rate limit exist.

## Decision
### 1. Hash-chained action log
- Every log line gets `seq` (1, 2, …), `prev` (the previous line's hash) and `hash`.
  - `hash` = SHA-256 over the line's canonical JSON without `hash` (keys sorted, serde_json).
  - SHA-256 is implemented in the core (no new dependency) and tested against the FIPS
    180-2 test vectors.
- **Chained across files.** A new log file (one per core start) begins with `prev` = the
  last hash of the newest existing file, so deleting or editing an older file breaks the
  chain at the next one. The very first file starts from 64 zeros.
- **Checking:**
  - `xtraos-core.exe --verify-log` prints `OK n entries in m files` or the first break
    (file, line, reason) and exits 0 / 1;
  - the viewer shows the same result at the top of the activity list.
- **Honest limit.** Without a secret the user does not hold, malware running as the same
  user can rewrite the whole chain. The chain shows edits, deletions, reordering and
  truncation of the middle of the log. It does not prove that nobody rewrote all of it.
  Shipping log heads off the PC (audit export, Phase 4) is the stronger answer.
- Lines written before this change have no `hash`; the check reports them as `legacy` and
  starts the chain after them.

### 2. Max actions per task
- **What a task is.** The agent never says when a task starts or ends, so a *task* is a run
  of actions with no pause longer than `task_idle_minutes` (default 10).
- **The limit.** `max_actions_per_task` (default 2,000, range 5–1,000,000) counts every
  action the policy lets through: observe, input, skills, browser, everything that is
  authorised.
- **When the limit is reached, the core asks the user:**
  - "The agent has done N actions in this task without a break. Continue for another N?"
  - *Continue* resets the count.
  - *Stop* or no answer within 60 s → `BLOCKED_BY_POLICY` until the task pauses for
    `task_idle_minutes`, or until the user continues at the next prompt.
  - It is a runaway guard, in addition to the per-minute rate limit.
- Both settings live in `policy.json`, the rules editor and the viewer. As with any policy
  change, the core asks before applying it.

## Tests
- **Unit:** SHA-256 vectors; chain verify (OK, edited line, deleted line, reordered,
  truncated middle, legacy lines, broken file link); task limiter (count, idle reset,
  continue).
- **End-to-end:**
  - `--verify-log` on a real log, then after editing one character;
  - a policy with `max_actions_per_task: 5`: the 6th action asks; Stop blocks; Continue
    lets it go on.

## Results
- Unit: SHA-256 FIPS vectors (including one million 'a'); chain checks (OK across files;
  edit, deletion, reorder, removed file, legacy lines); task limiter.
- LG-01: `--verify-log` printed "OK: 3 entries…" (exit 0). After one character was
  changed it reported line 2, "content changed after it was written" (exit 1).
- TL-01: the 6th action asked; Stop refused, and kept refusing without new dialogs;
  Resume reset the count; Continue carried on.
- The existing IPC log test (no secrets in the log) still passes with the chained lines.
