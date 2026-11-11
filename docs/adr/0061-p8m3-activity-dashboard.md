# ADR 0061 — P8-M3 Activity dashboard: who is acting, and pause per agent

- Status: accepted (owner: "Continue building", 2026-11-11; plan row P8-M3).
- Date: 2026-11-11

## Context
Several agents can use the PC at once (ADR 0059) with different rights (ADR 0060). The user can
stop everything (kill switch) but cannot see which agent is doing what, or stop only the one
that misbehaves while the others keep working.

## Decision
1. **Tracking.** The core keeps, per agent session: client name, connected or not, last action
   (method only, never arguments or typed text), when, number of calls and refused calls.
   Ended sessions stay listed 10 minutes (at most 20 entries). `status.agents` returns the list.
2. **Pause per agent.** The tray menu gets an **Agents** submenu: one item per connected agent
   ("Pause mcp:claude-code — last: click, 4 s ago, 12 calls"). Pausing is per client name, lasts
   until the user resumes it or the core restarts, and survives reconnects. While paused every
   call of that client — including one already running, at its next safety check — fails with
   `STOPPED` ("the user paused this agent; other agents keep working"); `status` still works and
   shows `state: "paused"`. Pause and resume come only from the tray menu itself, like Resume
   (ADR 0041): posted messages from other programs are ignored.
3. **Live view.** The "Rules and activity" window (ADR 0034) shows an "Agents now" section,
   read from the core over the pipe (it pairs as `xtraos-viewer`, which is not listed as an
   agent), refreshed every 3 s without losing the scroll position.
4. No new tool, no new error code: agents already treat `STOPPED` as "only the user can resume".

## Tests
- Unit: tracking (connect / act / disconnect / expiry / cap), pause by client name, tray labels.
- Interactive: two agents on one core; pausing agent A makes A's calls `STOPPED` and A's
  `status.state` `paused` while B keeps working; resuming restores A; `status.agents` lists both
  with their last method.
