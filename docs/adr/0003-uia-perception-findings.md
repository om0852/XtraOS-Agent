# ADR 0003 — UI Automation perception: findings from spike S2

- Status: accepted (Firefox/Explorer/Electron), open (Chrome, Terminal, Notepad: not running at test time)
- Date: 2026-11-07

## Context
Architecture D1 says "accessibility tree first". S2 checks whether real apps expose enough
through UI Automation (control view, depth ≤ 12, keep interactive + named text, cap 120).

## Evidence (spike S2, Windows 11, 1920×1080 @ 150%)

| App | visited | kept | first walk | repeat walk | ~tokens (compact) |
|---|---|---|---|---|---|
| Firefox (no extension) | 237 | 34 | 794 ms | 453 ms | ~240–256 |
| Explorer (folder window) | 92 | 45 | 362 ms | 183 ms | ~389 |
| Explorer (shell/taskbar) | 3 | 0 | 156 ms | 45 ms | 0 |
| Claude desktop (Electron) | 15 | 3 | 72 ms | 48 ms | ~14 |

Chrome was not running during the test (0 processes), so it is unverified.

## Decision
1. Keep UIA-first perception; token budget (≤ 400 median) is met by the compact format.
2. **Latency budget not met by naive walks** (183–794 ms vs 120 ms target). The core must use
   UIA `CacheRequest` (batch-fetch Name/ControlType/BoundingRectangle/IsOffscreen per subtree)
   and incremental diffs. Re-measure in Rust before closing M4.
3. **Electron/Chromium apps may expose almost nothing** until accessibility is enabled.
   Plan: detect "opaque" (few elements for a large window) → return `opaque: true` and fall
   back to screenshot/OCR; investigate enabling Chromium accessibility for desktop apps in Phase 2.

## Consequences
Contract keeps `opaque` flag and `screenshot`/`ocr` fallbacks as first-class. Bench must
include an Electron app and Chrome.
