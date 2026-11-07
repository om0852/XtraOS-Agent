# ADR 0021 — Phase 2 plan (Smart & cheap): milestones and order

- Status: accepted (plan). Owner: "continue" after Phase 1 closed (2026-11-07).
- Date: 2026-11-07

## Context
docs/07 Phase 2: element-ref hardening, trusted `batch`, local OCR, app map (DPAPI), skills
engine + first skills (WhatsApp send file, Telegram send, Explorer file ops), elevated helper,
browser bridge to XtraDevPilot, MSI packaging. Exit: F5 and F7 pass; a routine task takes
≤ 3 calls / ≤ 2,000 tokens. Carried over: `wait_for` (contract P1, **never built in Phase 1 —
missed at the Phase 1 close; still `NOT_IMPLEMENTED`**), large-list observe (ADR 0016),
virtualised list items, localised policy names, `type` latency on more apps.

## Decision — milestones (each with its own scope ADR, tests, gate, like Phase 1)
| # | Milestone | Why this order | Exit |
|---|---|---|---|
| P2-M1 | **Fewer calls**: `wait_for` (finish the P1 contract), scoped `observe(scope_ref)`, trusted `batch` | Biggest cost lever with no new dependencies; removes agent polling loops | wait_for conditions + batch policy per step; F6-style task in ≤ 3 calls; bench |
| P2-M2 | **OCR** for opaque windows: `ocr(window/region)` via Windows.Media.Ocr (built in, free, offline) | Opaque apps (games, canvas, some Electron) become readable as text, not images | text + boxes mapped to screen; tokens ≪ screenshot |
| P2-M3 | **App map + skills engine**: DPAPI-encrypted local profile; declarative skills with Rust executors; first skill: Explorer file ops | One call per routine task (cost lever #1) | routine task ≤ 3 calls / ≤ 2,000 tokens |
| P2-M4 | **Messaging skills**: WhatsApp (desktop/web) send file, Telegram send — always through the M5 confirm dialog | F7 | F7 passes **in a test chat the owner chooses** (owner input needed) |
| P2-M5 | **Elevated helper** (opt-in, one UAC approval, only accepts the core's pipe) | F5 (admin terminal) | F5 passes; uninstall removes it |
| P2-M6 | **Browser bridge** to XtraDevPilot; packaging (MSI only if a free toolchain fits; signing per owner) | DOM precision where UIA is weak | bridge flow; install/uninstall |

Rules unchanged (CLAUDE.md): contract + ADR before code, no paid services, nothing outward
(messages, purchases) without the user's confirmation, gates with hands-off runs.

## Consequences
docs/07 gets the milestone table; docs/08 gets P2 test cases per milestone when it starts.
P2-M4 needs the owner to name a test chat/contact; P2-M6 packaging/signing need owner decisions.
