# ADR 0050 — System health guard: readings, runaway warnings, and a brake on the agent's own excess

- Status: accepted (owner, 2026-11-10: "add a functionality to track the RAM use, temperature,
  CPU use, GPU use, Wi-Fi … if a harness opens browser tabs 20 times, 100 times, it should get
  notified … so our agents also prevent the things which can cause the system to damage. Also
  unwanted files, system related critical files").
- Date: 2026-11-10

## Context
An agent harness can loop: open the same page 100 times, launch an app again and again, or
keep a heavy page busy. The agent does not see that the PC is running out of memory, getting hot
or filling its disk; the user notices only when the PC freezes. XtraOS sits between agents and
the PC (every action passes the core), so it can watch and speak up.

Spike, 2026-11-10 (user account, no admin), all readable:

| Reading | Source (no admin) | Seen |
|---|---|---|
| CPU % | `GetSystemTimes` (two samples) / PDH `\Processor(_Total)\% Processor Time` | 12 % |
| RAM | `GlobalMemoryStatusEx` | 11.9 / 15.8 GB |
| GPU % | PDH `\GPU Engine(*engtype_3D)\Utilization Percentage` (sum) | 5 % |
| Temperatures | PDH `\Thermal Zone Information(*)\Temperature` (kelvin) | CPU zone 74.9 °C, battery 38.9 °C; unused zones report ≈ 0 K → dropped |
| Temperatures (ACPI WMI) | `MSAcpi_ThermalZoneTemperature` | access denied without admin → not used |
| Disk free | `GetDiskFreeSpaceExW` | C: 61.6 GB |
| Wi-Fi | WLAN API current connection, signal quality | 98 % |
| Memory per app | process list, working set grouped by name | Firefox ×13 2.0 GB, Chrome ×18 0.9 GB |

Temperature zones are whatever the laptop's firmware exposes; some PCs have none. That is
reported as "not available", never guessed.

Files: agent file skills already refuse protected places (ADR 0026: Windows folder, Program
Files, ProgramData, other users, drive roots, network paths, XtraOS's own files) and deletes go to
the Recycle Bin after asking. Gap: hidden system folders at a drive root (`$Recycle.Bin`,
`System Volume Information`, `Recovery`, `Boot`, `EFI`) are only protected as "the root" itself,
not below it.

## Decision
1. **`system {op: "health"}`** (read-only, background, no policy prompt): `cpu_pct`, `ram`
   {used_gb, total_gb, pct}, `gpu_pct`, `temps` [{zone, c}] (zones ≤ 0 °C dropped), `disks`
   [{drive, free_gb, pct_free}], `wifi` {signal_pct} (no network name: Windows treats it as
   location data), `battery` (as op battery), `top` = the 5 apps using most memory
   {app, processes, mb}, and `warnings`. Not in the core profile (the warnings reach every
   agent anyway, item 3).
2. **Watchdog** in the core, every 5 s, cheap readings only (CPU, RAM, disk, temperatures,
   process counts). Warning levels:
   | Check | warn | critical |
   |---|---|---|
   | RAM in use | ≥ 90 % | ≥ 95 % |
   | CPU | ≥ 95 % for 30 s | — |
   | Hottest zone | ≥ 90 °C | ≥ 98 °C |
   | System drive free | < 5 % or < 5 GB | < 1 GB |
   | Windows opened by the agent (launch_app, browser open_tab) | ≥ 15 in 10 min | ≥ 30 in 10 min |
   | One app's process count since the session's first action | +40 | +100 |
   The last two catch a harness that keeps opening tabs or windows even outside XtraOS's tools
   (a browser makes a process per tab or site).
3. **Agents are told.** While a warning is active, every tool result carries
   `health_warning: "RAM 93 % (Chrome ×120, 7.9 GB) — you may be opening too much"` (the MCP server
   adds one `⚠` line); `status` has `health` {cpu_pct, ram_pct, warnings}. The user gets the
   same text in the XtraOS banner, at most once a minute per warning.
4. **A brake on the agent's own excess.** At a critical level, actions that open more —
   `launch_app`, `browser` open_tab / navigate in a new tab, `run_skill files.open` — ask the
   user first ("The agent opened 31 windows in 10 minutes and RAM is at 96 %. Allow it to open
   more?"); a no → `BLOCKED_BY_POLICY`. Reading and closing are never blocked, and `cleanup`
   (closes only windows the agent opened) is named in the hint. XtraOS never kills processes or
   closes the user's own windows on its own.
5. **Files:** `protected_reason` also refuses `$Recycle.Bin`, `System Volume Information`,
   `Recovery`, `Boot`, `EFI` and `pagefile.sys`/`hiberfil.sys`/`swapfile.sys` on any drive.

## Not doing
- Kernel drivers or admin-only sensors (CPU package temperature via MSR, fan speeds).
- Killing processes, freeing memory, changing power plans (destructive or a system setting).
- Network names, browsing history or anything personal in the readings.

## Build notes (2026-11-10)
- `health.rs`: raw Win32 declarations (kernel32, pdh, wlanapi), like `winmgr.rs`. Wi-Fi signal
  quality is read at byte 576 of `WLAN_CONNECTION_ATTRIBUTES` (520 bytes of state/mode/profile
  name, then SSID 36, BSS type 4, BSSID 6 + 2 padding, PHY type 4, PHY index 4).
- Zone names get plain words (`\_TZ.CPUZ` → cpu, BATZ → battery, CHGZ → charger, EXTZ → case,
  LOCZ → surface); unknown ones keep the firmware's short name.
- What the agent opened is counted where the brake sees it (`safety::authorize` → `brake`), for
  every opening action it lets through; reused windows count too (a reuse loop is still a loop).
- The warning reaches agents through `ipc` (every result object) and the MCP server (one `⚠`
  line); `status` shows CPU/RAM, the dialog line (P6-M4, until now only in JSON) and warnings.
- Test-only environment overrides: `XTRAOS_HEALTH_OPENED_WARN/CRIT`, `XTRAOS_HEALTH_GROWTH_WARN/CRIT`
  and `XTRAOS_HEALTH_PROMPT_MS` (how long the prompt waits; nothing can answer it for the user).
- **All XtraOS prompts:** an answer other than the safe one (e.g. "Allow") now counts only
  after the prompt has been open 0.8 s (`safety::answer_counts`), on top of the existing
  real-input rule (ADR 0041), so a click aimed at something else cannot approve a prompt that
  just popped up under the cursor. During the regression runs the brake prompt was answered
  "Allow" by a real mouse click several times while the owner used the PC; unanswered runs
  blocked as designed.
- Live run on the owner's PC: CPU 14 %, RAM 11.5/15.8 GB, GPU 6 %, Wi-Fi 83 %, battery 100 %
  plugged in, cpu zone 79 °C, C: 61 of 475 GB free, top memory Firefox ×13 1.9 GB.

## Tests
- Built: unit `a_healthy_pc_has_no_warnings`, `levels_follow_the_adr_table`,
  `cpu_percent_from_two_samples`, `only_opening_actions_are_braked`, `zone_names_are_plain`,
  `live_readings_work_without_admin`, protected root folders in `paths_normalise_and_protect`;
  interactive `health01_readings_warning_and_brake` (passes).
- Proposed originally:
- Unit: warning levels (pure function over a reading), the opened-windows window (10 min), the
  per-app process growth, protected root folders.
- Live: `system health` on this PC returns RAM/CPU/GPU/temps/disk/Wi-Fi; an interactive test
  launches the testbed repeatedly with lowered limits (test-only env override) and checks the
  `health_warning` and the ask at the critical level (answered "no" by the test's dialog
  handling, or skipped when no human).
