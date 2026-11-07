# ADR 0024 — P2-M2 scope: `ocr` for opaque windows

- Status: accepted (scope). Findings and gate → ADR 0025.
- Date: 2026-11-07

## Context
Opaque windows (canvas apps, games, some Electron/custom-drawn UIs) expose no accessible
elements (`observe` → `opaque`). Today the agent can only take a screenshot: an image costs
far more tokens than text, and the model must estimate coordinates. Cost lever #4 (docs/06):
local OCR returns text, not images.

## Decision
1. **Windows.Media.Ocr** (built into Windows 10/11, free, offline, no new dependency besides
   enabling `windows` crate features `Media_Ocr`, `Graphics_Imaging`, `Storage_Streams`,
   `Security_Cryptography`, `Globalization`, `Foundation_Collections`). Engine from the user's
   profile languages, or `language` (installed BCP-47 tag). No OCR language installed →
   `NEEDS_HUMAN` with how to add one (Settings → Language → optional features); the core never
   installs anything.
2. **`ocr(window? | region?, query?, max_lines? = 80, language?)`**: captures like
   `screenshot` (a window via PrintWindow even when covered, or a screen region via BitBlt),
   at full resolution (downscaled only above the engine's max dimension), recognises, and
   returns compact lines with **screen** coordinates (physical px), ready for `click(x, y)`:
   `"Start Game" @412,318 180x38`. `query` (case-insensitive) keeps only matching lines and,
   when the match is part of a line, the box of the matching words. Result also carries
   `ms`, `language`, `lines`.
3. Same rules as `screenshot`/`observe`: policy class `observe` (deny-listed apps blocked), the
   agent's own XtraOS windows refused, the overlay is excluded from capture. OCR sees only what
   is drawn (password dots stay dots).
4. Budget (docs/06, new row): `ocr` of a window ≤ 400 ms p50; result ≤ 400 tokens median.

## Tests (docs/08, P2-M2)
P2-OCR-01..05 on an opaque testbed that draws text "buttons" by hand and reports their rects
and clicks over its control channel; plus a flow (F8-lite: opaque window → observe says opaque
→ ocr(query) → click the text) and the bench row.

## Consequences
docs/04 `ocr` row precise (P2-M2); docs/06 budget row; docs/03 F8 uses `ocr` before screenshot.
