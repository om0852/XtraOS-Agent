# ADR 0014 — Non-interference: background-first actions, polite real input

- Status: proposed (owner decision pending; evidence from spike S10)
- Date: 2026-11-07

## Context
Owner request (after M3): while the agent moves the mouse or types, it interferes with the human
working on the same PC. Can the agent use a separate mouse/keyboard, a separate workspace or
window, and work in the background?

Root cause: real input (`SendInput`, M2) drives **the** system cursor and **the** keyboard focus.
A Windows session has exactly one of each; everything the agent injects competes with the user.

## Options evaluated

| # | Option | Verdict |
|---|---|---|
| 1 | Second physical/virtual mouse + keyboard | **No.** Windows merges all HID devices into one cursor and one focus per session; a second device moves the same cursor. |
| 2 | Windows virtual desktops (Win+Ctrl+D) | **No.** Same session and cursor; input only reaches the active virtual desktop. |
| 3 | Hidden Win32 desktop (`CreateDesktop`) | **No.** `SendInput` only reaches the input desktop; many GPU/UWP apps do not render there; it is the "hidden VNC" malware technique and breaks CLAUDE.md §4 (no hidden operation). |
| 4 | Second Windows session (RDP loopback / second user) | **No.** Client Windows allows one interactive session; workarounds break licence terms. |
| 5 | **Windows Agent Workspace** (Microsoft, preview) | **Later.** A separate Windows session with its own agent account, "allowing agents to interact with apps in parallel to your own session"; admin toggle (Settings > System > AI Components > Experimental agentic features); MCP "agent connectors" registered in the On-Device Registry. Exactly the separate workspace asked for, but preview, admin-only, rules still changing. → Backlog spike, re-evaluate when GA. |
| 6 | Windows Sandbox / Hyper-V VM (free on Pro) | **Later, optional.** Full isolation with its own cursor, but none of the user's apps, files or logins. → Backlog "sandbox mode". |
| 7 | **Background actions via UI Automation patterns** | **Yes (M4).** Act on the control itself: no cursor, no keystrokes. Spike S10 below. |
| 8 | **Polite real input** for what (7) cannot do | **Yes (M4).** Wait for the user to be idle, shortest burst, put cursor + foreground back. |

## Evidence — spike S10 (`spikes/s10-background-actions/run.ps1`, 4 runs, user working meanwhile)
Calculator driven only through UIA: Invoke `C, 7, 8, +, 4, =`, read the display, close via
`WindowPattern.Close`.

| Check | Result |
|---|---|
| Result | "Display is 82" — correct, every run |
| Cursor moved by the agent | **never** (4/4) |
| Keystrokes sent | none (no `SendInput` call at all) |
| 6 actions | 250–430 ms (managed client, uncached) |
| Close without clicking the X | works (`WindowPattern.Close`) |
| Foreground | **Calculator (XAML) activated its own window on the first Invoke** (4/4) |
| Mitigation: agent hands the foreground back | restored in ~20–25 ms (4/4) |
| WinForms testbed via the *managed* .NET UIA client | controls appear as bare `Pane` with no patterns → the core must use the **native** COM `IUIAutomation` (MSAA proxy → Invoke/Value); verify in M4 |

Not yet proven: that UIA Invoke generates no input event at all (`GetLastInputInfo` changed in
every run, but the user was active) — re-check in a hands-off run in M4.

## Decision (proposed)
An **action ladder**; every action result says which rung was used: `mode: background | input`.

1. **Background (default when a `ref` target supports it).** `click(ref)` → Invoke / Toggle /
   SelectionItem / ExpandCollapse; `set_value(ref)` → Value; `scroll(ref)` → ScrollItem / Scroll;
   `close_window` already sends no input (WM_CLOSE). If the provider activates its window
   (S10), the core restores the previous foreground window immediately.
2. **Polite real input** (canvas, games, drag between apps, opaque apps, explicit `x,y`):
   - wait until the user has been idle ≥ `idle_ms` (default 1,000 ms, `GetLastInputInfo`), up to
     `timeout_ms`, else `USER_ACTIVE` with hint "user is working; retry or ask_human";
   - one short burst; existing M2 guards still abort on user activity;
   - afterwards restore the cursor position and the previously focused window (`restore: true`
     default; off for flows that must leave focus in the target, e.g. typing into a chat).
3. Never: hidden desktops, input to a desktop the user cannot see (CLAUDE.md §4).

Cost/perf: background actions skip focus + move + arrival waits, so they are also faster and
need no screenshot to aim. Target ≤ 30 ms per pattern call with native UIA + CacheRequest.

## Consequences (on acceptance)
- docs/04: `click`/`scroll` gain `ref` + `mode` in results; `idle_ms`, `restore` on real-input
  tools. docs/08: new cases P1-BG-01..05 (Invoke without cursor move or key event, Value on
  WinForms + XAML + browser, foreground restore, idle-wait refusal, cursor/focus restore).
- M4 = perception (refs) + background actions, since actions need refs.
- docs/backlog.md: Agent Workspace spike (Phase 3, when GA); sandbox mode (Phase 3).
- Source: Microsoft support, "Experimental agentic features"
  (https://support.microsoft.com/en-us/windows/ai/ai-features/experimental-agentic-features).
