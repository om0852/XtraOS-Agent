# Phase 1 / M2 gate (docs/08-test-plan.md): everything from M1 + testbed build + interactive
# input tests (only with -Interactive: they move the mouse and type into the testbed window).
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
if ($Interactive) {
  $steps += @{ Name = 'INTERACTIVE input e2e (testbed)'; Dir = 'core'; Cmd = 'cargo test --test input_e2e -- --ignored --test-threads=1' }
}
$results = @()
foreach ($s in $steps) {
  if ($s.Name -like 'INTERACTIVE*') { '>>> Interactive input tests: hands off the mouse and keyboard (~30 s)...' }
  Push-Location (Join-Path $root $s.Dir)
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $out = cmd /c "$($s.Cmd) 2>&1"
  $code = $LASTEXITCODE
  Pop-Location
  $results += [pscustomobject]@{ Step = $s.Name; Status = $(if ($code -eq 0) { 'PASS' } else { 'FAIL' }); Seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1) }
  if ($code -ne 0) { "---- $($s.Name) output ----"; $out | Select-Object -Last 60 }
  else {
    $summary = $out | Where-Object { $_ -match '^test result:|^. (tests|pass|fail) \d+|^test \S+ \.\.\. ok' }
    if ($summary) { "  $($s.Name):"; $summary | ForEach-Object { "    " + $_.Trim() } }
  }
}
if (-not $Interactive) { $results += [pscustomobject]@{ Step = 'INTERACTIVE input e2e (testbed)'; Status = 'SKIP'; Seconds = 0 } }
$results | Format-Table -AutoSize | Out-String
$fail = @($results | Where-Object Status -eq 'FAIL').Count
"M2 gate: PASS=$(@($results | Where-Object Status -eq 'PASS').Count) FAIL=$fail SKIP=$(@($results | Where-Object Status -eq 'SKIP').Count)"
if ($fail) { exit 1 }
