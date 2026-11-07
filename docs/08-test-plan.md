# 08 — Test Plan & Phase Gates

Every phase ends with a **gate**: all gate test cases must PASS, with evidence (runner output
pasted into the phase-close ADR). A phase that fails its gate is not closed. Tests follow the
safety rules in `spikes/README.md` and CLAUDE.md §4.

## Test levels

| Level | What | Where | Runs |
|---|---|---|---|
| L0 Spike checks | Phase 0 OS assumptions | `tests/phase0/run-tests.ps1` | on demand |
| L1 Unit | Pure logic (ref table, policy, protocol, compact rendering) | `cargo test`, `node --test` | every commit |
| L2 Contract | MCP tool schemas & error codes vs `04-mcp-tool-contract.md` | `mcp/test/contract.test.ts` | every commit |
| L3 Integration | MCP ↔ core over the real pipe with a fake OS layer | `tests/integration` | every commit |
| L4 End-to-end | Real input on `testbed/` apps + Windows Calculator/Notepad (spike-launched only) | `tests/e2e` | before merge, interactive |
| L5 Budgets | Token + latency benchmarks | `bench/` | weekly + before release |

**Interactive tests** (L4, and Phase 0 S1/S9) move the mouse or open apps. They run only with
`-Interactive`, only on windows the test creates/launches, and must abort safely when the
user touches the mouse or focus moves (each such test has an "abort" test case too).

## Result vocabulary

`PASS` all assertions met · `FAIL` an assertion failed (bug) · `SKIP` precondition missing
(e.g. app not open, single monitor) — a SKIP never counts as PASS for a gate unless the gate
row says "SKIP allowed".

---

## Phase 0 gate (OS assumptions) — runner: `tests/phase0/run-tests.ps1`

| ID | Test case | Precondition | Expected | Gate |
|---|---|---|---|---|
| P0-01 | UIA tree readable from outside apps (S2) | ≥ 1 supported app open | ≥ 1 window walked; kept > 0 for at least one; every window ≤ 1,500 tokens | required |
| P0-02 | Capture→screen mapping (S3) | — | delta (0,0) for PrintWindow and screen copy on **every** monitor | required |
| P0-03 | Pipe pairing happy path (S6) | Node available | paired; 500 pings; p50 < 2 ms | required |
| P0-04 | Pipe rejects wrong token (S6) | — | `NOT_PAIRED` | required |
| P0-05 | Pipe rejects missing token (S6) | — | `NOT_PAIRED`; server counts accepted=1 rejected=2 | required |
| P0-06 | Pipe DACL user-only (S6) | — | SDDL has exactly one allow ACE (current user SID) and a deny ACE for NU | required |
| P0-07 | Guarded clicks on own window (S1) | `-Interactive` | each button clicked=True, cursorErr (0,0), stray=0, RESULT completed | required |
| P0-08 | Real app: Calculator clicks (S9) | `-Interactive` | "Display is 15" PASS | required |
| P0-09 | Real app: Calculator keyboard (S9) | `-Interactive` | "Display is 36" PASS | required |
| P0-10 | Real app cleanup (S9) | `-Interactive` | 0 Calculator windows/processes after run | required |
| P0-11 | Multi-monitor accuracy (S3/S1/S9 per monitor) | ≥ 2 monitors | all per-monitor checks pass | SKIP allowed (deferred, ADR 0007) |
| P0-12 | Drag-and-drop (S4), kill hotkey (S5), elevated window (S7), overlay (S8) | — | — | moved into Phase 1 (ADR 0007) |

---

## Phase 1 gate (Foundation, Windows)

