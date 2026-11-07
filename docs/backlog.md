# Backlog (future-phase ideas — do not implement until their phase)

- macOS platform — Phase 3
- Linux platform — Phase 4
- Skill registry / marketplace — Phase 4
- Team policies and audit export — Phase 4
- Windows Agent Workspace spike: run the core inside Microsoft's separate agent session/account
  so the agent works in parallel with the user (preview in 2025–26; re-evaluate when GA) — ADR 0014, Phase 3
- Sandbox mode: run tasks inside Windows Sandbox / a Hyper-V VM (own cursor, no user apps) — ADR 0014, Phase 3
- Code signing for releases: choose a licence + apply to SignPath Foundation, or an owner certificate (`build-release.ps1 -CertThumbprint`) — ADR 0019/0020
- Fresh-machine install test incl. Smart App Control on (Windows Sandbox/VM) — ADR 0020
- Flow F6 with real Claude Code once installed on the test PC — ADR 0020
- `type` latency on more apps: bench row for the Run box added (ADR 0035); record it in the next hands-off bench run
- Faster observe on windows with thousands of items: provider-bound (ADR 0035 f2); revisit with UIA event-driven caching

## Done (moved out)
Scoped observe (P2-M1), virtualised/off-screen list rows, localised Send/Buy/Delete words, read-only rules + activity viewer, input-source census (P2-M7, ADR 0035). Phase 3: rules editor, frame diff, session recording, touch/pen (ADR 0037).
