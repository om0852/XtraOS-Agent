# 09 — Next phases plan (proposal, 10 Nov 2026)

**Status: proposal for the owner.** Nothing here is built. Once approved, each phase gets its
scope ADR and `docs/07-development-plan.md` is updated (CLAUDE.md §3).

**Goal:** make XtraOS Agent something people use every day, and something any AI tool can
plug into: IDE agents (Claude Code, Cursor, VS Code, Windsurf, Cline, Codex CLI, Gemini CLI)
and agent harnesses (Hermes Agent, OpenClaw, custom SDK agents).

## 1. Where we are

Built and verified (README table): 42 MCP tools over stdio, CLI bridge, Node + Python SDKs
(local only), policy engine, approvals, kill switch, hash-chained log, OCR, skills, session
recording, sandbox, elevated helper, browser bridge.

What blocks daily use and wide adoption today:

| Gap | Why it matters |
|---|---|
| No prebuilt download; users must build with Rust + VS Build Tools | Most users stop here. |
| Unsigned binaries; Smart App Control must be off | Blocks normal Windows 11 PCs. |
| Installer configures only Claude Code | Every other client needs manual config. |
| SDKs not on npm / PyPI | Harness authors cannot `npm i` / `pip install`. |
| stdio only | Hermes and OpenClaw usually run in WSL2 or on another box; they need a verified WSL path or a local HTTP endpoint. |
| 42 tools always listed, no server `instructions` | Costs tokens and confuses smaller models. |
| One agent at a time; needs the foreground for real input | The user and the agent fight for the screen during long tasks. |
| Open human-run gates (real UAC run, live browser bridge, fresh-PC install) | Claims stay "pending". |

## 2. What real use showed (this week's long session)

XtraOS was used for hours as a helper beside a browser tool. Concrete problems hit:

1. **Window targeting.** The contract says `window` accepts a title substring; the CLI
   refused it (`window must be a ref like "w123"`). The agent had to parse `list_windows`.
2. **Stale window handle.** After Chrome restarted, the saved handle belonged to Firefox.
   `focus_window` then focused the wrong app without any warning.
3. **Focus does not stick.** `focus_window` returned `focused: true`, but another window took
   the foreground again seconds later; nothing reported it.
4. **Argument guessing.** `press_keys` with `keys: "ctrl+t"` and `click` with `target:` failed.
   Errors pointed to a doc file instead of showing the right shape.
5. **Browser dialogs.** A JavaScript `alert` and a "Leave site?" box froze the browser tool.
   Only XtraOS could see and click them. This is a strong use case, but the agent had to guess
   that a dialog existed.
6. **`observe` surprises.** "no changes" diffs and a 3-element "opaque" result appeared when a
   full list was needed; Chrome page content was missing until the window was in front.
7. **Chrome internal pages.** Typing `chrome://extensions` into the address bar did not open it.
8. **Foreground contention.** Web forms that need real clicks failed whenever the user was
   using another window. This was the largest time loss.
9. **No plain wait.** The agent needed "wait 5 s then check"; `wait_for` needs a condition.

These go straight into Phase 6.

## 3. How other tools connect (checked 10 Nov 2026)

- **Hermes Agent** reads MCP servers from `~/.hermes/config.yaml` (`mcp_servers`), supports
  stdio and remote HTTP servers, and reloads with `/reload-mcp`. Native Windows is early beta;
  WSL2 is the usual path, and Windows executables are run through WSL interop.
- **OpenClaw** has `mcp.*`, `skills.*` and `plugins.*` config, an `openclaw mcp` command, and
  skills as folders with a `SKILL.md` (installable from Git or a local folder). It is also
  commonly run under WSL2 on Windows.
- **IDE agents** (Claude Code, Cursor, VS Code, Windsurf, Cline, Codex CLI, Gemini CLI) take a
  stdio MCP entry in their own config file.

So three connection forms cover everyone: **stdio MCP**, **the CLI bridge wrapped in a
skill** (`SKILL.md`), and **a local HTTP MCP endpoint** for agents that cannot spawn a Windows
process. The exact config of each client must be verified in Phase 5, not assumed.

## 4. Proposed phases

### Phase 5 — Connect anywhere (adoption) · ~3 weeks

