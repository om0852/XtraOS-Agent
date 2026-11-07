# ADR 0002 — Phase 0 spikes use built-in Windows tooling

- Status: accepted
- Date: 2026-11-07

## Context
The owner's machine has Node.js, Git and Windows PowerShell 5.1, but no Rust toolchain,
MSVC C++ build tools, or .NET SDK. Installing them is a large system change, and the owner
requires: no paid services and nothing that can disturb the OS or ongoing activity.
Phase 0 spikes test **Windows behaviour** (input, UI Automation, capture, pipes), which is
the same from any language.

## Decision
Phase 0 spikes are written in C# compiled at run time by Windows PowerShell 5.1 (`Add-Type`,
.NET Framework 4.x, C# 5 syntax) plus Node.js for the client side of the pipe spike.
Spikes follow the safety rules in `spikes/README.md`.
The product core stays Rust (ADR 0001); installing Rust + MSVC Build Tools is deferred until
the start of Phase 1 and needs the owner's explicit go-ahead.

## Evidence
Toolchain check on 2026-11-07: `node v24.18.0`, `git 2.55.0`; `rustc`, `cargo`, `dotnet` absent;
`vswhere` finds no VC++ tools.

## Consequences
Zero installs for Phase 0. Spike code is throwaway; only the measured behaviour carries
forward (into ADRs). Some APIs (Windows.Graphics.Capture, a WinRT API) are awkward from
PowerShell 5.1, so S3 tests the GDI/PrintWindow path and WGC is re-validated in Rust.
