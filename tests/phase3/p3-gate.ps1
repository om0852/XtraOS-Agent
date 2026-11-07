# Phase 3 gate (docs/08 P2-M1..M8, P3-M1..M3; ADRs 0022-0036): full regression (non-interactive): full regression (non-interactive)
# + release build + install test.
# -Automated adds the P2-M5 admin-helper e2e (helper UNELEVATED in test mode: no UAC, no human;
#   it opens testbed windows and answers the helper's dialog itself) and the P2-M6 browser-bridge
#   e2e against a fake bridge (dialog + kill-switch rows included). The live bridge row needs
#   XTRAOS_TEST_BRIDGE=1.
# -Interactive (hands off) adds the P2-M1..M4 e2e suites, safety, flows, bench, and the P2-M5
#   human test (real UAC prompt) when XTRAOS_TEST_UAC=1.
# Real WhatsApp sends (P2-WA-01/02) run only if XTRAOS_TEST_WHATSAPP holds the owner's own number.
param([switch]$Interactive, [switch]$Automated)
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
if ($Automated -or $Interactive) {
  $steps += @{ Name = 'AUTOMATED P2-M5 admin helper e2e (no UAC)'; Dir = 'core'; Cmd = 'cargo test --test p2m5_e2e -- --ignored --test-threads=1 --nocapture --skip eh_f5' }
  $steps += @{ Name = 'AUTOMATED P2-M6 browser bridge e2e (fake bridge)'; Dir = 'core'; Cmd = 'cargo test --test p2m6_e2e -- --include-ignored --test-threads=1 --nocapture' }
  $steps += @{ Name = 'AUTOMATED P2-M7/M8 + P3-M1/M2 e2e (lists, viewer, editor, frame_diff, record/replay)'; Dir = 'core'; Cmd = 'cargo test --release --test p2m7_e2e -- --ignored --test-threads=1 --nocapture --skip lo07' }
  $steps += @{ Name = 'AUTOMATED security fixes (spoofed Allow, posted Resume)'; Dir = 'core'; Cmd = 'cargo test --test sec_e2e -- --ignored --test-threads=1 --nocapture --skip sec03' }
  $steps += @{ Name = 'AUTOMATED P3-M3 gesture policy (no injection)'; Dir = 'core'; Cmd = 'cargo test --test p3m3_e2e tp04 -- --ignored --nocapture' }
}
if ($Interactive) {
  $steps += @{ Name = 'INTERACTIVE security: injected click on Allow ignored (SEC-03)'; Dir = 'core'; Cmd = 'cargo test --test sec_e2e sec03 -- --ignored --nocapture' }
  $steps += @{ Name = 'INTERACTIVE P3-M4 live Sandbox (needs Windows Sandbox, XTRAOS_TEST_SANDBOX=1)'; Dir = 'core'; Cmd = 'cargo test --test p3m4_e2e sb_live -- --ignored --nocapture' }
  $steps += @{ Name = 'INTERACTIVE P3-M3 touch + pen (real input)'; Dir = 'core'; Cmd = 'cargo test --test p3m3_e2e -- --ignored --test-threads=1 --nocapture --skip tp04' }
  $steps += @{ Name = 'INTERACTIVE P2-M7 input census (30 s hands off)'; Dir = 'core'; Cmd = 'cargo test --test p2m7_e2e lo07 -- --ignored --nocapture' }
  $steps += @{ Name = 'INTERACTIVE P2-M5 F5 admin prompt (UAC, human)'; Dir = 'core'; Cmd = 'cargo test --test p2m5_e2e eh_f5 -- --ignored --nocapture' }
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
    $summary = $out | Where-Object { $_ -match '^(PASS|FAIL|MISS) |^install test:|^flows:|^bench:|^built |^wf|^ba0|^ocr |^query|^scan:|^launched|^delete dialog|^RES-|^WA-|^EH-|^BR-|^LO-|^ED-|^FD-|^RR-|^TP-|^SB|^SEC-|^helper dialog|^dialog:|^cleanup:|^close dialog|^send dialog|^macro:|^routine|^kill switch:|^\"|^scoped|^  Routine|^test result:|^. (tests|pass|fail) \d+|^test \S+ \.\.\. ok|SKIP:|using elevated|title = |^observe |median tokens|diff after|^~ |^\+ |pad screen|electron |scroll\(ref\)|^drag:|note:|kill switch:|confirm dialog said|Why you are asked|overlay \d' }
    if ($summary) { "  $($s.Name):"; $summary | ForEach-Object { "    " + $_.Trim() } }
  }
}
if (-not ($Automated -or $Interactive)) { $results += [pscustomobject]@{ Step = 'AUTOMATED P2-M5 admin helper e2e (no UAC)'; Status = 'SKIP'; Seconds = 0 }; $results += [pscustomobject]@{ Step = 'AUTOMATED P2-M6 browser bridge e2e (fake bridge)'; Status = 'SKIP'; Seconds = 0 }; $results += [pscustomobject]@{ Step = 'AUTOMATED P2-M7/M8 + P3-M1/M2 e2e (lists, viewer, editor, frame_diff, record/replay)'; Status = 'SKIP'; Seconds = 0 }; $results += [pscustomobject]@{ Step = 'AUTOMATED P3-M3 gesture policy (no injection)'; Status = 'SKIP'; Seconds = 0 }; $results += [pscustomobject]@{ Step = 'AUTOMATED security fixes (spoofed Allow, posted Resume)'; Status = 'SKIP'; Seconds = 0 } }
if (-not $Interactive) { foreach ($n in 'INTERACTIVE security: injected click on Allow ignored (SEC-03)', 'INTERACTIVE P3-M4 live Sandbox (needs Windows Sandbox, XTRAOS_TEST_SANDBOX=1)', 'INTERACTIVE P3-M3 touch + pen (real input)', 'INTERACTIVE P2-M7 input census (30 s hands off)', 'INTERACTIVE P2-M5 F5 admin prompt (UAC, human)', 'INTERACTIVE P2-M1 e2e', 'INTERACTIVE P2-M2 OCR e2e', 'INTERACTIVE P2-M3 app map + skills e2e', 'INTERACTIVE P2-M4 resources + WhatsApp e2e', 'INTERACTIVE M5 safety e2e (policy approval)', 'INTERACTIVE recorded flows (REL-02)', 'INTERACTIVE release bench (REL-04)') { $results += [pscustomobject]@{ Step = $n; Status = 'SKIP'; Seconds = 0 } } }
$results | Format-Table -AutoSize | Out-String
$fail = @($results | Where-Object Status -eq 'FAIL').Count
"Phase 3 gate: PASS=$(@($results | Where-Object Status -eq 'PASS').Count) FAIL=$fail SKIP=$(@($results | Where-Object Status -eq 'SKIP').Count)"
if ($fail) { exit 1 }
