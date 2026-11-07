# ADR 0034 — P2-M7 scope: Phase 2 leftovers (lists, speed, languages, viewer)

- Status: accepted (scope). Findings and gate → ADR 0035.
- Date: 2026-11-07

## Context
These backlog items are tagged Phase 2 and need no owner decision. Baseline on 2026-11-07
(release core, quiet PC):

| Measure | Result |
|---|---|
| Explorer on `C:\Windows\System32`, `observe` | 167 ms p50 (budget 120) |
| Explorer, `observe` when nothing changed | 211 ms p50 |
| `find kernel32` in that Explorer | **0 matches**: the row is off-screen and virtualised |
| testbed `find "item 150"` (200-item WinForms list) | **0 matches**: the row exists in UIA but has an empty rectangle, and `find` skips those |
| testbed `observe` | 100 ms p50 |

Code signing stays an owner decision (ADR 0019) and is not part of this milestone.

## Decision
1. **Off-screen list rows in `find`.** Rows (ListItem, DataItem, TreeItem) with an empty
   rectangle are returned, marked `(offscreen)`, instead of being skipped. Acting on such a
   ref uses the background patterns that already exist: `click` → select, `scroll` →
   ScrollIntoView.
2. **Virtualised rows: `find {query, deep: true}`.** In List / DataGrid / Tree containers
   that support ItemContainerPattern, the core looks the name up through the container:
   - exact name first;
   - then it walks the items in order, comparing names, within 3 s and 20,000 items.
   - Hits are marked `(virtualized)`. They are realised (VirtualizedItemPattern.Realize) only
     when an action uses the ref, so `find` itself never scrolls the user's list.
   - `deep` is opt-in because it can take seconds.
3. **Faster `observe` on big windows.** Measure where the time goes, then try fetching the
   window's top-level parts in parallel (UIA is free-threaded in the MTA). The change is kept
   only if it is faster and returns the same elements. Target: Explorer System32 ≤ 120 ms p50.
4. **Send / Buy / Delete words in more languages:** Hindi (Devanagari and common romanised
   forms), Marathi, Spanish, French, German and Portuguese. The words go into the same
   policy lists. Unit-tested.
5. **Policy and action-log viewer:** `xtraos-core.exe --viewer`, a read-only window.
   - It shows the effective policy (each class and its decision, app rules) and the last
     200 action-log entries.
   - It has an "Open policy.json" button. Changes still go through the approval dialog
     (ADR 0026).
   - It never writes the policy and never shows secrets. The action log has none.
6. **Typing latency on more apps:** measure `type` on Notepad and the Win+R box, in addition
   to the testbed, and record the numbers. A tooling task, not a product change.
7. **Stray input events during hands-off runs.** Record the low-level hook events during
   a 30 s idle window: injected flag, source process when known, deltas. Write down what
   they are.

## Tests (docs/08 P2-LO-*)
Automated, no human: testbed off-screen find + select; Explorer `deep` find + realise + select
(on an agent-opened Explorer window); observe speed bench; localised word unit tests; viewer
starts, renders the policy and log, and closes.

## Consequences
docs/04 (`find.deep`), docs/05 (localised words), docs/08, bench notes, backlog cleanup.
