# ADR 0009 — M1 core IPC decisions, and Smart App Control blocks unsigned dev builds

- Status: accepted. **M1 closed** 2026-11-07 (see "Resolution" at the end).
- Date: 2026-11-07

## Decisions made while building M1

1. **Wire protocol:** line-delimited JSON-RPC 2.0 over the pipe; errors carry string codes
   from the contract (`{"error":{"code":"NOT_PAIRED","message","hint"}}`). The first request
   must be `pair` with the token; anything else closes the connection. Planned-but-unbuilt
   methods return `NOT_IMPLEMENTED` (never fake success). Max line 1 MiB.
2. **Pipe security:** SDDL `D:P(D;;GA;;;NU)(A;;GA;;;<user SID>)`, `PIPE_REJECT_REMOTE_CLIENTS`,
   `FILE_FLAG_FIRST_PIPE_INSTANCE` (pipe-squatting makes the core refuse to start).
   Verified by reading the DACL back (`GetNamedSecurityInfoW`) in test P1-IPC-03.
3. **Pairing token storage — deviation from the spec (DPAPI):** the token is a 256-bit hex
   value in `%LOCALAPPDATA%\XtraOS\pairing.token` (user-only by profile ACL), not DPAPI.
   Reason: the MCP server (Node) must read it without a native module, and DPAPI adds no
   protection against the same user, who is the only principal that can read the folder.
   `docs/05-security-model.md` is updated accordingly.
4. **Discovery:** the core writes `connection.json` (`pipe`, `pid`, `version`); env overrides
   `XTRAOS_PIPE`, `XTRAOS_TOKEN`, `XTRAOS_HOME` exist for isolated tests.
5. **MCP server** runs TypeScript directly on Node 24 (type stripping, erasable syntax only);
   `tsc --noEmit` type-checks. Exposes only `status` in M1; a contract test fails if a tool
   is exposed that `docs/04-mcp-tool-contract.md` does not define.
6. **Pins:** tokio =1.54.1, serde =1.0.229, serde_json =1.0.154, @modelcontextprotocol/sdk
   1.32.2, typescript 7.0.2, @types/node 24.19.2, Rust 1.99.1.

## Evidence — M1 gate run 1 (`tests/phase1/m1-gate.ps1`)

| Step | Result |
|---|---|
| rustfmt --check | FAIL (formatting only) → fixed with `cargo fmt` |
| clippy, warnings = errors | PASS |
| cargo test: 12 unit + 9 IPC integration (P1-IPC-01..04, squatting, concurrency, P1-LOG-01, restart) | PASS |
| cargo build | PASS |
| tsc --noEmit | PASS |
| MCP: 3 contract + 4 e2e (status end-to-end, wrong token, core restart/reconnect P1-IPC-05, action log via binary) | PASS (7/7) |

## Blocker found in gate run 2

After `cargo fmt` changed the sources, the rebuilt test and core executables were refused:
`An Application Control policy has blocked this file. (os error 4551)`.
Read-only checks: `HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy\VerifiedAndReputablePolicyState = 1`
(Smart App Control **On**); 20 Code Integrity block events (IDs 3033/3077) in 30 minutes, all
for binaries we compiled. Smart App Control blocks unsigned executables without cloud
reputation; every rebuild has a new hash, so blocks are unpredictable.

Not done, by policy (CLAUDE.md §4 / owner's "no damage" rule): changing Smart App Control,
or rebuilding repeatedly hoping a hash slips through.

## Consequences
- **Development:** M1 gate cannot be re-run reliably on this machine until the owner chooses
  an option: (a) turn Smart App Control off themselves (developer machine; on some Windows 11
  versions it cannot be turned back on without resetting Windows — check the prompt),
  (b) develop/test inside Windows Sandbox or a VM (free; enabling Windows Sandbox is an
  optional feature + reboot), or (c) sign dev builds with a certificate Smart App Control
  trusts (normally paid).
- **Product (new requirement):** released binaries **must be code-signed** with a trusted
  certificate, or users with Smart App Control on cannot run XtraOS. Added to Phase 1 M6 /
  release checklist; free options for open-source projects (e.g. SignPath Foundation) to be
  evaluated in an ADR.

## Resolution (2026-11-07)

The owner turned Smart App Control off on this developer machine (option a).
Read-only check afterwards: `VerifiedAndReputablePolicyState = 0` (Off).

M1 gate after `cargo clean` (fresh binaries), run twice to rule out flaky tests:

| Step | Run 3 (clean build) | Run 4 (repeat) |
|---|---|---|
| rustfmt --check | PASS | PASS |
| clippy, warnings = errors | PASS | PASS |
| cargo test: 12 unit + 9 IPC integration | PASS | PASS |
| cargo build | PASS | PASS |
| tsc --noEmit | PASS | PASS |
| MCP: 3 contract + 4 e2e | PASS (7/7) | PASS (7/7) |
| **Gate** | **6/6 PASS** | **6/6 PASS** |

Test cases covered: P1-IPC-01, -02, -03 (DACL + squatting), -04 (malformed, unknown,
oversized), -05 (restart + reconnect), P1-LOG-01. P1-LOG-02 (password masking) needs typing
and moves to M2.

The release requirement stays: signed binaries for users who keep Smart App Control on (M6).
