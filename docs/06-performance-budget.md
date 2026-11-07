# 06 — Performance & Cost Budget

Performance and cost are the two product goals. Budgets are enforced by `bench/` in CI.

## Token budgets (agent-visible output)

| Output | Budget (median) | Hard cap |
|---|---|---|
| `observe` (focused window, first call) | ≤ 400 tokens | 1,500 (then `truncated`) |
| `observe` diff after an action | ≤ 80 tokens | 400 |
| `list_windows` | ≤ 150 tokens | 600 |
| Action result (`click`, `type`, ...) | ≤ 30 tokens | 120 |
| `screenshot` | scale 0.5 default, region-first | never full multi-monitor by default |
| Routine task via skill | ≤ 3 calls, ≤ 2,000 tokens total | — |

Token counts are estimated with a fixed tokenizer approximation (chars/4) in `bench/` and
reported per release.

## Latency budgets (mid-range PC, release build)

| Operation | p50 | p95 |
|---|---|---|
| Pipe round trip (no-op) | 2 ms | 10 ms |
| `click(ref)` incl. re-validation | 80 ms | 200 ms |
| `type` 50 chars (unicode mode) | 150 ms | 400 ms |
| `observe` focused window (≤120 elements) | 120 ms | 400 ms |
| `screenshot` window at 0.5 scale | 60 ms | 150 ms |
| Kill switch to input halted | — | 100 ms |

## Cost levers (in priority order)

1. **Skills** replace many AI round trips with one deterministic call.
2. **UIA tree + refs** instead of screenshots.
3. **Diffs** instead of full re-observation.
4. **Local OCR/detection** returns text, not images.
5. **Region-cropped, downscaled screenshots** only when the tree is opaque.
6. **Event-driven `wait_for`** instead of agent polling.
7. **Trusted `batch`** for known sequences.

## Measurement

`bench/` scenarios: Notepad note, Chrome search, Firefox search, drag file to upload zone,
Settings navigation. Each records: tool calls, tokens out, wall time, success.
