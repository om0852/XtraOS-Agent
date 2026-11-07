# ADR 0039 — Command-line bridge, system controls, media keys

- Status: accepted, built and tested.
- Date: 2026-11-07

## Context
The owner's instructions (2026-11-07):
- Use XtraOS as the bridge for every task, with no ad-hoc scripts; anything missing goes into
  XtraOS.
- XtraOS must work with any LLM.
- Add the common system controls ("volume, …, for Windows, Mac, Linux").
- Stay inside XtraOS's scope and do not build unnecessary things.

MCP already serves any MCP-capable LLM. What was missing:
- a way for any shell-capable agent to call XtraOS without MCP;
- the system controls themselves. The owner's first request, lowering the system volume by
  10, had to go around XtraOS.

## Decision
1. **`xtraos-core call <tool> [params-json | -] [--json] [--out file.png]`**
   - It reads the pipe and the user-only pairing token from the data folder, as the MCP
     server does, and starts the core hidden if it is not running.
   - It calls exactly one tool. Policy, dialogs, kill switch and log apply as for MCP: no
     extra power.
   - Params can come from stdin (`-`) to avoid shell quoting.
   - Images are saved to a file. Exit code 0 = ok, 1 = error, 2 = usage.
   - Release builds attach to the caller's console so output shows in terminals.
2. **`volume {op: get|set|change|mute|unmute}`**: Core Audio IAudioEndpointVolume of the
   default speakers. Exact, background, 0–100 like the Windows flyout.
3. **Media keys for `press_keys`:** VolumeUp/Down/Mute, PlayPause, NextTrack, PrevTrack,
   MediaStop, sent as extended keys.
4. **`system {op}`**
   - Read: `battery` (GetSystemPowerStatus), `brightness` (WMI WmiMonitorBrightness),
     `wifi` / `bluetooth` (Windows.Devices.Radios).
   - Change:
     - `brightness value` (class input);
     - `wifi|bluetooth state on|off`, `lock`, `sleep` (class `system_settings`, so the user
       is asked).
   - **Not offered, by design:** shutdown/restart/sign-out, firewall/antivirus, accounts,
     network configuration, registry. They are out of scope and could lock the user out or
     weaken security.
5. **macOS / Linux.** The tool contract is platform-neutral, so a macOS or Linux core
   implements the same `volume` / `system` ops: CoreAudio and IOKit on macOS; PulseAudio/
   PipeWire, sysfs backlight and rfkill/NetworkManager on Linux. Not built here: that needs
   those machines to build and test (docs/07 P3-M5, Phase 4).

## Results
- Unit: volume targets and clamping, system op classes (lock/sleep/radio switches ask;
  shutdown refused), base64 round trip.
- Live through the CLI on this PC:
  - `volume get` → 48 (after the owner's −10);
  - battery 100 % plugged in; brightness 100; set 100 → 100;
  - Wi-Fi on; Bluetooth off;
  - `system shutdown` → INVALID_ARGS.
  - The radio switches, lock and sleep were not run: they would disconnect or lock the
    owner, and they belong in the hands-off session.
- MCP: 35 tools.