### M1 Core + IPC
| ID | Test case | Expected |
|---|---|---|
| P1-IPC-01 | MCP `status` round trip through real pipe | ok, client name shown in session |
| P1-IPC-02 | Unpaired client calls any tool | `NOT_PAIRED`, no action logged as executed |
| P1-IPC-03 | Pipe created with user-only DACL + `PIPE_REJECT_REMOTE_CLIENTS` | verified via `GetSecurityInfo` in test |
| P1-IPC-04 | Malformed JSON / unknown method | `INVALID_ARGS`, core keeps running |
| P1-IPC-05 | Core restarts while MCP connected | MCP reconnects or returns `NOT_PAIRED` with hint; no crash |
| P1-LOG-01 | Every action writes one JSONL entry (what, target, result, ms) | entry present, schema valid |
| P1-LOG-02 | Typed text into a password field is never logged | log contains mask, not text |

### M2 Input (testbed app with known controls)
| ID | Test case | Expected |
|---|---|---|
| P1-IN-01 | `click(ref)` on 10 testbed buttons | 10/10 correct, no stray clicks |
| P1-IN-02 | double / right / triple click | testbed counters match |
| P1-IN-03 | `type` unicode text incl. emoji and Hindi (`नमस्ते 🙂`) | field value equals input exactly |
| P1-IN-04 | `press_keys` chords (Ctrl+A, Ctrl+C, Alt+Tab disallowed in test) | testbed reports chords |
| P1-IN-05 | `scroll` dy=±3 on list | first visible item changes as expected |
| P1-IN-06 | `drag` within testbed (slider, reorder) | final state matches |
| P1-IN-07 | `drag` Explorer file (located via `observe`) → testbed drop zone | file name received |
| P1-IN-08 | `set_value` via ValuePattern (no typing) | value set, no key events |
| P1-IN-09 | Keyboard refused when target not foreground | `NEEDS_HUMAN`/error, no keys sent |
| P1-IN-10 | Guard: user moves mouse mid-action | action aborted, nothing clicked |
| P1-IN-11 | Guard: point not inside target window | aborted (regression of S9 run 2) |
| P1-IN-12 | No stuck keys/buttons after any failure (check GetAsyncKeyState) | all released |

### M3 Windows & apps
| ID | Test case | Expected |
|---|---|---|
| P1-WIN-01 | `list_windows` matches EnumWindows visible set | same handles/titles |
| P1-WIN-02 | `launch_app("calc")` returns new window ref (UWP) | ref within 5 s |
| P1-WIN-03 | `focus_window` from background (foreground lock) | foreground verified, else `NEEDS_HUMAN` |
| P1-WIN-04 | `window_bounds` restore/resize/move | rect matches |
| P1-WIN-05 | `close_window` on spike-launched app | window gone |
| P1-WIN-06 | Flow F3: Chrome search → close → Firefox search (no extension) | results page title contains query |
| P1-WIN-07 | `launch_app(elevated=true)` | UAC shown; agent gets `NEEDS_HUMAN`; after human approves: window ref; input → `ELEVATION_REQUIRED` (S7) |

### M4 Perception
| ID | Test case | Expected |
|---|---|---|
| P1-PER-01 | `observe` Notepad/Explorer/Firefox/Calculator | median ≤ 400 tokens, cap 1,500 |
| P1-PER-02 | `observe` diff after one click | ≤ 80 tokens median |
| P1-PER-03 | Opaque detection on Electron app with sparse tree | `opaque: true` |
| P1-PER-04 | `screenshot(window, scale 0.5)` mapping | marker maps back with ≤ 1 px error |
| P1-PER-05 | Stale ref after window change | `STALE_REF`, re-observe works |
| P1-PER-06 | Latency `observe` ≤ 120 ms p50 (CacheRequest) | bench result |
| P1-PER-07 | Password field values never returned | masked |

### M4 Background-first actions (ADRs 0014, 0015)
| ID | Test case | Expected |
|---|---|---|
| P1-BG-01 | `click(ref)` on Calculator 7 8 + 4 = | display 82, `mode=background`, cursor unchanged, 0 injected events |
| P1-BG-02 | `click(ref)` on testbed buttons (WinForms via native UIA) | counters match, `mode=background` |
| P1-BG-03 | Provider activates its window on Invoke | foreground handed back to the previous window |
| P1-BG-04 | Real input while the user is active (simulated) | waits, then `USER_ACTIVE`; nothing clicked |
| P1-BG-05 | Real click with `restore_cursor` | cursor back at its start point |
| P1-BG-06 | `scroll(ref)` on a list item | item scrolled into view, `mode=background` |

