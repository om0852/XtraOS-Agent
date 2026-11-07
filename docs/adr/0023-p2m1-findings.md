# ADR 0023 — P2-M1 Fewer calls: findings and gate result

- Status: accepted. **P2-M1 closed** 2026-11-07 (owner-confirmed hands-off gate run).
- Date: 2026-11-07

## What was built (ADR 0022)
- `wait_for` (perceive.rs): window_title (incl. across all windows), text_contains (visible
  tree + the window's main text, e.g. terminals), ref_gone, window_gone, idle_ms; polled in the
  core every 250 ms; `TIMEOUT` names the unmet conditions; stops on the kill switch.
- `observe(scope_ref)`: one cached fetch of an element's subtree; refs join the window table.
- `exec.rs`: one dispatcher for the pipe and for `batch` (≤ 50 steps, validated before anything
  runs; every step through `safety::authorize`; `batch:<tool>` action-log entries; the kill
  switch ends the batch even with `stop_on_error: false`).
- MCP: `wait_for`, `batch`, `observe.scope_ref` (22 tools). `read_text` window logic shared as
  `main_text()`. `ocr` is now the only planned-but-unbuilt method (P2-M2).
- Tests: unit test for batch validation; `tests/p2m1_e2e.rs` (7); flow R1; gate
  `tests/phase2/p2m1-gate.ps1`.

## Resolution (2026-11-07)
`tests/phase2/p2m1-gate.ps1 -Interactive`, owner hands-off: **13/13 PASS, 0 SKIP**
(Phase 1 regression, release build, install test 12/12, P2-M1 e2e 7/7, flows 6/6, bench).

| Test | Result |
|---|---|
| P2-WF-01 window_title across windows | met in 252 ms with the new window's ref |
| P2-WF-02 text after a background click | "Display is 7" met in 36 ms |
| P2-WF-03 ref_gone + window_gone | both met ~270 ms after the window quit |
| P2-WF-04 idle_ms 600 | met after ~820 ms |
| P2-WF-05 unmet text, timeout 1 s | `TIMEOUT` "not met after 1000 ms: text_contains" in 1.0–2.0 s |
| P2-OBS-01 scope_ref = list | ~67 vs ~107 tokens; only list items |
| P2-BA-01 Calculator batch | 6/6 steps in one call, 5 `batch:click` log lines |
| P2-BA-02 Send step | confirm dialog → Deny → step 2 `BLOCKED_BY_POLICY`, step 3 skipped, 0 sends |
| P2-BA-03 kill switch in a batch | step 1 `STOPPED`, step 2 skipped despite `stop_on_error: false`, button released |
| P2-BA-04 nested batch | `INVALID_ARGS`, nothing executed |
| **P2-BENCH-01 routine task (R1)** | **3 calls, ~335 result tokens** (budget ≤ 3 / ≤ 2,000); the same task in Phase 1 (F2): 8 calls |

Bench in this hands-off run (quiet machine): pipe 0.1 ms, observe 28.5 ms, click(ref) 5 ms,
screenshot 16.8 ms (all within budget), `type` 50 chars 287 ms until the app has the text
(app-bound, ADR 0020). Earlier misses of screenshot/observe were taken while the PC was in use.

## Findings
None blocking. Note: `wait_for(text_contains)` re-reads the tree every 250 ms; on very large
windows combine it with `scope_ref`-sized windows or `window_title` (event-driven UIA is a
later optimisation, ADR 0022).

## Consequences
Next: P2-M2 OCR (Windows.Media.Ocr) for opaque windows.