| ID | Work | Exit criteria |
|---|---|---|
| P5-M1 | **Prebuilt release**: GitHub Release with the zip + checksums; winget manifest; `npx xtraos-mcp` bootstrap that downloads the verified bundle | A fresh PC installs without Rust in under 2 minutes (recorded). |
| P5-M2 | **Code signing** (SignPath Foundation or owner certificate) | Install and first run pass with Smart App Control on. |
| P5-M3 | **`xtraos setup`**: detects installed clients and writes their MCP entry after showing the change; `xtraos doctor` checks core, pipe, pairing and each client | Verified on Claude Code, Claude Desktop, Cursor, VS Code, Windsurf, Cline, Codex CLI, Gemini CLI. **Built (ADR 0045); live check per client open.** |
| P5-M4 | **Publish SDKs**: `@xtraos/sdk` on npm, `xtraos` on PyPI | Install from the registry on a clean machine; SDK tests pass. |
| P5-M5 | **WSL path**: verify `xtraos-mcp.cmd` and `xtraos-core.exe call` through WSL interop; document the config for Hermes and OpenClaw | Hermes and OpenClaw in WSL2 each complete flow F2 (recorded). |
| P5-M6 | **Skill package**: `skills/xtraos/SKILL.md` teaching the CLI bridge, for OpenClaw, Hermes and Claude Code skills | Installed from Git in OpenClaw; one task done with no MCP. |
| P5-M7 | **Lean tool surface**: server `instructions` (how to work: observe → ref → act); tool profiles `core` (15 tools) / `full`; shorter descriptions | `core` profile `tools/list` ≤ 3,500 tokens (measured ~3,400 with 15 tools, down from ~8,500; ADR 0044); routine task still ≤ 3 calls. **Built.** |
| P5-M8 | **Local HTTP MCP endpoint** (opt-in, loopback only, pairing token). **Needs an ADR first**: CLAUDE.md §4 says local IPC only; loopback with a token is local, but the owner must decide | Security test: no token → refused; non-loopback bind impossible. |
| P5-M9 | **Integration guide** per client + a compatibility matrix in the README (verified rows only) | Each row links to a recorded run. |

### Phase 6 — Reliable daily driver · ~4 weeks

| ID | Work | Exit criteria |
|---|---|---|
| P6-M1 | **Window targeting**: accept title / process substrings everywhere (as the contract says); `launch_app` and `list_windows` return a stable id (handle + pid + start time) so a reused handle gives `STALE_REF` | The two failures in §2.1–2.2 have regression tests. **Built (ADR 0046).** |
| P6-M2 | **Focus that reports truth**: `focus_window` verifies the foreground after 300 ms; new `hold_focus` option for a batch; result says who took focus | Test with a focus-stealing window. **Built (ADR 0046): result reports `taken_by`; `hold_focus` not built.** |
| P6-M3 | **Forgiving arguments**: common aliases (`keys` as string, `target`→`ref`), and every `INVALID_ARGS` shows one correct example | Contract test per tool. **Built (ADR 0046): `keys` string, example hints, accepted-argument list; no other aliases.** |
| P6-M4 | **Dialog awareness**: `observe` / `status` report a blocking dialog of the target app first (browser alerts, "Leave site?", Save prompts) with its buttons | Flow test with a browser alert. **Built (ADR 0047): Edge alert and testbed message box tests pass; "Leave site?" and Firefox not yet verified live.** |
| P6-M5 | **Browser and Electron content**: switch on app accessibility when first needed, so page content is readable without the window in front; fix the `observe` diff / opaque surprises (§2.6) | Chrome, Edge, VS Code, Slack read in background (bench). **Built (ADR 0048): first read waits for the page; covered windows read fine; minimized ones via `observe reveal:true`. Edge + Chrome tested (`p6m5_e2e`), VS Code checked live; Slack not installed (planned). Not built: switching accessibility on by other means (not needed in the spike).** |
| P6-M6 | **Work beside the user, v2**: spike the options for real input without taking the user's screen (second virtual desktop, a lighter sandbox, per-window input); pick one in an ADR | A 10-minute form task runs while the owner keeps typing elsewhere. **Built (ADR 0049): no true background typing exists for browsers (spike), so real input borrows the window while the owner is idle (on by default); `type {ref}`, `<select>` by option, parent click; `p6m6_e2e` passes on Edge + Chrome. The 10-minute live run with the owner typing is not done.** |
| P6-M7 | **Small tools**: `wait` (plain delay ≤ 60 s), `notify` (Windows toast when the agent needs the human), open internal browser pages via `launch_app` | Unit + e2e tests. **Built (ADR 0051): `wait_for {delay_ms}`, `ask_human {kind: notify}` (tray notification, throttled), `launch_app` types internal pages into the address bar; `p6m7_e2e` passes.** |
| P6-M8 | **Large lists**: event-driven UIA cache to meet the 120 ms budget | Budget met on the 3 bench windows. |
| P6-M9 | **Close the open gates**: real UAC helper run, live browser bridge, fresh-PC install, multi-monitor | README rows move from "pending" to "verified". |

