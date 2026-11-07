# ADR 0028 — P2-M4 scope: whatsapp.send skill + resource discipline

- Status: accepted (scope). Findings and gate → ADR 0029.
- Date: 2026-11-07

## Context
Owner (2026-11-07): build the WhatsApp skill; use real mouse/keyboard when an app needs it,
background otherwise. And: *"keep proper track of what you open; reuse instead of opening
twice or thrice; close what you opened when the task is done; never close the user's open
work"* — integrity, confidentiality, precautions, because the agent controls the whole PC.
Owner's setup: WhatsApp Web in Firefox; test target = the owner's own number (self chat).

Facts from the prototypes (send of "Hello" + a file preview, 2026-11-07):
- `web.whatsapp.com/send?phone=<digits>&text=<text>` opens the right chat with the text in
  the compose box (`Edit "Type a message to +91 …"`), reliably.
- Firefox exposes the page (UIA tree and capture) only while the window is in front.
- WhatsApp ignores UIA Invoke on its own buttons → real keys/clicks there.
- Pasting a file (CF_HDROP clipboard) into the chat opens WhatsApp's send preview.
- Closing a WhatsApp window raises Firefox's "Leave page" prompt.
- The approval dialog took the foreground, so the approved key went nowhere → fixed: the core
  gives the foreground back after the user answers.

## Decision
### A. Resource discipline (core, every tool)
1. The core **tracks every window it opens** (launch_app, skills). `list_windows` marks them
   `agent`.
2. **Reuse, don't duplicate**: `launch_app` with the same app + args as a still-open window the
   agent opened returns that window (`reused`) instead of starting another (`reuse: false` to
   force a new one).
3. **The user's windows are theirs**: `close_window` on a window the agent did not open is a new
   policy class `close_user_window` → **confirm dialog** (never silent).
4. New tool **`cleanup`**: closes every still-open window the agent opened (gracefully; for
   browser "Leave page" prompts it confirms only in those windows); never touches others;
   reports what it closed and what is left.
5. Skills close what they opened when done and give the foreground back to where the user was.

### B. `whatsapp.send(phone, text?, file?, keep_open?)` (built-in skill)
- `phone`: digits with country code (8–15; `+`, spaces, dashes ignored). `text` ≤ 4,000 chars;
  `file`: an existing file outside protected folders (ADR 0026). At least one of text/file.
- Policy: class `message_send` (and `file_send` with a file) → **one confirm dialog showing the
  number, a text preview and the file name**; nothing is sent without it.
- Surface from the app map (`whatsapp` → browser + URL). If a WhatsApp window is already open
  (the user's), it is **reused** (navigated to the chat; left open afterwards); otherwise one
  window is opened and **closed again after sending** (`keep_open` to keep it).
- Steps (inside the approved skill, real input where WhatsApp needs it, polite waits):
  open the `send?phone=` URL → wait for the compose box (handles "Use here"; a number not on
  WhatsApp → `INVALID_ARGS`) → **verify the chat label contains the number** (never send to the
  wrong chat) → text: Enter; file: paste via the clipboard (the user's clipboard text is
  restored), caption = text, Enter → **verify** the message/file appears in the chat.
- Confidentiality: the result reports only what was sent; chat contents are never returned.

### Tests
P2-RES-01..04 (reuse, marks, cleanup only agent windows, closing a user window asks).
P2-WA-01..04 (send text, send file, deny → nothing sent, bad number / missing app map) —
real sends go **only to the owner's own number**, given by the env var
`XTRAOS_TEST_WHATSAPP` (opt-in; never stored in the repo).

## Consequences
docs/04: `cleanup`, `launch_app.reuse`, `list_windows` agent marker, skill row; docs/05:
`close_user_window`; docs/08: P2-M4 cases.
