# 07 — Development Plan

**Current phase: Phase 6 — Reliable daily driver** (ADR 0046; Phase 5 partly open, see below; ADR 0044, plan in `docs/09-next-phases-plan.md`; Phases 0–2 built, Phases 3–4 built except macOS, Linux, team policies and the skill registry; see the status lines under each phase). Phase 0 closed by ADR 0007 (gate 10 PASS /
0 FAIL / 1 SKIP). Toolchain installed and pinned (ADR 0008).
**M1 Core + IPC: closed** (ADR 0009) — gate 6/6 PASS twice from a clean build
(`tests/phase1/m1-gate.ps1`). Smart App Control turned off by the owner on this dev machine.
**M2 Input: closed** (ADRs 0010, 0011) — gate 8/8 PASS incl. 11/11 interactive, hands-off run;
`set_value` and Explorer→app drag
(P1-IN-07) moved to M4.
**M3 Windows & apps: closed** (ADRs 0012, 0013) — gate 9/9 PASS, 0 SKIP incl. UAC + browser
flow F3. **M4 Perception + background-first actions: closed** (ADRs 0014–0016) — gate 10/10
PASS, 0 SKIP; large-list observe latency is a stated miss (Phase 2). **M5 Safety: closed**
(ADRs 0017, 0018) — kill switch 15 ms, policy + approvals, overlay; safety e2e 6/6.
**M6 Release 0.1: closed as an unsigned developer preview** (ADRs 0019, 0020) — install
test 12/12, flows F2/F3/F4/F6/F10 recorded, bench report; open: signing + fresh SAC-on test.
**Phase 1 complete.** Phase 2 started 2026-11-07 (owner: "continue"); note: `wait_for` from the P1 contract was
not built in Phase 1 and is the first item of P2-M1. Only work on the current phase (see CLAUDE.md §3).
Each milestone has exit criteria; a milestone is done only when all are met and evidenced.

## Phase 0 — Spikes & skeleton (≈ 1 week)

Goal: prove every risky OS assumption before building on it. Each spike → one ADR.

| ID | Spike | Exit criteria |
|---|---|---|
| S1 | `SendInput` absolute click on each monitor with mixed DPI (100%/150%) | Clicks land within 2 px on testbed buttons on all monitors. |
| S2 | UI Automation tree walk of Notepad, Explorer, Chrome, Firefox, Windows Terminal | Element list with names/roles; time and element counts recorded. |
| S3 | Windows.Graphics.Capture window screenshot + mapping to screen coords | Saved PNG; mapping verified by clicking a detected point. |
| S4 | Real drag-and-drop Explorer → browser upload zone | File received by testbed page. |
| S5 | Global hotkey + releasing held input | Hotkey stops a running drag within 100 ms, no stuck keys. |
| S6 | Named pipe with user-only ACL + token from Node | Node connects; another user/process without token is refused. |
| S7 | UIPI check: input to elevated window from normal process | Documented failure + elevated helper path confirmed. |
| S8 | Tauri click-through top-most overlay | Overlay visible, does not intercept clicks. |

Progress (2026-11-07): S1, S2, S3, S6, S9 (real app: Calculator) done (ADRs 0003–0006;
single monitor only — multi-monitor needs a second display to verify).
S4, S5, S7, S8 not started. Spikes use built-in tooling (ADR 0002); Rust install deferred to Phase 1.

Deliverables: repo skeleton (`core/`, `mcp/`, `tray/`, `testbed/`, `bench/`), CI building on
Windows, ADRs 0002–0009 with results.

## Phase 1 — Foundation, Windows (≈ 4–5 weeks)

| Milestone | Scope | Exit criteria |
|---|---|---|
Gate: every Phase 1 test case in `docs/08-test-plan.md` PASSes (incl. S4/S5/S7/S8 carried over).

