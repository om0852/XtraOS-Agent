# ADR 0070 — Local vision: describe pictures as compact text instead of sending them

- Status: accepted (owner, 2026-11-11: "process it without you and give you data in such format
  so less token is required").
- Date: 2026-11-11

## Context
An agent that checks a picture sends a screenshot to its model: about width × height ÷ 750
tokens (≈ 1,000 for a 1152 × 648 view, ≈ 2,225 measured for a full window). In this session
about ten such looks cost ≈ 8–9k tokens. Many checks need much less than a picture: which
colours are where, is there a round brown thing in the middle, where is this icon, what does
this box say. That can be computed on the PC and sent as a few lines of text.

## Decision
Built-in skills, all local (no network, no paid service), on the same capture as `screenshot`
(`window` — also when covered — or `region {x,y,w,h}`):
1. **`vision.colors {window|region, grid? "8x6" (≤ 24x16), min_pct? (1)}`** — the main colours
   as named colours with their share, and a grid map: one letter per cell (its main colour) with
   a legend. A picture becomes ~10 short lines.
2. **`vision.regions {window|region, colors? [names], min_area? (0.02 %), max? (40)}`** — blobs of
   one named colour: colour, screen box, centre, area %, and a shape hint (`round`, `box`,
   `thin`, `blob`). The most common colour is taken as the background and left out unless named.
   Work is done on a copy scaled to ≤ 480 px wide.
3. **`vision.find_image {window|region, image (PNG path), threshold? (0.9), max? (5)}`** — where a
   saved small picture (≤ 512 × 512) appears: screen box and match score, best first. Coarse
   normalised cross-correlation on a reduced copy, then refined at full size.
4. **`vision.snip {region, path}`** — saves a screen region as PNG (to make a template); class
   `file_change`, never overwrites, protected places refused.
5. Reading text in one area needs nothing new: `ocr {region}` already exists (Windows OCR).

Colours are named against a fixed palette of 22 everyday names (nearest colour, weighted RGB
distance). Results are approximations and say so; anything that truly needs judgement ("is this
drawing good?") still needs a vision model.

Also: `window` and `region` together analyse that area of the window's own capture, which
works while the window is covered (a screen `region` alone reads whatever is visible there).
`find_image` refuses a flat template (one colour), which would match anywhere.

## Results (2026-11-11, owner's PC, test build)
| Check | Result | Tokens | Screenshot instead |
|---|---|---|---|
| `vision.colors` Paint canvas (window covered) | shares + 16×9 map | ≈ 85 | ≈ 995 |
| `vision.regions` same | moon, suit, scarf, leaves with boxes | ≈ 241 | ≈ 995 |
| `vision.find_image` Paint's Fill icon | exact position, score 1.00, 0.8 s | ≈ 21 | ≈ 995 |
| `ocr {window, query}` VS Code chat line | the typed line | ≈ 31 | ≈ 1,004 |

First live run found three problems, fixed: covered windows read through a screen region
(added window + region), a flat snip matching everywhere (refused now), 5.6 s for a full
window (larger reduction factor: 0.8 s).

## Tests
- Unit: nearest colour names, grid letters and legend, connected regions and shape hints on
  synthetic images (a disc, a bar, a box), template match on a synthetic image (exact and
  shifted), argument checks.
- Live: the Paint drawing described with `vision.colors` / `vision.regions`, the VS Code chat
  box read with `ocr {region}`; characters returned compared with the screenshot token estimate.
