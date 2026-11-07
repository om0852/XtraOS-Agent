# ADR 0008 — Phase 1 toolchain installed and pinned

- Status: accepted
- Date: 2026-11-07

## Context
Phase 1 builds `xtraos-core` in Rust (ADR 0001). ADR 0002 deferred the install until the owner
approved it; the owner approved on 2026-11-07. All tools are free.

## Decision
| Tool | Version | How installed | Scope |
|---|---|---|---|
| Visual Studio Build Tools 2022 (C++ workload `Microsoft.VisualStudio.Workload.VCTools` + recommended Windows SDK) | 17.14.37827.9 | `winget install Microsoft.VisualStudio.2022.BuildTools` (winget verified the installer hash; one UAC approval) | machine |
| rustup + Rust stable (MSVC host) | rustc/cargo 1.99.1 | official `rustup-init.exe`; SHA-256 verified against `rustup-init.exe.sha256` from static.rust-lang.org (binary is not Authenticode-signed) | current user (`%USERPROFILE%\.cargo`) |
| Node.js | 24.18.0 | pre-existing | — |

Pinned in `rust-toolchain.toml` (channel 1.99.1, clippy, rustfmt, x86_64-pc-windows-msvc).
Build Tools 2022 was chosen over 2026 as the long-proven pairing with Rust's MSVC target.

## Evidence
- `vswhere` reports "Visual Studio Build Tools 2022" with `VC.Tools.x86.x64`.
- Toolcheck program (Rust, links `user32`, calls `SetThreadDpiAwarenessContext`,
  `GetSystemMetrics`, `GetDpiForSystem`): `cargo run --release` printed
  `virtual screen: 1920x1080  system dpi: 144` — matches Phase 0 measurements (ADR 0004).

## Consequences
Phase 1 M1 (core + IPC) can start. A new shell may be needed for `cargo` to be on PATH
(rustup added `%USERPROFILE%\.cargo\bin` to the user PATH).