| M1 Core + IPC | Pipe server, pairing, sessions, error codes, action log | MCP `status` works end to end; unpaired client refused (test). |
| M2 Input | click/double/right, type (unicode + keys), press_keys, scroll, drag, mouse path, clipboard | All input tools pass e2e on testbed; latency budgets met. |
| M3 Windows & apps | list/focus/launch/close/bounds, elevated launch (UAC by human) | F3 flow passes (Chrome → close → Firefox search). |
| M4 Perception | observe (refs, caps, diffs), find, read_text, screenshot | Token budgets met on 5 bench apps; opaque detection works. |
| M5 Safety | policy engine (defaults), HITL `ask_human`/`wait_for_human`, kill switch, overlay | Kill switch ≤ 100 ms; confirm dialog blocks a `message_send`-class action. |
| M6 Release 0.1 | MCP packaging, install script, README capability table (verified only), **code-signed binaries** (Smart App Control, ADR 0009) | Fresh-machine install incl. a machine with Smart App Control on; flows F2, F3, F4, F6, F10 recorded. |

## Phase 2 — Smart & cheap (≈ 4–6 weeks)

Element-ref hardening, trusted `batch`, local OCR, app map (DPAPI), skills engine + first
skills (WhatsApp desktop/web send file, Telegram send, Explorer file ops), elevated helper,
browser driver bridge to XtraDevPilot, MSI packaging.
Exit: F5 and F7 pass; routine task ≤ 3 calls / ≤ 2,000 tokens in bench.

