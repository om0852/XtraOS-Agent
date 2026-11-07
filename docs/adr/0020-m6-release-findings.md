# ADR 0020 — M6 Release 0.1: findings and gate result

- Status: accepted. **M6 closed as an unsigned developer preview** 2026-11-07. Open (owner
  decisions, ADR 0019): code signing and a test on a truly fresh machine with Smart App
  Control on.
- Date: 2026-11-07

## What was built (ADR 0019)
- `release/build-release.ps1` → `dist/xtraos-0.1.0-win-x64.zip` (**33.6 MB, 10 files**) +
  `SHA256SUMS.txt`: release core (windowless), MCP server bundled into one file with
  esbuild (`esbuild` 0.28.2, dev dependency only), official Node 24.18.0 (OpenJS-signed,
  hash recorded), `xtraos-mcp.cmd`, install/uninstall scripts, INSTALL.md, THIRD-PARTY.md
  (generated from `cargo metadata` + `npm ls`). `-CertThumbprint` signs with signtool.
- `release/install.ps1` / `uninstall.ps1` (per user, no admin, opt-in login entry and Claude
  Code registration, never touches security settings); `release/test-install.mjs` (REL-01).
- MCP server starts the core on demand (detached, hidden); core tray "Quit" for clean exits.
- `tests/flows/run-flows.mjs` → `docs/flows/` (REL-02); `tests/bench/bench.mjs` →
  `docs/bench/release-0.1.md` (REL-04); gate `tests/phase1/m6-gate.ps1`.

## Findings (all fixed unless noted)
1. **Install took 115–122 s** (3,675 files, mostly `node_modules`, extracted and copied one by
   one). Bundling the MCP server into one file: 10 files, **install → working `status` in
   5.9–13 s**, faster MCP start too.
2. **Empty `XTRAOS_TOKEN` was accepted as the pairing token** (an empty or short override
   made pairing guessable). The core now ignores an empty override and refuses anything that
   is not a full 256-bit token.
3. **Uninstall failed while an AI client had the MCP server open** (`node.exe` locked). It now
   stops MCP servers started from the install folder.
4. **The tray "Quit"/"Resume" commands never reached the core from PowerShell**: `$null`
   passed as a .NET `string` becomes `""`, so `FindWindowEx` looked for an untitled window;
   then `[UIntPtr]1003` is not a valid cast in PowerShell 5.1. Fixed (`[NullString]::Value`,
   `[UIntPtr]::new(...)`); the install test now requires the graceful path ("closed via its
   tray Quit command"), no forced kill.
5. **`type` returned while keystrokes were still queued** — a following observe/read could see
   half the text, and a browser could search a partial query (bench + flow F3). `type` now
   sends 8-character chunks paced by `WaitForInputIdle`, then waits until the focused field's
   value stops changing (≤ 2 s) **before Enter and before returning**; the result reports
   `settled`. Verified: read-back exact after every call (bench), M2 input e2e 11/11.
   Consequence: `type` time is now app-bound (see Budgets).
6. **`read_text` on Windows Terminal returned only the title**: the terminal exposes its
   buffer via TextPattern on a non-Document element. read_text now picks the largest
   element with a TextPattern of any role (flow F6 reads `v24.18.0`).
7. **Paths with spaces** broke `cmd /s /c "<launcher>"` (outer quotes stripped); the
   recorder double-quotes. (Client configs call the `.cmd` directly and are not affected.)
8. **Flow robustness**: Explorer opened behind other windows → drag refused with
   `TARGET_OBSCURED` (the guard working); the recorder now focuses both windows like an agent
   should. Browser result pages are network-bound: the recorder re-checks the title (≤ 8 s)
   instead of one fixed wait.
9. **"User active" during hands-off runs explained**: sampling showed the real cursor moving
   (169 input events in 12 s, cursor jumping between positions) while runs were in progress,
   with the core's own injection counter at zero. Polite input correctly refused to type then
   (closes ADR 0016 finding 4 / ADR 0018 finding 1: user/system input, not the agent).

## Resolution (2026-11-07)
`tests/phase1/m6-gate.ps1 -Interactive`: all non-interactive steps PASS (fmt, clippy, unit +
IPC, build, tsc, 10 MCP tests, testbed, bundle, release build, **install test 12/12**), bench
PASS (script), flows: 4/5 in that gate run (F3: Chrome results page slower than the fixed
3.5 s wait); after finding 8's re-check: **5/5 in three consecutive full runs**.

| Flow (`docs/flows/`) | Result | Calls | ~Tokens |
|---|---|---|---|
| F2 observe → act (Calculator, background) | PASS | 8 | 340 |
| F3 Chrome search → close → Firefox search → close | PASS | ≥ 10 | ~160 |
| F4 Explorer file → web drop zone (refs, real drag) | PASS | 13 | ~320 |
| F6 terminal command + read output (`node --version`; Claude Code not installed) | PASS | 5 | ~220 |
| F10 kill switch mid-drag → STOPPED → tray resume | PASS | 8 | ~160 |

## Budgets (P1-REL-04, `docs/bench/release-0.1.md`)
Met: pipe 0.2 ms, observe Calculator 62–102 ms, click(ref) 9–32 ms, kill switch 15 ms.
Deviations, stated: `type` 50 chars 0.7–0.9 s p50 *until the app has the text* (app-bound:
the WinForms field takes ~13 ms/char; 150 ms budget kept for fast fields, Phase 2 re-measure);
screenshot 52–84 ms vs 60 ms (p95 within 150); observe on huge lists (ADR 0016).

## Open
Code signing (licence + SignPath, or the owner's certificate); fresh machine with Smart App
Control on; F6 with real Claude Code once installed; `type` budget on more apps.
