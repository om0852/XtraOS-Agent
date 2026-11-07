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
