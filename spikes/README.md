# Phase 0 spikes

Throwaway experiments that prove Windows behaviour before we build on it. Results go into
ADRs (`docs/adr/`); the code here is not product code.

## Tooling (see ADR 0002)

Spikes use **C# compiled at run time by Windows PowerShell 5.1** (`Add-Type`, .NET Framework,
both built into Windows) and the already-installed **Node.js**. No installs, no paid services,
no network access.

## Safety rules for every spike (mandatory)

1. **No installs, no admin, no registry/system-settings writes, no network.** Spikes run as the
   normal user and only write to `spikes/out/` (git-ignored).
2. **Read-only by default.** Spikes that only read (UI tree, screenshots of our own window,
   pipes) can run any time.
3. **Input spikes only touch windows the spike itself creates.** Before every injected event:
   - the window under the target point must belong to the spike (checked with `WindowFromPoint`),
   - no mouse button may be held by the user,
   - the cursor must still be where the spike last put it (if the user moved it, abort).
   Any failed check aborts immediately. The user's cursor position is restored at the end.
4. **Our test windows never steal focus** (`WS_EX_NOACTIVATE`), so typing in other apps is
   not interrupted.
5. **Privacy:** spikes that read other apps print only counts/timings to the console; any
   detailed dump stays in `spikes/out/` on this machine.
6. **Input spikes run only with an explicit `-Run` switch** and print what they will do first.

## Spikes

| ID | Folder | Touches | Status |
|---|---|---|---|
| S1 | `s1-input-guarded` | moves the cursor briefly over its own window | done (owner-approved run) → ADR 0005 |
| S2 | `s2-uia-tree` | reads UI Automation trees (read-only) | done → ADR 0003 |
| S3 | `s3-capture-mapping` | its own non-activating window only | done → ADR 0004 |
| S6 | `s6-pipe-pairing` | local named pipe only | done → ADR 0004 |
| S9 | `s9-real-app` | launches, operates and closes its own Windows Calculator | done (owner-approved) → ADR 0006 |
| S4, S5, S7, S8 | — | — | next round |