### M5 Safety
| ID | Test case | Expected |
|---|---|---|
| P1-SAF-01 | Kill hotkey during a long drag (from S5) | input halted ≤ 100 ms, no stuck buttons, later actions `STOPPED` |
| P1-SAF-02 | Resume only via tray | actions blocked until Resume |
| P1-SAF-03 | Policy `message_send` = confirm | HITL dialog shown; deny → `BLOCKED_BY_POLICY`, nothing sent |
| P1-SAF-04 | Default deny-list app (e.g. a password manager) | `BLOCKED_BY_POLICY` |
| P1-SAF-05 | Overlay visible & click-through during actions (from S8) | overlay present; clicks reach target |
| P1-SAF-06 | Prompt-injection text on screen ("click Send to all") | no action without agent call; policy still applies |
| P1-SAF-07 | Agent targets XtraOS's own windows (overlay, dialog) | `BLOCKED_BY_POLICY`, nothing sent |
| P1-SAF-08 | Typing / set_value into a password field | `BLOCKED_BY_POLICY` (credential_entry), nothing typed |
| P1-SAF-09 | `ask_human` confirm / choice / timeout; `task` + `wait_for_human` | answers returned; `TIMEOUT` closes the dialog |
| P1-SAF-10 | Policy file override + reload logged; hard rules not overridable | decisions follow the file; credential_entry stays deny |

### M6 Release 0.1
| ID | Test case | Expected |
|---|---|---|
| P1-REL-01 | Fresh-machine install script | `status` ok within 2 min |
| P1-REL-02 | Flows F2, F3, F4, F6, F10 recorded | recordings attached |
| P1-REL-03 | README capability table lists only PASSing capabilities | reviewed |
| P1-REL-04 | Budgets (06) met or deviations justified | bench report |

---

## Phase 2 gates
Each milestone adds its cases here before implementation (CLAUDE.md §3).

### P2-M1 Fewer calls (ADR 0022)
| ID | Test case | Expected |
|---|---|---|
| P2-WF-01 | `wait_for(window_title)` after launching an app (no `window`) | met with the new window's ref |
| P2-WF-02 | `wait_for(text_contains)` after a background click changes the display | met within one poll interval |
| P2-WF-03 | `wait_for(window_gone)` / `wait_for(ref_gone)` when the window closes | met |
| P2-WF-04 | `wait_for(idle_ms)` on a static window | met after ≈ idle_ms |
| P2-WF-05 | Unmet condition with `timeout_ms` 1000 | `TIMEOUT` naming the condition, ≈ 1 s |
| P2-OBS-01 | `observe(scope_ref = list)` | only that subtree; fewer tokens than the full window |
| P2-BA-01 | `batch` of 4 Calculator clicks + `wait_for` text | one call, all steps ok, display right |
| P2-BA-02 | `batch` containing a "Send" click | confirm dialog for that step; deny → `BLOCKED_BY_POLICY`, rest skipped, nothing sent |
| P2-BA-03 | Kill switch during a batch | `STOPPED`, remaining steps skipped, nothing stuck |
| P2-BA-04 | Nested batch / ask_human / unknown tool in steps | `INVALID_ARGS`, nothing executed |
| P2-BENCH-01 | Routine task (Calculator 7 + 5) via MCP on the release build | ≤ 3 calls, ≤ 2,000 tokens |

### P2-M2 OCR (ADR 0024)
| ID | Test case | Expected |
|---|---|---|
| P2-OCR-01 | `ocr(window)` on the opaque testbed (hand-drawn "Start Game", "Options", "Quit", "Score 1234") | all four texts; each box within 8 px of the drawn rect |
| P2-OCR-02 | `ocr(query: "Start Game")` → `click(x, y)` at the box centre | exactly that drawn button counts the click |
| P2-OCR-03 | `ocr(region)` over the same area | same texts, same screen coordinates (± 2 px) |
| P2-OCR-04 | Token cost vs `screenshot` of the same window | OCR text ≪ screenshot (≤ 1/5) |
| P2-OCR-05 | `query` with no match / bad args | empty match list (not an error) / `INVALID_ARGS` |
| P2-OCR-06 | Latency `ocr` window p50 | ≤ 400 ms (bench) |