Milestones (ADR 0021): P2-M1 fewer calls (`wait_for`, scoped observe, trusted `batch`) ·
P2-M2 OCR · P2-M3 app map + skills engine (Explorer file ops) · P2-M4 messaging skills (F7, in
an owner-chosen test chat) · P2-M5 elevated helper (F5) · P2-M6 browser bridge + packaging.
**P2-M1 Fewer calls: closed** (ADRs 0022, 0023) — gate 13/13; routine task 3 calls / ~335 tokens.
**P2-M2 OCR: closed** (ADRs 0024, 0025) — gate 14/14; opaque window read in ~54 tokens, 33 ms.
**P2-M3 app map + skills: closed** (ADRs 0026, 0027) — routine file task in one call; policy edits need
the user's OK; input hooks ignore jitter and the core's own input.
**P2-M4 messaging + resource discipline: built** (ADRs 0028, 0029). `whatsapp.send` (text and
file, with the user's OK) sends to the owner's own chat. The agent tracks, reuses and cleans up
only the windows it opened; closing a user window asks. Closes after the hands-off interactive gate.
**P2-M5 elevated admin helper: built** (ADRs 0030, 0031). It is opt-in per session through UAC and
locked to the core. It asks the user in its own elevated dialog for each admin window. The
automated gate passes 11/11; it closes after the human F5 run with a real UAC prompt.
**P2-M6 browser bridge + packaging: built** (ADRs 0032, 0033). The core drives Chrome pages
through the XtraDevPilot bridge (loopback + token, an action allowlist, the same policy); the
zip + install.ps1 package stays (no MSI). The automated gate passes 12/12. The live check
needs an XtraDevPilot build that includes the bridge security fix.
**P2-M7 Phase 2 leftovers: built** (ADRs 0034, 0035).
- Off-screen and virtualised list rows can be found (`find deep`); clicking a row selects it.
- Send/Buy/Delete words are recognised in Hindi, Marathi, Spanish, French, German and Portuguese.
- A read-only rules and activity viewer.
- Input census.
- Automated gate: 13/13.
**Phase 2 build complete.** Open: the hands-off interactive regression, F5 with real UAC, and P2-BR-LIVE.

## Phase 3 — Reach

**P2-M8 rules editor + XtraOS windows off limits; P3-M1 `frame_diff`; P3-M2 `record_session`;
P3-M3 `touch`/`pen`: built** (ADRs 0036, 0037). The automated Phase 3 gate passes 14/14. The
real touch/pen tests run in the hands-off session.
**P3-M4 sandbox desktop: built** (ADR 0040). The agent works in Windows Sandbox with its own cursor; the live test needs the owner to enable Windows Sandbox. Microsoft's Agent Workspace stays out until it has a public API.
Open:
P3-M5 macOS (needs a Mac).

Game/canvas helpers (frame diff, detector hooks), touch & pen injection (`InjectSyntheticPointerInput`),
multi-finger gestures, recording/replay, macOS platform (CGEvent, AX, ScreenCaptureKit).

## Phase 4 — Platform

**Part 1 built** (ADR 0042): `audit_export` (CSV/JSON, chain check, head hash), the Node SDK
(`sdk/node`) and the Python SDK (`sdk/python`). Open: Linux (needs a Linux machine), team
policies (needs an org model from the owner), skill registry (later, with connectors/plugins).

Linux (AT-SPI, libei/portals), SDKs (Node/Python), team policies + audit export, skill registry.

## Phase 5 — Connect anywhere

Goal: easy to install, and usable from any AI tool (IDE agents, Hermes Agent, OpenClaw).
Milestones and what each needs from the owner: ADR 0044. Later phases (6 reliable daily
driver, 7 everyday work, 8 many agents): `docs/09-next-phases-plan.md`.
**P5-M7 lean tool surface: built** (ADR 0044) — server instructions; `XTRAOS_TOOLS=core`
lists 15 tools (~3,400 tokens instead of ~8,500).
**P5-M3 `xtraos setup` / `xtraos doctor`: built** (ADR 0045) — dry run by default, backs up each
config, 9 clients incl. Hermes Agent; tests SU-01..SU-12 pass; the live check of each client is open (P5-M9).
**P5-M6 skill package: built** — `skills/xtraos/SKILL.md` teaches the command-line bridge;
every command in it was re-run against a core (2026-11-11); an unknown tool now says
"unknown tool 'x'" with a pointer to the list. **P5-M9 integration guide: built** —
`docs/10-integration-guide.md` with a compatibility matrix; verified: Claude Code (MCP) and the
command line; the other 8 clients are config tested, live runs pending.
**P5-M1 / M2 / M4 / M8: built as far as code goes** (ADR 0071, 0072) — CI and a release
workflow that makes a draft GitHub Release (zip, checksums, winget manifests that pass
`winget validate`); `npx xtraos-install` (downloads only from this repository's releases,
refuses a checksum mismatch); `release/sign.ps1` (thumbprint or PFX); SDKs ready to publish with
an owner-triggered workflow; `xtraos-mcp --http` on 127.0.0.1 with its own token (CLAUDE.md §4
amended). Found and fixed on the way: an auto-started core kept the caller's folder as its
working directory (a project or an extracted bundle could not be deleted); it now runs from its
own folder. Open (owner): publish the first release, a code-signing certificate, npm org and
PyPI publishing; WSL (P5-M5), live runs per client (P5-M9), the OpenClaw install run (P5-M6).

## Phase 6 — Reliable daily driver

Goal: it works the same way every time; every item comes from a real failure
(`docs/09-next-phases-plan.md` §2). Scope: ADR 0046.
**P6-M1 window targeting, P6-M2 focus truth, P6-M3 clearer argument errors: built** (ADR 0046)
— `window` accepts a title part or process name; a reused handle is `STALE_REF`;
`focus_window` reports whether the window stayed in front; `INVALID_ARGS` carries an example.
Unit tests + interactive `win06` pass.
**P6-M4 dialog awareness: built** (ADR 0047) — `observe` puts a `!! dialog` line (browser alert
inside the window, or an owned message box) right after the header with its button refs;
`status.dialog` names a message box of the front window. Interactive `p6m4_e2e` (testbed
message box; Edge alert with `XTRAOS_TEST_BROWSERS=1`) passes.
**P6-M5 browser/Electron page content: built** (ADR 0048) — the first read of a Chrome, Edge or
Electron window waits (≤ 1.5 s) for its page instead of returning only the browser frame;
`observe {reveal:true}` reads a minimized one without a focus change; `page` says why a page is
missing. Interactive `p6m5_e2e` (Edge + Chrome) passes.
**P6-M6 work beside the user: built** (ADR 0049) — `type`, `press_keys` and real clicks borrow a
window that is not in front once the user is idle 1.5 s, then give the user's window back (on by
default, owner decision); `type {ref, text}`; `set_value` on a drop-down selects the option;
a plain clickable web `<div>` is clicked through its parent. Interactive `p6m6_e2e` (a form behind
the testbed, Edge + Chrome) passes.
**System health guard: built** (ADR 0050, owner request) — `system {op: "health"}` reads CPU,
RAM, GPU, temperatures, disks, Wi-Fi signal, battery and the apps using most memory without
admin; a 5 s watchdog warns agents (`health_warning` on every result, `⚠` in MCP) and the user
(banner) when the PC is under strain or the agent keeps opening windows/tabs; at a critical level
opening more asks the user. File skills also refuse Windows' hidden system folders at drive roots.
Interactive `health01` passes.
**P6-M7 small tools: built** (ADR 0051) — `wait_for {delay_ms}` (plain wait), `ask_human
{kind: "notify"}` (Windows notification, ≤ 1 per 10 s; also used for critical health warnings),
`launch_app` opens `chrome://`/`edge://` pages through the address bar. The P6-M6 user-interrupt
stop was seen live (USER_ACTIVE while the owner used the PC). Interactive `p6m7_e2e` passes.
Open: P6-M8 large lists (parked: re-measure on a quiet PC), P6-M9 open human-run gates, a clean
hands-off run of all interactive suites.

