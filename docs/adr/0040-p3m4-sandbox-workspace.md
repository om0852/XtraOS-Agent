# ADR 0040 — P3-M4: a sandboxed desktop for the agent (Windows Sandbox)

- Status: accepted, built. The live test needs the owner to enable Windows Sandbox
  (admin + restart).
- Date: 2026-11-07

## Context
Real input (mouse, keyboard, touch) shares the user's cursor and foreground (ADR 0014).
The owner wants the agent to work **in parallel, with its own cursor**.

There are two options on Windows:
- **Microsoft's Agent Workspace.** It is a preview without a public API for third parties;
  re-evaluate at GA.
- **Windows Sandbox.** It is a disposable VM with its own desktop and is available on Windows
  11 Pro, which this PC runs.

Facts about this PC (2026-11-07):
- Windows 11 Pro, 16 GB RAM, hypervisor running;
- `WindowsSandbox.exe` is **not installed** (the optional feature is off).

## Decision
### Shape
- **One new host tool:** `sandbox {op: status | start | call | stop}`.
- **Inside the Sandbox,** a second XtraOS runs with its own desktop, cursor, keyboard and
  safety UI: `xtraos-core.exe --sandbox-agent C:\XtraOSExchange`.
- **The host core talks to it through one shared exchange folder,** with a request file and a
  response file per call. There is no network listener and no pipe across the VM boundary.
- **`start`:**
  - writes a `.wsb` configuration and opens it;
  - waits (up to 120 s) for the agent's heartbeat;
  - is class `launch`.
- **`.wsb` settings:**
  - mapped folders: the XtraOS program folder **read-only**
    (`C:\XtraOS`), the exchange folder read-write (`C:\XtraOSExchange`), and the session
    folder **read-only** (`C:\XtraOSSession`, with the per-start token);
  - Networking **Disable** by default (`network: true` to allow it);
  - clipboard, printer, audio input and video input redirection **Disable**;
  - ProtectedClient **Enable**.
- **`call {tool, args, timeout_ms}`:**
  - runs one tool inside the Sandbox and returns its result with `via: "sandbox"`;
  - inside, the Sandbox's own XtraOS applies its own policy, kill switch and dialogs
    (credential entry and purchases are still refused);
  - on the host it is class `observe`, because it touches nothing on the user's desktop.
- **`stop`:** asks the inner agent to shut the Sandbox down. Everything inside is discarded.
- `status` shows whether the feature is installed, whether a Sandbox is running, and
  whether its agent answers.

### Security
- **The inner agent refuses to run outside a Sandbox.** The account must be
  `WDAGUtilityAccount`. Otherwise a same-user process could start it to bypass pairing.
- **Test mode** (`--test-on-host`, for automated tests) allows only read-only tools
  (status, list_windows, observe, find, read_text, screenshot, ocr, frame_diff). `stop`
  then ends the test agent, never the PC.
- **The host treats everything from the Sandbox as untrusted data:**
  - responses are limited to 32 MB, parsed as JSON and never executed;
  - file names are the host's own request ids;
  - the token check stops stale or foreign requests from being answered.
- **Nothing from the user's profile is mapped.** Only what the agent needs goes in; files go
  in and out through the exchange folder.
- **Enabling Windows Sandbox is a system change the user makes**, not the agent. `start`
  without the feature returns `NEEDS_HUMAN` with the exact steps.

### Not doing
- Agent Workspace (preview, no API).
- Hyper-V VMs (heavier; Sandbox covers the need).
- Persistent sandboxes (Sandbox is disposable by design).

## Tests
- **Automated (no Sandbox needed):**
  - unit: `.wsb` generation (paths escaped, isolation settings);
  - the channel end to end with the inner agent in test mode on a temp exchange folder:
    `status` and `list_windows` through the channel; a wrong token is refused; an input tool
    is refused in test mode; `stop` ends the agent; a timeout when the agent is gone.
- **Human (later, after enabling Windows Sandbox):**
  - `sandbox start`;
  - `call launch_app notepad` + `type` inside the Sandbox while the user keeps working;
  - `call screenshot`;
  - `stop`.

## Results
- Unit: `.wsb` isolation and escaping; op classes.
- P3-SB-01..05, with the inner agent in test mode on this PC:
  - it refused to run outside a Sandbox (exit 3);
  - `list_windows` came back through the channel (`via: sandbox`);
  - `click` was refused in test mode;
  - a changed token was refused;
  - `stop` ended the agent;
  - `call` without a Sandbox → NEEDS_HUMAN;
  - `start` without the feature → NEEDS_HUMAN with the exact steps.
- Phase 3 gate (`-Automated`): 13/14 on the first run. `rustfmt --check` failed on the newly
  added live test; it was formatted and re-checked (fmt, clippy, the sandbox test).
