# ADR 0071 — Distribution: release pipeline, winget, npx installer, signing, SDK packages

- Status: accepted (owner: "implement it", 2026-11-11; plan rows P5-M1, P5-M2, P5-M4).
- Date: 2026-11-11

## Context
Users must build XtraOS with Rust today. The binaries are unsigned (Smart App Control blocks
them). The SDKs exist but are not packaged for the registries.

## Decision
1. **CI** (`.github/workflows/ci.yml`, every push and pull request, Windows runner): core unit
   tests, MCP typecheck and tests.
2. **Release** (`.github/workflows/release.yml`, on a `v*` tag or by hand): runs the tests, builds
   the bundle with `release/build-release.ps1`, signs it when signing secrets exist, makes the
   winget manifests and uploads everything to a **draft** GitHub Release. A draft is not public:
   the owner reviews it and presses Publish.
3. **winget** (`release/winget/make-manifests.ps1`): manifests (schema 1.6) for the release zip:
   zip with a nested portable `xtraos-core.exe` (alias `xtraos-core`), checked with
   `winget validate`. Limitation: winget's portable install puts the core and its command-line
   bridge on PATH; connecting MCP clients still uses `xtraos setup` from the installed folder.
   Submitting to the winget-pkgs repository is a separate, public step for the owner.
4. **npx installer** (`release/npx`, package `xtraos-install`): `npx xtraos-install` downloads the
   release zip and `SHA256SUMS.txt` from the GitHub Release, verifies the hash, extracts it and
   runs `install.ps1`. Windows only. `--from <zip>` installs a local zip; `--dry-run` stops after
   verifying. No other sources: it only reads from this repository's releases.
5. **Signing** (`release/sign.ps1`, called by the build): signtool with SHA-256 and an RFC 3161
   timestamp, from a certificate thumbprint (`-CertThumbprint`) or a PFX (`-PfxPath`, password
   in `XTRAOS_SIGN_PFX_PASSWORD`). Signs XtraOS's own executables only (Node stays signed by the
   OpenJS Foundation). Without a certificate the build says "not signed". Getting a certificate
   (SignPath Foundation for open source, or a purchased one) is the owner's step.
6. **SDK packages**: `@xtraos/sdk` (npm) and `xtraos` (PyPI) get licence, repository and
   description metadata and are no longer marked private; `publish-sdks.yml` publishes them only
   when the owner runs it (npm token secret; PyPI trusted publishing configured by the owner).
   Checked locally with `npm pack --dry-run`.

Nothing is published by this change; every public step stays with the owner.

## Tests
- `release/npx` unit tests: checksum parsing and verification, mismatch refused, local zip dry run.
- `winget validate` on generated manifests; `npm pack --dry-run` for the SDK; build script run
  with and without a certificate (without: not signed, bundle built).
