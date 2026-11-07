# ADR 0007 — Phase 0 closed; remaining spikes folded into Phase 1

- Status: accepted
- Date: 2026-11-07

## Context
Phase 0 planned spikes S1–S8. S1, S2, S3, S6 and an extra real-app spike S9 were completed.
The owner decided Phase 0 has proven enough to start Phase 1.

## Evidence — Phase 0 gate (`tests/phase0/run-tests.ps1 -Interactive`, 2026-11-07)

```
P0-01 PASS  UIA tree readable from outside apps        windows=4 maxTokens=389
P0-02 PASS  Capture->screen mapping on every monitor   monitors=1 mismatches=0
P0-03 PASS  Pipe pairing happy path, p50 < 2 ms        p50=0.470 ms
P0-04 PASS  Pipe rejects wrong token
P0-05 PASS  Pipe rejects missing token
P0-06 PASS  Pipe DACL: current user only + NETWORK denied
P0-07 PASS  Guarded clicks on own window               clicks=3 bad=0
P0-08 PASS  Real app: Calculator clicks = 15
P0-09 PASS  Real app: Calculator keyboard = 36
P0-10 PASS  Real app cleanup (Calculator closed)       CalculatorApp processes left=0
P0-11 SKIP  Multi-monitor accuracy                     monitors=1
Phase 0 gate: PASS=10 FAIL=0 SKIP=1
```

Bugs found and fixed during Phase 0: S1 "stray" metric counted earlier legitimate clicks
(now counts only clicks during the current action); S9 needed control polling (UWP lazy
content) and a foreground-lock-aware focus step (guard had correctly blocked a click into
Firefox); S9 log returned before cleanup.

## Decision
1. Phase 0 is **closed**. Current phase becomes **Phase 1 — Foundation (Windows)**.
2. Not-run spikes become Phase 1 test cases, each proven before its milestone closes:
   S4 drag-and-drop → P1-IN-07 (M2); S5 kill hotkey → P1-SAF-01 (M5);
   S7 elevated windows/UIPI → P1-WIN-07 (M3); S8 click-through overlay → P1-SAF-05 (M5).
3. Multi-monitor (P0-11) stays open as a known limitation until a second display is
   available; the tests already loop over all monitors.

## Known limitations carried into Phase 1
- UIA walk latency 183–794 ms (needs CacheRequest; budget 120 ms) — ADR 0003.
- Electron apps (Claude desktop) expose a sparse tree → opaque fallback — ADR 0003.
- Phase 1 requires Rust + MSVC Build Tools (free); install needs the owner's go-ahead (ADR 0002).
