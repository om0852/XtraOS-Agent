# ADR 0051 — P6-M7 Small tools: a plain wait, a notification, internal browser pages

- Status: accepted (owner: "if you have any further improvement do that", 2026-11-10; plan row
  P6-M7 in docs/09-next-phases-plan.md).
- Date: 2026-11-10

## Context
From the session failures (docs/09 §2): the agent needed "wait 5 s, then check" but `wait_for`
demands a condition (§2.9); the agent had no way to get the user's attention without a blocking
dialog; typing `chrome://extensions` did not open it (§2.7).

Spike, 2026-11-10 (throw-away profiles):
- `chrome.exe chrome://extensions`, `chrome://settings`, `msedge.exe edge://extensions`,
  `edge://settings`: every one opened a **New Tab** — browsers ignore internal pages on the
  command line.
- `find {query: "Address"}` → `Edit "Address and search bar"`, then `type {ref, text:
  "chrome://extensions", submit: true}` (borrowed 670 ms) → title "Extensions - Google Chrome".
  The same run on Edge was stopped with `USER_ACTIVE` because the owner used the PC during the
  borrow: the P6-M6 interrupt worked live.

## Decision
No new tools (the core tool list has a size budget, ADR 0044); three additions to existing ones:

1. **`wait_for {delay_ms}`** (0–60,000): waits that long first (kill switch checked every
   100 ms). Alone it is the plain wait; with conditions they are checked after the delay.
2. **`ask_human {kind: "notify", message}`**: a Windows notification from the XtraOS tray icon
   (shown as a toast); returns at once (`{shown}`), no answer. At most one per 10 s per session
   (`shown: false, throttled: true` otherwise) so a looping agent cannot flood the user. The
   health guard (ADR 0050) also uses it for a critical warning, at most once per 10 minutes.
3. **`launch_app` with an internal page** (`chrome://`, `edge://`, `brave://`, `vivaldi://`,
   `opera://`; not `about:`, which works on the command line) for a Chromium browser: the browser opens without it, then XtraOS
   presses Ctrl+L (works in every UI language), types the address and Enter, borrowing the
   window like `type` (ADR 0049). Only the first internal page is opened; the result says
   `opened_internal`. The typing is part of the authorised `launch_app` (no second prompt).

## Tests
- Unit: delay argument range, internal-page detection, notification throttle.
- Interactive: `wait_for {delay_ms: 1500}` takes ≥ 1.5 s; `ask_human notify` returns at once
  and a second one within 10 s is throttled; `launch_app chrome chrome://extensions` (throw-away
  profile) ends on the Extensions page (browsers opt-in).
