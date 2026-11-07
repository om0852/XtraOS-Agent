# Phase 2 / P2-M4 gate (docs/08 P2-M1..M4, ADRs 0022-0028): full regression (non-interactive)
# + release build + install test; -Interactive adds the P2-M1..M4 e2e suites (P2-M4: resource
# discipline + whatsapp.send), safety, the recorded flows and the bench.
# Real WhatsApp sends (P2-WA-01/02) run only if XTRAOS_TEST_WHATSAPP holds the owner's own number.
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
  $steps += @{ Name = 'INTERACTIVE P2-M1 e2e'; Dir = 'core'; Cmd = 'cargo test --test p2m1_e2e -- --ignored --test-threads=1 --nocapture' }
  $steps += @{ Name = 'INTERACTIVE P2-M2 OCR e2e'; Dir = 'core'; Cmd = 'cargo test --test p2m2_e2e -- --ignored --test-threads=1 --nocapture' }
  $steps += @{ Name = 'INTERACTIVE P2-M3 app map + skills e2e'; Dir = 'core'; Cmd = 'cargo test --test p2m3_e2e -- --ignored --test-threads=1 --nocapture' }
  $steps += @{ Name = 'INTERACTIVE P2-M4 resources + WhatsApp e2e'; Dir = 'core'; Cmd = 'cargo test --test p2m4_e2e -- --ignored --test-threads=1 --nocapture' }
  $steps += @{ Name = 'INTERACTIVE M5 safety e2e (policy approval)'; Dir = 'core'; Cmd = 'cargo test --test safety_e2e -- --ignored --test-threads=1 --nocapture' }
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
    $summary = $out | Where-Object { $_ -match '^(PASS|FAIL|MISS) |^install test:|^flows:|^bench:|^built |^wf|^ba0|^ocr |^query|^scan:|^launched|^delete dialog|^RES-|^WA-|^cleanup:|^close dialog|^send dialog|^macro:|^routine|^kill switch:|^\"|^scoped|^  Routine|^test result:|^. (tests|pass|fail) \d+|^test \S+ \.\.\. ok|SKIP:|using elevated|title = |^observe |median tokens|diff after|^~ |^\+ |pad screen|electron |scroll\(ref\)|^drag:|note:|kill switch:|confirm dialog said|Why you are asked|overlay \d' }
    if ($summary) { "  $($s.Name):"; $summary | ForEach-Object { "    " + $_.Trim() } }
  }
}
if (-not $Interactive) { foreach ($n in 'INTERACTIVE P2-M1 e2e', 'INTERACTIVE P2-M2 OCR e2e', 'INTERACTIVE P2-M3 app map + skills e2e', 'INTERACTIVE P2-M4 resources + WhatsApp e2e', 'INTERACTIVE M5 safety e2e (policy approval)', 'INTERACTIVE recorded flows (REL-02)', 'INTERACTIVE release bench (REL-04)') { $results += [pscustomobject]@{ Step = $n; Status = 'SKIP'; Seconds = 0 } } }
$results | Format-Table -AutoSize | Out-String
$fail = @($results | Where-Object Status -eq 'FAIL').Count
"P2-M4 gate: PASS=$(@($results | Where-Object Status -eq 'PASS').Count) FAIL=$fail SKIP=$(@($results | Where-Object Status -eq 'SKIP').Count)"
if ($fail) { exit 1 }
