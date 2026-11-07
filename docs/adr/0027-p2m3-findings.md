# ADR 0027 — P2-M3 App map + skills: findings and gate result

- Status: accepted. **P2-M3 closed** 2026-11-07 (owner-confirmed hands-off gate run).
- Date: 2026-11-07

## What was built (ADR 0026)
- `appmap.rs`: Apps-folder scan (classic + Store apps → `shell:AppsFolder\…` launch ids),
  `set/get/list/remove`, DPAPI-encrypted per-user file; `launch_app` resolves mapped services.
- `skills.rs`: built-in file skills through `IFileOperation` on an STA thread (undoable,
  never overwriting, deletes to the Recycle Bin), protected paths (Windows, Program Files,
  ProgramData, other users, UNC, drive roots, XtraOS install + data; `..` and junctions
  resolved first), JSON macro skills (`${arg}`, run as a batch, policy per step; built-in
  skills allowed as steps, no macro recursion).
- Policy: class `file_change` (allow); **a changed `policy.json` applies only after the user
  approves the listed changes in a core dialog** (unchanged rules apply silently).
- MCP: `app_map`, `list_skills`, `run_skill` (26 tools). Flow S1. Gate `tests/phase2/p2m3-gate.ps1`.

## Findings (fixed)
1. **Rejected policy changes were not logged** — the note travelled with the action's approval,
   and the action itself was blocked. Reload decisions now go through a queue the IPC layer
   always writes to the action log.
2. **Overlay hid while an action was still running** (it hid 2.5 s after the action *started*;
   an action waiting for the user to be idle outlived it). Now it stays up for the whole action
   and hides 2.5 s after it ends.
3. **A resting hand / noisy mouse blocked the agent forever.** Sampling during "hands-off"
   runs showed the cursor drifting 1–4 px (9 moves in 3 s) with no software injecting input —
   and `GetLastInputInfo` counts every such event as the user working, so polite real input
   waited until its timeout and failed. **Fix:** low-level keyboard + mouse hooks on the safety
   thread classify input: the core's own injected events are ignored; keys, buttons and wheel
   are always the user; mouse movement counts only beyond 6 px. During a click, drift ≤ 6 px
   re-aims instead of aborting; more is still `USER_ACTIVE`. Falls back to
   `GetLastInputInfo` if the hooks cannot be installed. This also explains the intermittent
   "user active" seen in earlier milestones (ADR 0016 f4, 0018 f1, 0020 f9).
4. Policy-change dialog also covers the earlier saf04 test path (the user now approves reloads).

## Resolution (2026-11-07)
`tests/phase2/p2m3-gate.ps1 -Interactive` after an automatic idle check (no movement beyond
jitter): **15/16** — every interactive step PASS:

| Step | Result |
|---|---|
| fmt / clippy / 55 unit + 9 IPC / build / tsc / 10 MCP / bundle / release build | PASS |
| install test (REL-01) | 12/12, 26 tools, 5.2 s to a working status |
| P2-M1 e2e / P2-M2 OCR e2e | 7/7 / 3/3 |
| **P2-M3 app map + skills e2e** | **6/6** (scan + launch via map; DPAPI file without readable URL, survives restart; mkdir/copy/move/rename/list, no overwrite; delete dialog → deny keeps, allow → Recycle Bin (verified, test item removed from the bin); protected paths blocked without a dialog; macro skill with a Send step still needs the user; routine file task in one call; macro-in-macro refused) |
| M5 safety e2e incl. policy approval | 6/6 (kill switch, Send confirm, policy change kept/applied + both logged, overlay, own windows, passwords) |
| flows | **8/8** incl. **S1** (folder + copy + open in one `run_skill`) |
| bench | pipe 0.1, observe 28.9, click 5.5, screenshot 17.7, ocr 33.3 ms (met); type 50 chars 317 ms (app-bound, ADR 0020) |
| testbed build | FAIL: `testbed.exe` locked by a test window left over from an earlier manual run of mine; closed it, rebuilt OK (not a product issue) |

## Open
Messaging skills need an owner-chosen test chat (P2-M4).
