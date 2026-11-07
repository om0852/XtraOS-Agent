# ADR 0025 — P2-M2 OCR: findings and gate result

- Status: accepted. **P2-M2 closed** 2026-11-07 (owner-confirmed hands-off gate run).
- Date: 2026-11-07

## What was built (ADR 0024)
- `ocr.rs`: Windows.Media.Ocr (WinRT, built in, offline) on the shared capture
  (`capture::grab`: window via PrintWindow even when covered, or a screen region); lines
  `"text" @cx,cy (x,y wxh)` in screen pixels; `query` → box of the matching word span;
  no OCR language → `NEEDS_HUMAN` with how to add one. MCP tool `ocr` (23 tools).
- Testbed `--opaque`: hand-drawn "Start Game" / "Options" / "Quit" / "Score 1234", hit-tested
  clicks, "Last click: …" painted, control channel (rects / state). Flow F8; bench row; gate
  `tests/phase2/p2m2-gate.ps1`.

## Resolution (2026-11-07)
`tests/phase2/p2m2-gate.ps1 -Interactive`, owner hands-off, screen unlocked:
**14/14 PASS, 0 SKIP** — P2-M2 OCR e2e 3/3, P2-M1 e2e 7/7, flows 7/7 (F2, F3, F4, F6, F10,
**F8**, R1), install test 12/12, bench.

| Test | Result |
|---|---|
| P2-OCR-01 read the opaque window | all 4 drawn texts (+ title); each centre inside its drawn rect, box within 8 px; `observe` → `opaque` |
| P2-OCR-02 click what OCR found | `ocr(query "start game")` → 1 line → real click at its centre → only "Start Game" counted |
| P2-OCR-03 region = window | same screen centres ± 2 px |
| P2-OCR-04 tokens | **~54 tokens vs ~2,225** for a screenshot of the same window (≈ 41× fewer) |
| P2-OCR-05 no match / bad args | empty match list ("-- 0 matching lines"); `INVALID_ARGS` |
| P2-OCR-06 latency | **p50 33 ms** (budget 400 ms) |
| F8 flow via MCP (release) | opaque → OCR finds "Start Game" → click → OCR reads "Last click: Start Game"; 6 calls |

## Findings
1. **Locked screen made everything fail with misleading errors.** One gate run happened while
   Windows had locked the PC: captures failed ("BitBlt: the handle is invalid"), every window
   looked covered (`TARGET_OBSCURED`), UI trees came back empty — 4 steps failed with seven
   different-looking errors. (An earlier run's single `ocr02` `TARGET_OBSCURED` fits the same
   moment.) **Fix:** before every OS action the core checks the input desktop; if it is not
   the user's `Default` desktop (locked screen, UAC prompt), the action returns
   `NEEDS_HUMAN` "the screen is locked or a secure prompt is showing; nothing was done". The
   full gate passed right after, unlocked.
2. OCR quality depends on what is drawn: clear UI text at normal sizes reads exactly; tiny or
   stylised text may not. The agent can fall back to `screenshot` (docs/03 F8).

## Consequences
docs/04 notes the locked-screen `NEEDS_HUMAN`. Next: P2-M3 app map + skills engine.