## Phase 7 — Everyday work

Owner choice (2026-11-10): the **developer pack** first.
**P7-M3 slice 1: built** (ADR 0052) — `run_skill dev.check` checks a window against several
expectations (text, no text, element, title) and reports PASS/FAIL for each, retrying until a
timeout and saving a screenshot on failure. Reading terminals verified: consoles, Windows
Terminal and the VS Code editor work; the VS Code integrated terminal exposes no text.
Interactive `p7m3_e2e` passes.
**P7-M3 slice 2: built** (ADR 0053) — `record_session {op: start, report: true}` writes a
"reproduce this bug" report to Documents\XtraOS reports: every action (failed ones too) with a
screenshot of the app, `note` observations, versions and a replay hint. Interactive `p7m3b_e2e`
passes. `dev.check` also checks an element's value and state and a visible change.
Installer and emulator flows: not built, by decision (2026-11-11). A dedicated installer skill
would accept license terms on the user's behalf, which needs the user each time; the existing
tools already drive a wizard with the user approving. Emulator flows need an Android SDK,
which this PC does not have.
**P7-M4 routines: built** (ADR 0063) — a scheduler inside the core (nothing registered with
Windows): `routine.add` always asks the user; every N minutes (≥ 5) or daily at HH:MM; a
missed run runs once; each run is the agent `routine:<name>` through the normal rules, logged,
pausable from the tray; `routine.summary` is the daily summary; a routines file edited outside
XtraOS waits for the user's OK. `p7m4_e2e` passes. **Slice 2: built** (ADR 0064) — a
`new_file_in {folder, pattern}` trigger runs the skill once per new file (settled 10 s, not an
unfinished download, files already there never; `{file}`/`{name}` in the arguments); `notify`
shows a Windows notification on failure (only when it starts failing or recovers), always or
never; the "Rules and activity" window lists the routines. Checked live: only the one new
.txt was processed, once.
**P7-M2 housekeeping: built** (ADR 0054) — `files.duplicates` (byte-for-byte), `files.largest`,
`files.organize` (by type or month) and `files.bulk_rename`; the last two preview first and move
or rename only after the user approves the plan, in one undoable shell operation. Unit tests on
a real temp folder pass; on the owner's Downloads (read-only): 148 GB in 3,384 files, 2.7 GB in
duplicates found, 384 loose files planned into 8 folders.
**P7-M1 Office pack slice 1: built** (ADR 0055) — `office.excel_read`, `office.excel_write` and
`office.export_pdf` through Office automation, each in its own hidden Office instance (macros
forced off, no dialogs, a password-protected file fails instead of prompting), ended reliably
afterwards. Interactive `p7m1_e2e` passes on the owner's Office 16.
**Slice 2: built** (ADR 0056) — `office.word_read` and `office.word_append` (creates the
document if needed). Outlook drafts stay planned: the owner's PC has no classic Outlook mail
profile, so they cannot be verified. **Slice 3: built** (ADR 0062) — `office.ppt_read`
(each slide's title, text and optional notes) and `office.ppt_add_slide` (title + bullets
slide at the end or at `at`; a missing .pptx creates the deck; `save_as` never overwrites);
`p7m1_e2e` checks it live with a real PowerPoint, including PDF export and that no PowerPoint
process is left. Open: Outlook drafts.
**P7-M11 local vision: built** (ADR 0070) — `vision.colors` (colour shares + a letter grid
map), `vision.regions` (coloured shapes with box, centre and a shape hint), `vision.find_image`
(where a saved icon is), `vision.snip` (save an area as PNG); with `window` + `region` they read
a covered window. On the owner's PC: 85 / 241 / 21 tokens instead of ~995 for a screenshot, and
`ocr {window, query}` read the VS Code chat line in 31 tokens instead of ~1,004.
**P7-M10 everyday pack 2: built** (ADR 0068) — `window.snap` (halves, quarters, center,
maximize on the window's own monitor), `files.recent` (changed lately in the real Desktop,
Documents and Downloads, also under OneDrive), `files.read_text` (text files for MCP-only
clients; binary refused), `disk.usage` (LOW under 10 % / 5 GB), `routine.pause / resume`.
One interactive test, `everyday_e2e`, covers all five.
**P7-M9 find on the whole screen: built** (ADR 0067) — `screen.find {query}` runs `find` on
every open window (front first, within 5 s by default) and returns the matches per window
with refs that work at once. `screen_e2e` passes; 2.1 s cold, 0.4 s warm over 4 windows.
**P7-M8 reminders: built** (ADR 0066) — `notify.show {text, title?}` shows a Windows
notification; a routine with `once_at` ("17:00" or "2026-11-12 09:30") runs once and is then
removed, so a reminder is `routine.add {skill: "notify.show", once_at}` (approved by the user
like any routine). A reminder missed while XtraOS was off runs once when it is back. Checked
live: shown at its time, then gone from `routine.list`.
**P7-M7 window layouts: built** (ADR 0065) — `layout.save {name, filter?}` remembers the open
windows (position, size, maximized/minimized); `layout.restore {name}` puts the open ones back
(same process, then the closest title), in the background, and reports the ones that are not
open. `layouts_e2e` passes (back within 3 px; minimized stays minimized).
**P7-M6 task checkpoints: built** (ADR 0057) — `task.save / load / list / done` keep a long
task's progress in XtraOS's own data folder so another agent or session can resume; a state
with a credential-looking key is refused. Checked live: saved by one client, loaded by another.
**P7-M5 skill sharing: built** (ADR 0058) — `skill.export` writes a recorded skill as one file
(typed text left out unless asked); `skill.import` checks the file and shows the user every step
in plain words; only "Import" keeps it. Unit tests pass; not run live (both show a dialog).

