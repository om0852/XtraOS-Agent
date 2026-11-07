# ADR 0029 — P2-M4 Messaging + resource discipline: findings and gate result

- Status: accepted (built and tested). **P2-M4 closes after the full hands-off interactive
  gate run** (`tests/phase2/p2m4-gate.ps1 -Interactive`).
- Date: 2026-11-07

## What was built (ADR 0028)
- **Resource discipline** (`winmgr.rs`): the core keeps a list of the windows it opened.
  `list_windows` marks them `agent`. `launch_app` reuses a still-open window it opened with
  the same app + args (`reused`; `reuse: false` forces a new one). The new tool `cleanup`
  closes only agent windows and confirms a browser "Leave page" prompt only in those windows.
- **Policy**: class `close_user_window` (confirm). Closing a window the agent did not open
  always asks.
- **Approval dialog gives the foreground back** (`safety.rs`) after the user answers, so the
  approved key goes to the app it was meant for.
- **`whatsapp.send(phone, text?, file?, keep_open?)`** (`skills.rs`):
  - Class `message_send`, or `file_send` with a file. One dialog shows the number, a text
    preview and the file.
  - Opens WhatsApp Web through the app map (`whatsapp` → browser + URL), or reuses the user's
    open WhatsApp window and leaves it open.
  - Checks that the chat's compose label contains the number before sending.
  - Text: the `send?phone=&text=` URL fills the box; the skill checks it, then presses Enter.
  - File: puts the file on the clipboard as CF_HDROP, pastes it, waits for WhatsApp's preview,
    types the caption and presses Enter. The user's clipboard text is restored afterwards.
  - Checks that the send happened: the compose box is empty, or the file preview has closed.
  - Closes the window it opened, gives the foreground back, and reports only what was sent,
    never chat contents.
  - The kill switch stops the skill between steps.
- **Input helpers**: `clipboard_set_files` and `clipboard_restore` (CF_HDROP).
- **MCP**: `cleanup` tool (27 tools in total), `launch_app.reuse`, and the `agent` /
  `reused` markers.
- **Tests**: `p2m4_e2e.rs` P2-RES-01..04 and P2-WA-01..04. Real sends are opt-in through
  `XTRAOS_TEST_WHATSAPP` (the owner's own number, never stored in the repo).
- **Gate**: `tests/phase2/p2m4-gate.ps1`.

## Findings (fixed)
1. **The approval dialog stole the foreground**, so after "Allow" the approved Enter went
   nowhere. Fixed: the core focuses the previous foreground window again (never its own).
2. **The skill's window stayed open after a real send.** The first WA-01 run sent the message
   and then reported "window closed" anyway. The browser's "Leave page?" prompt appears late
   (while the sent message still syncs), after the single check had already run.
   - It did not show up with an unsent draft or with no chat open: those closed in 0.1–2 s.
   - Fixed: `cleanup` polls for up to 8 s and focuses only its own window, because Firefox
     exposes the page only in front. The skill now reports `window_closed` from what really
     happened.
   - The leftover window was closed by confirming its own prompt. The user's other Firefox
     window was not touched.
3. **The test exposed an untruthful result.** The result text claimed something that had not
   been checked. Now every claim in the result is verified first (rule kept for later skills).

## Results
- P2-M4 e2e 5/5:
  - RES-01: the second launch was reused (same ref); `reuse: false` opened a new window.
  - RES-02: agent windows marked `agent`, the user's window not.
  - RES-03: `cleanup` closed 2 agent windows with no dialog; the user's testbed stayed open.
  - RES-04: closing the user's window asked ("close_user_window"); after Deny it stayed open.
    Closing an agent window did not ask.
  - WA-03/04: a bad number was refused before any dialog. The dialog showed the number and
    text; after Deny nothing happened. With no app-map entry the skill stopped and opened
    nothing.
  - WA-01: text sent to the owner's own number in ~46 s, and the window it opened was closed.
  - WA-02: file and caption sent in ~54 s, the clipboard text restored, and the window closed.
- Non-interactive gate: **10/10 PASS** (fmt, clippy -D warnings, unit + IPC, MCP 10/10,
  release bundle, clean-profile install test 12/12 with 27 tools).

## Open
- Send time is 46–54 s, mostly WhatsApp Web loading in a new window. Reusing an already open
  WhatsApp window avoids the load.
- A contact saved under a name only (no number in the compose label) returns `NEEDS_HUMAN`
  on purpose. The skill never guesses the chat.
- Run the full interactive regression (P2-M1..M3, safety, flows, bench) hands-off, since the
  approval-foreground change touches every confirm dialog.