### Phase 7 — Everyday work · ~4 weeks

Skills people ask for daily, each one call, each policy-checked:

| ID | Work |
|---|---|
| P7-M1 | **Office pack**: Excel (read / write a range, save as), Word (insert, export PDF), Outlook (draft only; sending asks), PowerPoint |
| P7-M2 | **Desktop housekeeping**: sort Downloads by rule, bulk rename, find duplicates, tidy desktop, screenshot to file |
| P7-M3 | **Developer pack**: drive the user's own app for UI checks (`assert` text / element / frame change), read VS Code and terminal output, emulator and installer flows, "reproduce this bug" recording |
| P7-M4 | **Routines**: run a skill on a schedule or trigger (Task Scheduler), with a daily summary; changes ask first |
| P7-M5 | **Skill sharing**: export / import a recorded skill as one file with a review screen (first step of the skill registry) |
| P7-M6 | **Task checkpoints**: long tasks save progress so another agent or session can resume |

Exit: 10 recorded everyday flows, each ≤ 3 calls and ≤ 2,000 result tokens.

### Phase 8 — Many agents, more trust · ~3 weeks

| ID | Work |
|---|---|
| P8-M1 | **Several clients at once**: sessions with per-window locks and a fair queue; `status` shows who holds what |
| P8-M2 | **Per-client permissions**: profiles such as read-only, app allowlist, "no real input"; chosen by the user when a client first pairs |
| P8-M3 | **Activity dashboard**: live view of what each agent does, with pause per agent |
| P8-M4 | **Team policies** and signed policy files (needs the owner's org model) |

### Phase 9 — Platforms

macOS (needs a Mac), then Linux, as already planned. Unchanged.

## 5. Order and reasons

1. **Phase 5 first.** Reach is the biggest gap: today only someone with a Rust toolchain and
   Claude Code can try it. P5-M1, M2, M3 and M7 alone remove most of the friction.
2. **Phase 6 next.** Reliability decides whether people keep using it; every item comes from a
   real failure.
3. **Phase 7** builds daily value on a stable base. **Phase 8** matters once several agents
   share one PC.

First two weeks if approved: P5-M1 (release), P5-M7 (lean tools), P6-M1 + P6-M3 (cheap,
high-impact fixes), and the P5-M8 ADR.

## 6. Decisions needed from the owner

1. **Local HTTP endpoint (P5-M8):** allow loopback + token, or stay pipe-only and rely on WSL
   interop and the CLI skill?
2. **Code signing:** apply to SignPath Foundation (free for open source, takes time) or buy a
   certificate?
3. **Phase order:** adoption first (as proposed) or reliability first?
4. **Phase 7 packs:** which pack first — Office, housekeeping or developer?

## 7. Unchanged boundaries

No LLM inside, no CAPTCHA solving, no password or payment typing, no UAC automation, no
hidden operation, no remote control. Every new tool gets a contract entry, an ADR and tests.

## Sources

- Hermes Agent MCP docs: https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp
- Hermes MCP guide: https://hermes-agent.nousresearch.com/docs/guides/use-mcp-with-hermes
- OpenClaw MCP and skills: https://docs2.openclaw.ai/nodes/mcp-and-skills
- OpenClaw config (MCP, skills, plugins): https://docs2.openclaw.ai/gateway/config-extensions.md
