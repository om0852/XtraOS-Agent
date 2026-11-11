---
name: xtraos
description: Operate the user's Windows PC through XtraOS Agent from a shell, without MCP — see windows as text (UI Automation), click and type safely, run file/Office/routine skills. Use when the task needs a desktop app, a window, files through XtraOS, or anything on the user's screen, and the xtraos MCP tools are not available.
---

# XtraOS Agent from the command line

XtraOS Agent lets an AI agent see and operate this Windows PC. Every call goes through the
user's safety rules: approval dialogs, a kill switch, an action log. You get no more power
than through MCP.

## The command

```powershell
$x = "$env:LOCALAPPDATA\Programs\XtraOS\bin\xtraos-core.exe"
'{"filter":"notepad"}' | & $x call list_windows -
```

- `call <tool> [json | -] [--json] [--out file.png]` calls exactly one tool.
- Pass the JSON on stdin with `-`; it avoids shell quoting problems.
- A tool with a short text result prints that text; others (e.g. `status`) print JSON.
  `--json` always prints the raw result. Images (`screenshot`) need `--out`.
- Exit code 0 = ok, 1 = error, 2 = usage. Errors go to stderr as `ERROR <CODE>: message`
  followed by a `hint:` line.
- The core starts by itself if it is not running (tray icon). It runs as the user, locally.

From WSL (not verified yet): call the .exe by its Windows path through interop, e.g.
`/mnt/c/Users/<you>/AppData/Local/Programs/XtraOS/bin/xtraos-core.exe call status`.

## How to work

1. `call status` first. If it says `stopped` or `paused`, stop and tell the user.
2. Get a window: `list_windows {"filter":"..."}` or `launch_app {"app":"notepad"}` →
   a ref like `w460650`.
3. `observe {"window":"w460650"}` → elements with refs like `e12`. A repeat observe returns only
   what changed; `{"full":true}` returns everything.
4. Act on refs: `click {"ref":"e12"}`, `set_value {"ref":"e7","value":"text"}`,
   `scroll {"ref":"e3","dy":3}` work in the background — no cursor, no focus change.
   `type {"window":"w..","text":"..."}` and `press_keys {"window":"w..","keys":["Ctrl+S"]}` borrow
   the window while the user is idle.
5. Prefer `find {"window":"w..","query":"Save"}`, `read_text` and `wait_for` over screenshots.
   Use `screenshot` or `ocr` only when observe says the window is opaque (games, canvas).
6. `batch {"steps":[{"tool":"click","args":{"ref":"e12"}}, ...]}` runs up to 50 steps in one call.

## Skills (one call does a common job)

`call list_skills` shows them all. Through `run_skill {"name": "...", "args": {...}}`:

| Skill | What it does |
|---|---|
| `files.find` / `files.list` / `files.open` | find files (newest first), list a folder, open with the default app |
| `files.copy` / `move` / `rename` / `mkdir` / `delete` | file changes (delete and overwrites ask the user) |
| `files.duplicates` / `files.largest` | read-only clean-up reports |
| `files.organize` / `files.bulk_rename` | a preview; `apply: true` asks the user, then does it undoably |
| `office.excel_read` / `excel_write` | read or write a range (hidden Excel, macros off) |
| `office.word_read` / `word_append` | read a document, add paragraphs |
| `office.ppt_read` / `ppt_add_slide` | read slides, add a title + bullets slide |
| `office.export_pdf` | Word / Excel / PowerPoint → PDF |
| `dev.check` | assert text / elements / a visual change in a window (UI tests) |
| `task.save` / `task.load` / `task.list` / `task.done` | checkpoint a long task so another session can resume it |
| `routine.add` / `list` / `run` / `remove` / `summary` | run a skill on a schedule while XtraOS runs (adding asks the user) |
| `skill.export` / `skill.import` | share recorded skills (import shows the user every step) |

Example:

```powershell
'{"name":"files.find","args":{"path":"C:/Users/me/Documents","pattern":"*resume*"}}' | & $x call run_skill -
```

## Errors and what to do

| Code | Meaning | Do |
|---|---|---|
| `STALE_REF` | the element changed | observe again |
| `USER_ACTIVE` | the user is using mouse or keyboard | wait, retry later |
| `WINDOW_BUSY` | another agent is working in that window | work elsewhere or retry later |
| `NEEDS_HUMAN` | locked screen, UAC, a password or a dialog only the user can handle | tell the user |
| `BLOCKED_BY_POLICY` | the user's rules or answer said no | tell the user; never work around it |
| `STOPPED` | kill switch (Ctrl+Alt+Esc), or the user paused this agent | stop; only the user resumes |
| `ELEVATION_REQUIRED` | an admin window | ask the user (opt-in admin helper) |

## Rules you must keep

- Never type passwords, payment data or one-time codes; never create accounts or sign in for
  the user. Those fields are refused anyway.
- Sending messages, deleting files, changing settings, buying: the user approves in a dialog.
  Do not try to approve it yourself (XtraOS's own windows are off limits to you).
- Do not close windows the user opened unless asked; `cleanup` closes only what XtraOS opened.
- Keep the PC healthy: when a result carries a health warning (RAM, CPU, many windows), slow
  down and close what you opened.
- `status` tells you your access on this PC (`profile`: full, background_only, read_only,
  blocked). Stay inside it.

Full tool contract: `docs/04-mcp-tool-contract.md` in the XtraOS Agent repository.
