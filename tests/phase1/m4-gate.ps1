# Phase 1 / M4 gate (docs/08-test-plan.md): everything from M3 + perception and background-first
# actions (ADRs 0014-0016). Interactive tests only with -Interactive (keep hands off: several
# tests prove the cursor did not move); -Uac adds the human UAC test, -Browsers Chrome/Firefox.
param([switch]$Interactive, [switch]$Uac, [switch]$Browsers)
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
  $steps += @{ Name = "INTERACTIVE input e2e (testbed)"; Dir = "core"; Cmd = "cargo test --test input_e2e -- --ignored --test-threads=1" }
  if ($Uac) { $env:XTRAOS_TEST_UAC = "1" } else { Remove-Item Env:XTRAOS_TEST_UAC -ErrorAction SilentlyContinue }
  if ($Browsers) { $env:XTRAOS_TEST_BROWSERS = "1" } else { Remove-Item Env:XTRAOS_TEST_BROWSERS -ErrorAction SilentlyContinue }
  $steps += @{ Name = "INTERACTIVE window e2e"; Dir = "core"; Cmd = "cargo test --test windows_e2e -- --ignored --test-threads=1 --nocapture" }
  $steps += @{ Name = "INTERACTIVE perception + background e2e"; Dir = "core"; Cmd = "cargo test --test perception_e2e -- --ignored --test-threads=1 --nocapture" }
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
    # also surface SKIP reasons and evidence lines printed by the window tests
    $summary = $out | Where-Object { $_ -match '^test result:|^. (tests|pass|fail) \d+|^test \S+ \.\.\. ok|SKIP:|using elevated|title = |^observe |median tokens|diff after|^~ |^\+ |pad screen|electron |scroll\(ref\)|^drag:|note:' }
    if ($summary) { "  $($s.Name):"; $summary | ForEach-Object { "    " + $_.Trim() } }
  }
}
if (-not $Interactive) { foreach ($n in 'INTERACTIVE input e2e (testbed)', 'INTERACTIVE window e2e', 'INTERACTIVE perception + background e2e') { $results += [pscustomobject]@{ Step = $n; Status = 'SKIP'; Seconds = 0 } } }
$results | Format-Table -AutoSize | Out-String
$fail = @($results | Where-Object Status -eq 'FAIL').Count
"M4 gate: PASS=$(@($results | Where-Object Status -eq 'PASS').Count) FAIL=$fail SKIP=$(@($results | Where-Object Status -eq 'SKIP').Count)"
if ($fail) { exit 1 }
