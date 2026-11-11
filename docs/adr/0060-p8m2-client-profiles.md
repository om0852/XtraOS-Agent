# ADR 0060 — P8-M2 Per-client permissions: full, background only, read only, blocked

- Status: accepted (owner: "Continue building and improvements", 2026-11-11; plan row P8-M2).
- Date: 2026-11-11

## Context
Several agents use one PC (ADR 0059). The owner may trust them differently: Claude Code fully,
an experimental harness only to read the screen. Today every client has the same rights.
Also, every MCP client paired as `xtraos-mcp`, so Claude Code and Codex were indistinguishable.

## Decision
1. **Client names.** The MCP server pairs with the MCP client's own name from `initialize`
   (`clientInfo.name`, e.g. `claude-code`), prefixed `mcp:` (`mcp:claude-code`); the CLI stays
   `xtraos-cli`. Names are self-declared by local processes that already hold the user's pairing
   token: profiles keep well-behaved agents within bounds; they are not a boundary against a
   malicious program running as the user (that program could use the token directly).
2. **Profiles** (`policy.json`):
   ```json
   "clients": { "mcp:codex-mcp-client": "background_only",
                "mcp:hermes": {"profile": "full", "apps": ["chrome.exe"]} },
   "new_clients": "full"
   ```
   - `full` — as today (the normal policy applies).
   - `background_only` — no real mouse, keyboard, touch or pen input: only actions done through
     the element itself (click/scroll/set_value by ref in the background) and everything else
     that sends no input. A call that would fall back to real input → `BLOCKED_BY_POLICY`.
   - `read_only` — only reading: observe, find, read_text, screenshot, ocr, frame_diff,
     wait_for, list_windows, status, list_skills, read-only skills, health, ask_human.
   - `blocked` — nothing but `status`.
   - optional `apps`: acting calls only in windows of these processes.
   Names match case-insensitively; a `*` entry is the fallback for unnamed clients.
3. **New clients.** `new_clients: "full"` (default — today's behaviour, no prompt), `"read_only"`,
   or `"ask"`: the first time an unknown client acts, the user picks Full access / Background
   only / Read only / Block in an XtraOS dialog (no answer = Read only); the choice is kept in
   `<data>\clients.json` and shown in the rules viewer.
4. `status` shows the caller its `profile`.

## Tests
- Unit: profile parsing (string / object / `*`), method classification for read_only, the apps
  allowlist, `new_clients`.
- Interactive: a core with a test policy giving client `agent-ro` read_only and `agent-bg`
  background_only — `agent-ro`'s observe works, its click is refused; `agent-bg`'s click by ref
  (background) works, its click at x,y (real input) is refused; `status.profile` matches.

## Implementation notes (built 2026-11-11)
- Order of lookup: the client's own `clients` entry → the user's dialog choice (`clients.json`)
  → `*` → `new_clients`. With `"ask"`, reading is allowed until the first change shows the dialog;
  an unanswered dialog gives read only for that call and asks again next time (not stored).
- A changed `clients` / `new_clients` entry appears in the "your safety rules were changed"
  approval (ADR 0026), so an agent cannot raise its own access by editing `policy.json` silently.
- `background_only` refuses up front calls that are real input by their arguments (`mouse`,
  `touch`, `pen`, click/scroll at a point, drag not ref-to-ref) and `focus_window`; everything
  else is caught where input would be sent (SendInput, pointer injection, borrowing the
  foreground) and where a call would go to the admin helper (another process).
- `apps`: the target windows of an acting call (or the foreground window when it names none;
  the app name for `launch_app`) must belong to a listed process; `chrome` matches `chrome.exe`.
- Refused calls take no window lease (P8-M1).
- Tests: `clients::tests` (6 unit tests), MCP `mcpClientName` test, interactive `p8m2_e2e`.