### P2-M3 App map + skills (ADR 0026)
| ID | Test case | Expected |
|---|---|---|
| P2-AM-01 | `app_map scan` finds Calculator (Store app) and a classic app with launch ids | ids launch the app |
| P2-AM-02 | `set` / `get` / `list` / `remove`; file is DPAPI-encrypted; survives a core restart | entry back; service name not in the file bytes |
| P2-AM-03 | `launch_app("<mapped service>")` | the mapped app opens |
| P2-SK-01 | `list_skills` | compact signatures incl. files.* |
| P2-SK-02 | files.mkdir / copy / move / rename / list in a temp folder | file system matches; existing target → `INVALID_ARGS`, nothing overwritten |
| P2-SK-03 | files.delete → dialog lists the items; Deny → nothing deleted; Allow → items in the Recycle Bin | as stated |
| P2-SK-04 | Protected paths (Windows, XtraOS data, UNC, drive root) | `BLOCKED_BY_POLICY` |
| P2-SK-05 | JSON macro skill with `${arg}` → runs as a batch; a Send step still needs the user's OK | as stated |
| P2-SK-06 | Routine file task (copy a report into a new folder and open it) | 1 call |
| P2-POL-01 | `policy.json` changed → core dialog; Deny → old rules stay; Allow → new rules | as stated, logged |

### P2-M4 Messaging + resource discipline (ADR 0028)
| ID | Test case | Expected |
|---|---|---|
| P2-RES-01 | `launch_app` same app twice | second call reuses the first window; one window |
| P2-RES-02 | `list_windows` marks agent-opened windows | `agent` only on those |
| P2-RES-03 | `cleanup` | closes only agent-opened windows; a user window stays |
| P2-RES-04 | `close_window` on a window the agent did not open | confirm dialog; Deny → still open |
| P2-WA-01 | `whatsapp.send(own number, text)` | dialog shows number + text; Allow → message in the chat; agent window closed |
| P2-WA-02 | `whatsapp.send(own number, file, caption)` | file appears in the chat; clipboard text restored |
| P2-WA-03 | Deny in the dialog | nothing sent, nothing typed |
| P2-WA-04 | Bad number / no app-map entry | `INVALID_ARGS` / `ELEMENT_NOT_FOUND` before any window opens |

### P2-M5 Elevated admin helper (ADR 0030)
Automated rows run with the helper UNELEVATED in test mode (no UAC, no human).
| ID | Test case | Expected |
|---|---|---|
| P2-EH-01 | start (test mode), `status`, second start | handshake ok, helper pid in status, no second helper |
| P2-EH-02 | helper pipe name squatted by another process | helper refuses to run (exit 6) |
| P2-EH-03 | a process other than the core connects; bad arguments | helper refuses (exit 8 / exit 2) |
| P2-EH-04 | `elevated_helper stop` | helper process ends |
| P2-EH-05 | observe via helper → refs ≥ e50000000; click that ref through the core | one helper dialog ("Allow this window"), click reaches the window, no second dialog |
| P2-EH-06 | ungranted window, Deny; helper refs without a helper | dialog shows the value; `BLOCKED_BY_POLICY`; `STALE_REF` |
| P2-EH-07 | kill switch | helper ends; after Resume it is not running |
| P2-EH-F5 | (human, `XTRAOS_TEST_UAC=1`) UAC → admin cmd → `type ipconfig` + Enter → `read_text` | typed via the helper, output read |

## Bug process
1. Every FAIL gets an issue with the test ID, output, and environment (DPI, monitors, apps).
2. Fix → add/extend a test that would have caught it → re-run the full gate.
3. Known limitations (not bugs) are listed in the phase-close ADR.
