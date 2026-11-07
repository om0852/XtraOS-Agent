# ADR 0001 — Foundational decisions

- Status: accepted
- Date: 2026-11-07

## Context
XtraOS Agent must let any AI client operate a full desktop with low token cost, low latency,
and strong safety, building on lessons from XtraDevPilot (browser bridge).

## Decision
1. Product name **XtraOS Agent**; category: local AI action runtime (Agent–Computer Interface).
2. No LLM inside the product; the AI client is the brain.
3. Interface: MCP over stdio (`xtraos-mcp`, TypeScript).
4. Native work only in `xtraos-core` (Rust); IPC via user-ACL named pipe + pairing token.
5. Perception: UI Automation tree first, element refs, diffs; screenshots only for opaque UI.
6. Safety: policy engine, human-in-the-loop, kill switch, overlay, action log are core features.
7. Windows first; macOS in Phase 3, Linux in Phase 4.
8. Hard boundaries: no CAPTCHA solving, no detection evasion, no UAC/secure-desktop automation,
   no network control.

## Evidence
Design rationale in docs/02-architecture.md §3. OS-level assumptions are validated by
Phase 0 spikes S1–S8 (ADRs 0002–0009).

## Consequences
Two languages (Rust + TS) to maintain; clean separation lets the MCP layer evolve quickly while
the core stays stable and testable.