## Phase 8 — Many agents, more trust

**P8-M1 several clients at once: built** (ADR 0059) — an acting call leases the windows it
targets for its agent session (until 20 s after its last action there, or its disconnect);
another session waits in a first-come queue for 10 s, then gets `WINDOW_BUSY` naming the holder.
Reading is never blocked; the user never is. `status` lists the leases. Interactive `p8m1_e2e`
(two agents, one testbed) passes.

**P8-M2 per-client permissions: built** (ADR 0060) — the MCP server pairs as
`mcp:<clientInfo.name>`; `policy.json` `clients` gives each client `full`, `background_only`
(no real mouse/keyboard/touch/pen, no focus change, no admin helper), `read_only` or `blocked`,
optionally only in listed `apps`; `*` is the fallback; `new_clients` is `full` (default),
`read_only` or `ask` (the user picks in a dialog; kept in `clients.json`). A changed client rule
needs the user's approval like any other rule. `status.profile` shows it. Names are
self-declared: this keeps well-behaved agents in bounds, it is not a security boundary.
Interactive `p8m2_e2e` passes.

**P8-M3 activity dashboard: built** (ADR 0061) — the core tracks each agent session (client,
last method, calls, refused; never arguments) and returns it as `status.agents`. Tray →
Agents lists connected agents; clicking one pauses (or resumes) it: its calls, including one
already running, get `STOPPED` while the others keep working, and its `status.state` is
`paused`. Pause/resume come only from the tray menu. The "Rules and activity" window shows a
live "Agents now" section (it pairs as `xtraos-viewer`, its polls are not logged, and a
"viewer" that acts is listed like any agent). Interactive `p8m3_e2e` passes; the tray submenu
needs a real click and was not exercised by a test. **P8-M4 team policies: built** (ADR 0073)
— an organisation signs `team-policy.json` with its key (`xtraos-core --team-keygen`,
`--team-sign`); the core verifies it with Windows CNG and applies it as a floor: the stricter of
the user's and the team's decision, profile and limits wins; a file that does not verify blocks
acting calls (reading still works); `status.team` shows it. `team_e2e` passes. Earlier note: P8-M4 team policies (owner's org
model).

## Working agreement

- Each task: issue → branch → small commits → PR with evidence (test output / recording).
- Spec/contract changes land before code (with ADR).
- Weekly: run `bench/`, update the budget table, review backlog.
