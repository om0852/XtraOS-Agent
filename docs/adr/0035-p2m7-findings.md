# ADR 0035 — P2-M7 Phase 2 leftovers: findings and gate result

- Status: accepted. The automated gate passed. Two measurements wait for the hands-off run:
  `type` into the Run box (bench) and the idle input census.
- Date: 2026-11-07

## What was built (ADR 0034)
1. **Off-screen list rows in `find`.** Rows with an empty rectangle are returned as
   `(offscreen)`. Example: testbed "Item 150" in a 200-row list, which used to give 0 matches.
2. **`find {deep: true, timeout_ms?}`** for virtualised lists:
   - ItemContainerPattern lookup by exact name, returned at once. Explorer System32,
     `zipfldr.dll`: 333–729 ms.
   - Otherwise a bounded walk comparing names. "kernel32" was found after 2,375–3,961 rows
     in the default 3 s.
   - The result says how many rows were scanned and whether the budget ran out.
   - Placeholders are realised only when an action uses the ref, so `find` never scrolls
     the user's list.
3. **The policy recognises Send / Buy / Delete in more languages:** Hindi (Devanagari and
   romanised), Marathi, Spanish, French, German and Portuguese.
   - Devanagari vowel signs are now kept as part of the word. Before, they were replaced
     by spaces, so no Hindi word could ever match.
   - Exact names only, as in English. "भेजें गए संदेश" (the sent-items folder) does not
     count as Send.
4. **Read-only viewer:** `xtraos-core.exe --viewer`, also tray → "Rules and activity…".
   - It shows every class's decision, the app rules and the rate limit.
   - It lists the newest 200 log entries (time, method, result, policy, client).
   - Buttons: Refresh, Open policy.json, Open log folder. It never writes.
5. **Input census** (`safety::input_census`): hook counters by source; counts only.
6. **Bench:** a `type` 50 chars row on a classic Win32 edit (the Run box), next to the
   WinForms testbed row.
7. **Flows runner:** it runs `cleanup` after every flow.

## Findings
1. **`click(ref)` on a list row opened the item instead of selecting it** (existing bug,
   found by P2-LO-02). Background-first click tried Invoke first, and on an Explorer file row
   Invoke means *open* (a double-click). A single click selects. Fixed: on
   ListItem/DataItem/TreeItem, SelectionItem.Select comes first. In the test, invoking a DLL
   happened to open nothing.
2. **Fetching the window's parts in parallel is not faster.**
   - Measured interleaved in one process: Explorer System32 214 ms serial vs 230 ms
     parallel; testbed 403 vs 460; identical output.
   - The apps' UIA providers answer one request at a time.
   - Reverted. The single serial fetch stays.
   - The large-list budget miss (Explorer System32 167–328 ms p50 depending on load, budget
     120) remains a provider-bound limit. `scope_ref` (P2-M1) and `find` are the fast paths
     for big windows.
3. **The "unexplained input during hands-off runs" is the physical mouse.**
   - A 30 s census on this PC: 0 events injected by other programs; 157 sub-6 px moves
     (sensor jitter, already ignored); 56 larger moves and 4 button/wheel events. The PC was
     in use during that window.
   - No software is injecting input.
   - A clean idle census runs in the hands-off gate (P2-LO-07).
4. **Test windows leaked.**
   - Two Explorer windows on `xtraos-flows-*` temp folders stayed open from earlier
     recorded-flow runs, plus one from a failed P2-LO-02 run.
   - They were closed as test leftovers; the user's own Explorer window was not touched.
   - Fixed: the flows runner calls `cleanup` after each flow. The P2-M7 tests already end
     with `cleanup`.

## Results
- Unit tests: 68.
- P2-LO-01, 02, 03, 05 automated: 4/4.
- Gate `tests/phase2/p2m7-gate.ps1 -Automated`: **13/13 PASS**, including the install test
  (29 tools) and the P2-M5/M6 automated suites.

## Open
- Hands off: the bench Run-box typing row and the P2-LO-07 idle census.
- Code signing stays an owner decision.
