# ADR 0026 — P2-M3 scope: app map, skills engine, file skills

- Status: accepted (scope). Findings and gate → ADR 0027.
- Date: 2026-11-07

## Context
Cost lever #1 (docs/06): one call per routine task. The agent also needs memory of *where*
things are ("WhatsApp is the Store app", "Gmail is Chrome profile 2"). docs/02 planned YAML
skills; YAML would add a parser dependency for no gain over JSON (serde_json is already in).

## Decision
1. **App map** (`app_map` tool, ops `scan` / `list` / `get` / `set` / `remove`):
   - `scan(query?)`: installed apps from the Windows Apps folder (classic and Store apps) —
     name + launch id (AUMID or path), so the agent never guesses launch names.
   - `set(service, surface: "desktop"|"browser", location)`: desktop → a launch id/path;
     browser → `{browser, url, profile?}`. Stored per user in `%LOCALAPPDATA%\XtraOS\appmap.bin`
     **encrypted with DPAPI** (CryptProtectData, current user; free, built in).
   - `launch_app(app)` accepts a mapped service name (desktop: its launch id; browser: the
     browser with the URL and profile). Policy class: get/list/scan `observe`, set/remove `input`.
2. **Skills engine** (`list_skills(app?)`, `run_skill(name, args)`):
   - **Built-in skills** = Rust executors with a JSON signature (name, description, args, class).
   - **Macro skills** = JSON files in `%LOCALAPPDATA%\XtraOS\skills\*.json`:
     `{name, description, args: {n: {required?, default?}}, steps: [{tool, args}]}` with
     `"${n}"` placeholders; they run through `batch` — **every step still passes the policy**.
     (Replaces "YAML" in docs/02.)
3. **First skills — files** (Explorer file operations through the Windows shell, not by
   clicking Explorer: deterministic, undoable, one call):
   `files.list(path, pattern?)`, `files.open(path)` (Explorer at a folder / reveal a file),
   `files.mkdir(path)`, `files.copy(from[], to_dir)`, `files.move(from[], to_dir)`,
   `files.rename(path, new_name)`, `files.delete(paths[])` → **Recycle Bin, never permanent**.
   - Copy/move/rename never overwrite: an existing target → `INVALID_ARGS`, nothing done.
   - New policy class `file_change` (allow by default) for mkdir/copy/move/rename;
     `files.delete` is class `delete` → **confirm dialog listing the items**.
   - **Protected paths → `BLOCKED_BY_POLICY`**: the Windows folder, Program Files,
     ProgramData, other users' profiles, network (UNC) paths, drive roots, and XtraOS's own
     install and data folders (pairing token, policy, logs, app map).
4. **Hardening found while designing this:** the agent could loosen its own rules by editing
   `policy.json` (with a file skill — now blocked — or by typing in an editor). **A changed
   policy file now applies only after the user confirms it in a core dialog**; denied → the
   previous policy stays and the action log says so.

## Tests (docs/08, P2-M3)
P2-AM-01..03, P2-SK-01..06, P2-POL-01.

## Consequences
docs/04 Skills & memory rows precise; docs/02 skills line (JSON, not YAML); docs/05 protected
paths + policy-change confirmation; MCP tools `app_map`, `list_skills`, `run_skill`.