- MCP: 36 tools.
- Open: P3-SB-LIVE after the owner enables Windows Sandbox.
- **Found with the first live start (owner's PC, Windows 11 26200):**
  - On its first start, Windows Sandbox spent more than 120 s on "Downloading and installing
    updates…". Then it failed with 0x800705b4 and fell back to "Classic Windows Sandbox", with a
    feedback question (answered No through XtraOS: UIA Invoke, no real input). `start` timed
    out meanwhile.
  - Defect: a second `start` while that Sandbox was still open would have wiped its mapped
    exchange folder, written a new token and launched a second Sandbox, which Windows refuses.
  - Fix: if a Windows Sandbox window is already open, `start` never resets anything.
    - If this core launched a Sandbox in the last 30 minutes, it only waits for the agent
      inside. The timeout hint says the first start can take minutes and that `start` again
      keeps waiting.
    - Otherwise (a Sandbox the user opened) it returns NEEDS_HUMAN: close it first. This came
      from the Codex review of the first version, which waited on any open Sandbox.
- **Shared folders under `%LOCALAPPDATA%\XtraOS` failed (0x80070003, "cannot find the path").**
  The same settings without shared folders, and a plain Sandbox, booted fine. Folders in the
  user profile and in `%LOCALAPPDATA%\Temp` shared fine. The exact cause was not found (the
  permissions are alike).
  - Fix: the shared folders are `%USERPROFILE%\.xtraos-sandbox\{program, exchange, session}`.
    `start` copies the core's own exe into `program`, because the install folder is under
    AppData. The new folder is protected from the file skills like the data folder.
- **The program inside did not start (live).** Two causes:
  - The core needed `VCRUNTIME140.dll`, which a clean Windows (the Sandbox) does not have.
    Fix: the C runtime is linked statically (`core/.cargo/config.toml`). This also helps
    installs on PCs without the Visual C++ redistributable.
  - In the live test the core runs inside the test binary, so "copy my own exe" copied the
    test harness ("Unrecognized option: 'sandbox-agent'"). Test override:
    `XTRAOS_SANDBOX_PROGRAM`.
  - The agent inside now writes `agent.txt` in the exchange folder (started, or why it
    stopped). On a timeout, `start` quotes it, or says the program never started.
- **Pipe hang.** `WindowsSandbox.exe` inherited the caller's standard handles and outlives the
  call, so a shell or gate reading XtraOS through a pipe waited until the Sandbox closed. Fix:
  it is started with no standard handles.
- **Slow channel (10–20 s per call).** Inside the Sandbox, a listing of the shared folder
  showed a new request file only 10–20 s late. Traced from the host: the request sat 10.7 s
  before the agent took it. Setting the SMB cache lifetimes to 0 inside did not help.
  - Fix: requests are numbered 1, 2, 3… The counter lives in the session folder and is reset
    at `start`. The agent opens the next number by name and lists the folder only every 2 s,
    to resync.
  - Codex review: two calls at the same time could become visible out of order; the agent then
    moves past the lower number. That request is still picked up by the 2 s listing (bounded
    delay, no stall). Kept on purpose: never waiting on a missing number also covers a request
    the host already gave up on.
- **The Sandbox has no Notepad** (a Store app on Windows 11). Windows inside does not let a
  window come to the front while the Sandbox window is in the background on the user's
  desktop. So `type` returns NEEDS_HUMAN there; text goes in through `set_value`.
  - The live test uses Character Map: `find` the "Characters to copy" box, then `set_value`,
    `read_text`, a window `screenshot`, `stop`.
- **Test fixes:** the channel test's env overrides leaked into the live test (now cleared);
  the test client's reply timeout was raised to 150 s (above `start`'s 120 s).
- P3-SB-LIVE: PASS on the owner's PC, twice in a row (79 s each, including the Sandbox boot).

### Full interactive gate (2026-11-08)

The first full gate with Windows Sandbox on hung twice and turned up these; all fixed:

- **A finished Sandbox kept the gate open.** After the Sandbox inside shuts down,
  WindowsSandbox.exe stays running without a window (14 of them had piled up). It also held
  the output pipe of whoever started it, because `Stdio::null()` does not stop Windows from
  handing every inheritable handle to the child. Now: the CLI's "keep my console/pipe
  handles private" step is shared (`cli::keep_std_handles_private`) and runs before the
  Sandbox launch; `stop` ends the WindowsSandbox.exe this core started (only that one) once
  its window has closed; the gate writes each step's output to a file instead of a pipe.
- **A fast test booted a real Sandbox.** The channel test called `start` to check the
  not-installed answer; with the feature on, it launched a real Sandbox. It now calls
  `start` only when the feature is missing (the live test covers the rest).
- **`stop` waited only 15 s.** The shared folder sometimes shows a request 10-20 s late;
  `stop` now waits 60 s like every other call.
- **Dead approval dialogs (safety, not Sandbox).** A dialog closed by timeout or by the kill
  switch was looked up by its prompt id on the UI thread after the caller had already
  forgotten that id, so the window stayed on screen and did nothing. The close request now
  carries the window itself, and only an XtraOS dialog window is destroyed. Found by
  P2-M4 WA-03, which now names any new window instead of comparing counts.
- **Test fixes:** F5 ran the core inside the test binary, so UAC elevated the test binary
  and the helper (which serves only a core with its own image) could never connect; F5 now
  starts the real xtraos-core.exe (`Core::start_process`), and a test core process always
  ends, even when the test fails (a leaked one kept the kill switch hotkey and broke every
  later kill switch test). Flow F10 posted the tray "Resume" command, which ADR 0041 rightly
  ignores; it now checks that a posted Resume does nothing.
- **Result:** every automated and hands-off step passes (unit, release bundle, install test,
  P2-M1..M4, safety, security, live Sandbox 93 s with nothing left running, flows 8/8,
  bench 5/6 — typing 50 characters is above its budget, as before). Left for the human
  pass: F5 (UAC, then the helper's own Allow dialog) and resuming from the tray.
