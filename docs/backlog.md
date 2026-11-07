# Backlog (future-phase ideas — do not implement until their phase)

- Multi-finger trackpad gestures (three-finger swipe etc.) — Phase 3
- Touch/pen injection — Phase 3
- macOS platform — Phase 3
- Recording & replay of sessions — Phase 3
- Linux platform — Phase 4
- Skill registry / marketplace — Phase 4
- Team policies and audit export — Phase 4
- Windows Agent Workspace spike: run the core inside Microsoft's separate agent session/account
  so the agent works in parallel with the user (preview in 2025–26; re-evaluate when GA) — ADR 0014, Phase 3
- Sandbox mode: run tasks inside Windows Sandbox / a Hyper-V VM (own cursor, no user apps) — ADR 0014, Phase 3
- Scoped / incremental observe (`scope_ref` subtree, per-window cache) for windows with hundreds of list items (Explorer, big lists): M4 measured 283–698 ms p50 there vs 120 ms budget — ADR 0016, Phase 2
- Virtualised list items (ItemContainer / VirtualizedItem patterns) so `find` + `scroll(ref)` reach rows outside the viewport — ADR 0016, Phase 2
- Localised policy name heuristics (Send/Buy/Delete in other languages) — ADR 0018, Phase 2
- Tray policy editor + action-log viewer (separate app) — ADR 0017, Phase 2
- Find the source of system input events seen during hands-off runs (ADR 0016 f4, ADR 0018 f1) — Phase 2
