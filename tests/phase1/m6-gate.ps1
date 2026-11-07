# Phase 1 / M6 gate (docs/08-test-plan.md, ADRs 0019-0020): regression (non-interactive) +
# release bundle build + clean-profile install test; -Interactive adds the recorded flows
# (F2, F3, F4, F6, F10) and the release bench. Interactive parts move the mouse: hands off.
param([switch]$Interactive)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '..\..')
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$steps = @(
  @{ Name = 'rustfmt --check';                  Dir = 'core';    Cmd = 'cargo fmt --all -- --check' },
  @{ Name = 'clippy (warnings = errors)';        Dir = 'core';    Cmd = 'cargo clippy --all-targets -- -D warnings' },
  @{ Name = 'cargo test (unit + IPC)';           Dir = 'core';    Cmd = 'cargo test' },
  @{ Name = 'cargo build (core binary)';         Dir = 'core';    Cmd = 'cargo build' },
  @{ Name = 'tsc --noEmit';                      Dir = 'mcp';     Cmd = 'npm run --silent typecheck' },
  @{ Name = 'MCP contract + e2e tests';          Dir = 'mcp';     Cmd = 'npm test --silent' },
  @{ Name = 'testbed build';                     Dir = 'testbed'; Cmd = 'powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1' }
)
$steps += @{ Name = 'MCP bundle (esbuild)';             Dir = 'mcp';     Cmd = 'npm run --silent bundle' }
$steps += @{ Name = 'release bundle build';             Dir = 'release'; Cmd = 'powershell -NoProfile -ExecutionPolicy Bypass -File build-release.ps1' }
$steps += @{ Name = 'clean-profile install test (REL-01)'; Dir = '.';    Cmd = 'node release/test-install.mjs' }
if ($Interactive) {
  $steps += @{ Name = 'INTERACTIVE recorded flows (REL-02)'; Dir = '.'; Cmd = 'node tests/flows/run-flows.mjs' }
  $steps += @{ Name = 'INTERACTIVE release bench (REL-04)';  Dir = '.'; Cmd = 'node tests/bench/bench.mjs' }
}
$results = @()
foreach ($s in $steps) {
  if ($s.Name -like 'INTERACTIVE*') { '>>> Interactive: hands off the mouse and keyboard...' }
  Push-Location (Join-Path $root $s.Dir)
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $out = cmd /c "$($s.Cmd) 2>&1"
  $code = $LASTEXITCODE
  Pop-Location
  $results += [pscustomobject]@{ Step = $s.Name; Status = $(if ($code -eq 0) { 'PASS' } else { 'FAIL' }); Seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1) }
  if ($code -ne 0) { "---- $($s.Name) output ----"; $out | Select-Object -Last 60 }
  else {
    # also surface SKIP reasons and evidence lines printed by the window tests
    $summary = $out | Where-Object { $_ -match '^(PASS|FAIL|MISS) |^install test:|^flows:|^bench:|^built |^test result:|^. (tests|pass|fail) \d+|^test \S+ \.\.\. ok|SKIP:|using elevated|title = |^observe |median tokens|diff after|^~ |^\+ |pad screen|electron |scroll\(ref\)|^drag:|note:|kill switch:|confirm dialog said|Why you are asked|overlay \d' }
    if ($summary) { "  $($s.Name):"; $summary | ForEach-Object { "    " + $_.Trim() } }
  }
}
if (-not $Interactive) { foreach ($n in 'INTERACTIVE recorded flows (REL-02)', 'INTERACTIVE release bench (REL-04)') { $results += [pscustomobject]@{ Step = $n; Status = 'SKIP'; Seconds = 0 } } }
$results | Format-Table -AutoSize | Out-String
$fail = @($results | Where-Object Status -eq 'FAIL').Count
"M6 gate: PASS=$(@($results | Where-Object Status -eq 'PASS').Count) FAIL=$fail SKIP=$(@($results | Where-Object Status -eq 'SKIP').Count)"
if ($fail) { exit 1 }
