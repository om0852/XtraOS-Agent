# ADR 0042 — Phase 4 (part 1): audit export, Node and Python SDKs

- Status: accepted, built.
- Date: 2026-11-07

## Context
Phase 4 (docs/07) lists Linux, SDKs (Node/Python), team policies + audit export, and a skill
registry. What can be built and tested on this PC without a decision from the owner:
- **Audit export.** It is the "off-PC anchor" ADR 0038 names as the stronger answer to a
  rewritten log chain.
- **SDKs**, so scripts and other agents can use XtraOS directly.

Not here:
- **Team policies:** they need an org/admin model the owner has not specified.
- **Skill registry:** the owner wants it later, with connectors and plugins.
- **Linux:** needs a Linux machine.

## Decision
### 1. `audit_export {from?, to?, format?, out?}`
- The action log between two dates (local `YYYY-MM-DD`, inclusive; default: everything) as
  **CSV** (default) or **JSON**.
- Columns: seq, time (UTC ISO 8601), session, client, method, ok, code, ms, policy, hash, prev.
- The result reports how many entries were exported, the **chain check** (`verify_dir`) and
  the **head hash** (the last line's hash). Keeping the head hash elsewhere (an email, a
  ticket) makes a later rewrite of the whole log detectable.
- `out` defaults to `Documents\xtraos-audit-<from>_<to>.csv`.
  - The path passes the same protected-path check as the file skills (ADR 0026).
  - It never overwrites a file.
  - Policy class `file_change`.
- The log holds no request parameters, so the export contains no secrets (CLAUDE.md §5).

### 2. SDKs: thin clients of the same pipe protocol, no extra power
- **Node / TypeScript (`sdk/node`, package `@xtraos/sdk`, no runtime dependencies).**
  - `new XtraOS()` reads the pipe and token from `%LOCALAPPDATA%\XtraOS`.
  - It starts the installed core if needed: detached, no handles inherited.
  - `await x.call(tool, params)` returns the result or throws `XtraOSError {code, message,
    hint}`; `close()` ends it.
- **Python (`sdk/python`, package `xtraos`, standard library only).**
  - `XtraOS().call(tool, **params)`, with the same behaviour and `XtraOSError`; usable as a
    context manager.
- **Both** pair with the user's token, so policy, dialogs, the kill switch and the log apply
  as for MCP and the CLI. Overrides for tests are `XTRAOS_PIPE`, `XTRAOS_TOKEN` and
  `XTRAOS_HOME`, as in the core.

## Tests
- **Unit:** date range filter; CSV escaping; out-path refusal; no overwrite.
- **End-to-end:**
  - `audit_export` on a private log (count, chain OK, head hash, CSV/JSON readable);
  - the Node SDK and the Python SDK against a private core: `status`, `list_windows`, an
    error mapped to `XtraOSError` (`INVALID_ARGS`).

## Results
- AX-01: 3 entries; chain OK; head hash = the last line's; a comma in a field was quoted;
  a second export to the same file → INVALID_ARGS; `to: 2000-01-01` → 0 entries; output into
  the XtraOS data folder → BLOCKED_BY_POLICY.
- SDK-N1 (Node) and SDK-P1 (Python): status, list_windows, INVALID_ARGS → XtraOSError, and a
  wrong token refused.
- **Found while installing:** `xtraos-core call` started the core with the CLI's own output
  handle inherited, so a shell reading the CLI through a pipe waited until the core exited.
  Fixed: the CLI clears the inherit flag on its standard handles before starting the core.
  Verified: through a pipe the call returned at once.
- **Found with the first real export (the owner's own log):** the chain check reported a
  line as "changed" that nobody had edited. Its `"ms":0.013600000000000001` was re-parsed
  by serde_json's default float parser to a neighbouring value, so the re-serialised line,
  and its hash, differed.
  - Fix: serde_json `float_roundtrip` (a feature of the same pinned crate), with a test
    using 500 awkward float timings.
  - The owner's log now verifies: OK, 174 entries in 8 files.
- **Gate.** The first run was 17/17. The run after the float fix was 16/17: the
  browser-bridge suite (fake bridge) failed once. Re-run 4 times on its own, it passed 6/6
  each time. The cause is not identified (its failure output was not captured). It is
  noted as intermittent; watch it in the next gate runs.
- The release bundle was reinstalled on the owner's PC with the CLI and float fixes. The
  installed CLI starts the core through a pipe without hanging.
