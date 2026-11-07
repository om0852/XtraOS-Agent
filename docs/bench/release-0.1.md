# Bench — Release 0.1 (P1-REL-04)

Recorded 2026-11-07T12:17:59.576Z by tests/bench/bench.mjs: release `xtraos-core.exe` over its real named pipe, Intel(R) Core(TM) i5-8350U CPU @ 1.70GHz, 8 threads, Windows build 10.0.26200. Budgets from docs/06.

| Operation | p50 | p95 | Budget p50 / p95 | Result | Note |
|---|---|---|---|---|---|
| Pipe round trip (status) | 0.1 ms | 0.2 ms | 2 / 10 ms | met |  |
| observe Calculator (39 el., full) | 28.9 ms | 30.1 ms | 120 / 400 ms | met | ~254 tokens |
| click(ref) background (Calculator "Clear") | 5.5 ms | 9.5 ms | 80 / 200 ms | met |  |
| screenshot window @0.5 | 17.7 ms | 33.3 ms | 60 / 150 ms | met | 242x401 PNG 18 KB |
| ocr opaque window | 33.3 ms | 35.0 ms | 400 / 1000 ms | met | ~80 tokens, 8 lines |
| type 50 chars (unicode), until the app has it | 316.8 ms | 361.6 ms | 150 / 400 ms | **missed** | WinForms testbed field; read back exactly after every call |
| Kill switch → input halted | — | 15 ms | — / 100 ms | met | measured in the M5 gate (ADR 0018) |

Typed text read back exactly: yes.

## Deviations (ADR 0020)

- **type 50 chars**: since Release 0.1 `type` returns only when the app has really taken the text (before, it returned while keys were still queued, so a following observe could see half of it — found by this bench). The time is now app-bound: the WinForms testbed field processes ~13 ms per character (measured the same with and without pacing). The 150 ms budget assumed "keys sent"; it is kept as the target for fast fields and re-measured on more apps in Phase 2.
- **screenshot @0.5**: 52–84 ms p50 across runs on this PC (PrintWindow + downscale + PNG), around the 60 ms budget; p95 always within 150 ms.
- **observe** on windows with hundreds of list items (Explorer with 1,566 files: 170–222 ms p50) — ADR 0016, Phase 2 scoped observe.
