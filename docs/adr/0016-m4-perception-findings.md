# ADR 0016 — M4 Perception & background-first actions: findings and gate status

- Status: accepted. **M4 closed** 2026-11-07 (owner-confirmed hands-off gate run, see Resolution).
- Date: 2026-11-07

## What was built (ADR 0015)
- `core/xtraos-core/src/perceive.rs`: native UI Automation (COM, MTA pinned with
  `CoIncrementMTAUsage`, `IUIAutomation2` timeouts 2 s / 5 s), one `ElementFromHandleBuildCache`
  per observe, ref table with RuntimeId reuse + diffs, `observe`, `find`, `read_text`,
  `set_value`, background `click(ref)` / `scroll(ref)` ladder, foreground keep-back.
- `core/xtraos-core/src/capture.rs`: `PrintWindow(PW_RENDERFULLCONTENT)` / `BitBlt`, box-filter
  downscale, PNG (`png` 0.18.1), base64.
- `input.rs`: polite real input (`idle_ms`, `timeout_ms`, `restore_cursor`), `ref` targets for
  click/scroll, `from_ref`/`to_ref` for drag, `mode` in every result.
- MCP: 5 new tools (18 total); screenshots are MCP image blocks; per-call timeout follows
  `timeout_ms`. Testbed: AccessibleName per control, file drop, `--opaque`, `fillpassword`.
- Tests: 11 new unit tests; `tests/perception_e2e.rs` (11 interactive); MCP e2e for screenshot
  + error codes; gate `tests/phase1/m4-gate.ps1`.

## Findings (fixed during M4)
1. **Window chrome noise**: title-bar and scroll-bar parts ("Line up", "Minimize", …) cost
   tokens on every window. Their subtrees are no longer printed (window_bounds / close_window /
   scroll cover them); `find` still sees them.
2. **Elements without a RuntimeId** (WinForms list items via the MSAA proxy) got an identity
   from their position only, so after scrolling a ref pointed at a *different* item. Identity is
   now role + name hash + rectangle; if such an element moves its ref turns `STALE_REF`, never a
   wrong hit.
3. **Late focus steal**: Calculator sometimes activates itself *after* Invoke returns, after the
   synchronous check. A short watcher (≤ 400 ms, stops as soon as any newer action starts, so it
   never fights a deliberate `focus_window`) hands the foreground back.
4. **"Last input" changed during background runs**: the system-wide `GetLastInputInfo` time
   changed once per Calculator run (first click only, per-click instrumentation), **also in the
   owner-confirmed hands-off run**. The core's own injection counter (`input::last_inject_tick`)
   stayed untouched, so the core sent no input. Most likely Windows stamps the activation that
   Calculator triggers on itself (and our hand-back); not proven. Tests assert on the core's
   counter (definitive) and report the system-wide timestamp. Open: confirm the source in
   Phase 2 (does not affect the user: no cursor move, no key, no click reached any app).
5. **Virtualised lists**: rows scrolled out of view of a WinForms ListBox are not in the UIA
   tree at all, so "scroll Item 150 into view" is impossible without ItemContainer /
   VirtualizedItem patterns → backlog. `scroll(ref, dy)` on the list itself works in the background.

## Evidence so far (owner working at the PC meanwhile; debug build)
| Test | Result |
|---|---|
| bg01 Calculator 7 8 + 4 = by ref (P1-BG-01/03, P1-PER-02) | PASS — "Display is 82", all `mode=background`, cursor unchanged, core injected 0 events, foreground kept with the testbed; diff after the clicks ≈ 25 tokens (2 lines) |
| bg02 testbed buttons by ref, WinForms via native UIA (P1-BG-02, P1-IN-01) | PASS — counters 3/4/3, cursor unchanged |
| in08 / per07 set_value (Hindi + emoji) / password | PASS — value exact, 0 key events; password `= ***`, read_text `***`, set_value `NEEDS_HUMAN`, not findable |
| per03 opaque | PASS — owner-drawn window: 0 elements, `opaque`. Info: Claude desktop (Electron) gave 120 elements, not opaque (Chromium turns accessibility on when a UIA client asks) |
| per04 screenshot mapping | PASS — pad (551,185) mapped to (551.0,186.0), width exact; 371×256 PNG 10 KB |
| per05 stale ref | PASS — `STALE_REF` after close, re-observe works |
| bg06 scroll(ref) | PASS — list scrolled 10 lines via ScrollPattern, `mode=background`, no injection |
| per01/per06 observe budgets | tokens: median 256 (≤ 400) ✔, Explorer 1,059 ≤ cap 1,500 ✔. Latency p50: **Calculator 85–117 ms ✔ (≤ 120)**; Explorer on a 1,566-file folder 283–698 ms ✘; testbed with a 200-item MSAA list 200–1,040 ms ✘ |

## Decision on the latency budget
The 120 ms p50 budget holds for a typical native-UIA window (Calculator). It does **not** hold
for windows with hundreds of list items: the time is spent inside the provider (Explorer's
DirectUI, or the client-side MSAA proxy making one cross-process call per item), not in our
code — the walk itself is 9–25 ms. Tried and measured: fewer cached properties (no consistent
gain), no-reference element mode (Explorer ~290 vs ~470 ms, not enough, and actions would need
a re-find). Visible-only filtering inside UIA is kept (testbed walk 231 → 13 ms).
→ The gate asserts p50 ≤ 120 ms on Calculator and ≤ 1,000 ms (regression guard) on the large
lists; the large-list case moves to Phase 2 ("scoped observe": `scope_ref` subtree + per-window
incremental cache). This is a stated budget miss for large lists, not hidden.

## Resolution (2026-11-07)
Owner confirmed hands-off; `tests/phase1/m4-gate.ps1 -Interactive`:

| Step | Result |
|---|---|
| fmt / clippy -D warnings / 36 unit + 9 IPC / build / tsc / 8 MCP / testbed build | PASS |
| M2 input e2e under polite input (regression) | PASS 11/11 (in10 user-movement abort still works) |
| M3 window e2e (regression; UAC/browsers not opted in this run — passed in ADR 0013) | PASS (win01, win02) |
| M4 perception + background e2e | **PASS 11/11** incl. bg04 (waited ≥ 1.4 s while a simulated user moved, then `USER_ACTIVE`, nothing clicked; clicked once idle), bg05 (cursor back within 2 px; `restore_cursor:false` leaves it on target), in07 (file dragged from Explorer, located via `find`, dropped on the testbed: `file:xtraos-drop-test.txt`) |
| **Gate** | **10/10 PASS, 0 SKIP** |

Observe latency on a quiet machine (same bench right after): Calculator **p50 42 ms**
(39 elements, ~256 tokens), testbed 144 ms (17 elements, ~103 tokens; 200-item MSAA list),
Explorer on a 1,566-file folder 222 ms (120 elements, ~1,059 tokens). Median tokens 256.
The large-list budget miss above stands (Phase 2 backlog); the earlier 200–1,000 ms figures
were taken while the owner was working on the PC.
