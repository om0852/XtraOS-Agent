# ADR 0044 — Phase 5 "Connect anywhere": scope, and P5-M7 lean tool surface

- Status: accepted (owner: "Yes, start Phase 5", 2026-11-10). P5-M7 built; the rest open.
- Date: 2026-11-10

## Context
`docs/09-next-phases-plan.md` proposes four phases after the current work. The owner chose to
start with Phase 5: make XtraOS easy to install and usable from any AI tool (IDE agents and
harnesses such as Hermes Agent and OpenClaw).

Measured today (37 tools): `tools/list` is 33,884 characters, about **8,500 tokens**, sent to
the model in every session. There are no server `instructions`.

## Decision
### 1. Phase 5 milestones
| ID | Work | Needs from the owner |
|---|---|---|
| P5-M1 | Prebuilt release (GitHub Release zip + checksums, winget, `npx` bootstrap) | OK to publish a Release |
| P5-M2 | Code signing | SignPath Foundation or a certificate |
| P5-M3 | `xtraos setup` (writes each client's MCP entry after showing it) and `xtraos doctor` | — |
| P5-M4 | SDKs on npm and PyPI | npm / PyPI accounts |
| P5-M5 | Verified WSL path for Hermes Agent and OpenClaw | — |
| P5-M6 | `SKILL.md` package around the CLI bridge | — |
| P5-M7 | Lean tool surface: server instructions + tool profiles | — (this ADR) |
| P5-M8 | Opt-in loopback HTTP MCP endpoint | **Decision pending**: CLAUDE.md §4 says local IPC only. Not built until the owner decides and a separate ADR lands. |
| P5-M9 | Integration guide + compatibility matrix (verified rows only) | — |

### 2. P5-M7: server instructions
The MCP `initialize` result carries `instructions`: a short text (under 200 words) that tells
the model how to work with XtraOS: list or launch a window, `observe` it, act on `ref`s
(background, no cursor), use `find` / `read_text` / `wait_for` instead of screenshots, what
the common error codes mean, and that the user's approval dialogs and kill switch cannot be
bypassed. It adds no capability.

### 3. P5-M7: tool profiles
The environment variable `XTRAOS_TOOLS` selects what `tools/list` returns:

- `full` (default, unchanged): all 37 tools.
- `core`: 15 tools for everyday work — `status`, `list_windows`, `launch_app`,
  `focus_window`, `observe`, `find`, `read_text`, `screenshot`, `click`, `type`, `press_keys`,
  `scroll`, `wait_for`, `list_skills`, `run_skill`.

Rules:
- A profile only changes the **list**. Every tool stays callable by name, and every call still
  passes the core's policy engine, approvals, kill switch and log. A profile is a token
  saving, not a security boundary.
- An unknown value falls back to `full` with a warning on stderr.
- `full` stays the default so existing setups do not change. `xtraos setup` (P5-M3) will offer
  `core` for clients with small context windows.

### 4. Exit criterion, corrected by measurement
The plan said "`core` ≤ 2,500 tokens". The 15 tools above measure about **3,400 tokens**
(4 characters per token). Dropping `scroll`, `screenshot` or `focus_window` to reach 2,500
would break everyday tasks. The criterion is therefore: **`core` ≤ 3,500 tokens** (a 60%
cut from 8,500), enforced by a contract test (≤ 14,000 characters).

## Consequences
- No tool signature changes; `docs/04-mcp-tool-contract.md` gains a "Profiles and
  instructions" section.
- New tests: `core` lists exactly the 15 tools and stays under the size limit; an unlisted tool
  is still callable; `initialize` returns instructions; a bad profile value falls back.
- Shorter tool descriptions (to cut `full` as well) are left for later; they need care because
  descriptions are the model's only manual.
