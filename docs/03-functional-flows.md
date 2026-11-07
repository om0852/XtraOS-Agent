# 03 — Functional Flows

Each flow lists the tool calls the agent makes. Fewer calls = cheaper and faster.
Flows marked (P2) depend on Phase 2 features.

## F1. Pairing and session start

1. User installs XtraOS Agent → tray starts core.
2. Core creates the pairing token (DPAPI-protected file in `%LOCALAPPDATA%\XtraOS\`).
3. MCP client config launches `xtraos-mcp`; it reads the token and opens the named pipe.
4. Core checks pipe ACL (same user) + token → creates `Session`; tray shows "Connected: <client>".
5. Failure → MCP returns `NOT_PAIRED` with a hint to open the tray app.

## F2. Observe → act loop (the basic loop)

```text
observe()                      → compact tree of focused window (refs)
click(ref)/type(ref,text)/...  → result + small diff
(repeat)
```
Rules: act by `ref` when available; coordinates only for opaque surfaces.

## F3. Open a browser, search, close, open Firefox (no extension)

```text
launch_app("chrome", args=["https://www.google.com/search?q=xtraos"])   # 1 call
observe()                                                              # verify results
close_window(window="chrome")                                         # 1 call
launch_app("firefox")                                                  # no extension needed
observe() → e3 Edit "Search or enter address"
type(ref="e3", text="xtraos agent", submit=true)
```

## F4. Drag and drop between apps

```text
list_windows()                                   # find Explorer + browser
observe(window="Explorer")  → e21 ListItem "report.pdf"
observe(window="Chrome")    → e40 Group "Drop files here"
drag(from_ref="e21", to_ref="e40")               # real OS drag (press, path, release)
```
Core performs a real mouse drag with intermediate moves so OLE drag-and-drop engages.

## F5. Admin command prompt (human in the loop)

```mermaid
sequenceDiagram
  participant A as Agent
  participant C as Core
  participant H as Human
  A->>C: launch_app("cmd", elevated=true)
  C->>H: Windows UAC prompt (secure desktop — core cannot see/click it)
  H-->>C: approves (or denies)
  alt elevated helper installed (P2)
    C-->>A: {ok, window: "Administrator: Command Prompt"}
    A->>C: type(window, "ipconfig", submit=true)  (routed via xtraos-elevated)
  else no helper
    C-->>A: {ok, window, note: "ELEVATION_REQUIRED for input; install helper"}
  end
```
The agent is told explicitly that UAC is a human step (`NEEDS_HUMAN` while pending).

## F6. Coding with Claude Code in a terminal

```text
launch_app("wt", args=["-d", "C:\\proj"])        # Windows Terminal
type(window="Terminal", text="claude", submit=true)
wait_for(window="Terminal", text_contains="│ >")  # prompt ready
type(window="Terminal", text="add unit tests for utils.ts", submit=true)
wait_for(window="Terminal", idle_ms=3000)
read_text(window="Terminal", last_lines=40)      # terminal text via UIA TextPattern
```

## F7. Send a file on WhatsApp (P2: app map + skill + confirmation)

```text
whatsapp.send_file(contact="Rahul", path="C:\\Users\\me\\report.pdf")   # ONE call
```
Inside the skill (no AI round trips):
1. App map → WhatsApp surface = desktop app (or Chrome profile 2 + web.whatsapp.com).
2. Launch/focus; search contact; verify exact match (ambiguous → `NEEDS_HUMAN` with options).
3. Attach file via the app's attach flow (or drag-drop).
4. **Policy: sending messages = confirm** → HITL dialog "Send report.pdf to Rahul?".
5. Approved → press send → verify the bubble appears → `{ok, sent_at}`.

## F8. Game / canvas app (OCR targeting P2-M2; frame helpers P3)

```text
observe() → {"opaque": true}
ocr(window, query="Start Game")                  # P2-M2: text + screen coordinates, no image
click(x, y)                                      # at the OCR box centre
screenshot(window, scale=0.5)                    # only when there is no text to read
mouse(action="click"|"drag"|"path", x, y, space="window")   # window-relative coords
```
Optional per-game helper: core streams frames to a local detector; agent gets compact events.

## F9. Credential / CAPTCHA / sensitive field encountered

1. Core detects a password field (UIA `IsPassword`), a CAPTCHA widget, or an OS credential
   prompt in the target.
2. Action returns `NEEDS_HUMAN { kind: credential|captcha }`; tray asks the user to complete it.
3. Agent waits with `wait_for_human(request_id)`; resumes after the user clicks "Done".

## F10. Kill switch

1. User presses Ctrl+Alt+Esc (or tray Stop).
2. Core cancels the queue, releases all held keys/buttons, blocks new actions → `STOPPED`.
3. Overlay shows "Stopped". New actions require the user to press Resume in the tray.

## F11. Errors and recovery (agent guidance)

| Code | Agent should |
|---|---|
| `ELEMENT_NOT_FOUND` / `STALE_REF` | `observe()` again, then retry once. |
| `WINDOW_NOT_FOUND` | `list_windows()` or `launch_app`. |
| `BLOCKED_BY_POLICY` | Stop and tell the user; do not try alternatives. |
| `NEEDS_HUMAN` | Tell the user what is needed; `wait_for_human`. |
| `ELEVATION_REQUIRED` | Ask the user to install/enable the elevated helper. |
| `STOPPED` | Stop entirely; wait for the user. |
| `TIMEOUT` | Re-observe; report if repeated. |
