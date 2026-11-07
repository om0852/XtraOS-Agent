# Phase 1 / M1 gate (docs/08-test-plan.md): formatting, lints, unit + integration + MCP e2e.
# Safe: uses random pipe names, random tokens and temp folders; no input injection.
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '..\..')
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$steps = @(
  @{ Name = 'rustfmt --check';            Dir = 'core'; Cmd = 'cargo fmt --all -- --check' },
  @{ Name = 'clippy (warnings = errors)';  Dir = 'core'; Cmd = 'cargo clippy --all-targets -- -D warnings' },
  @{ Name = 'cargo test (unit + IPC)';     Dir = 'core'; Cmd = 'cargo test' },
  @{ Name = 'cargo build (core binary)';   Dir = 'core'; Cmd = 'cargo build' },
  @{ Name = 'tsc --noEmit';                Dir = 'mcp';  Cmd = 'npm run --silent typecheck' },
  @{ Name = 'MCP contract + e2e tests';    Dir = 'mcp';  Cmd = 'npm test --silent' }
)
$results = @()
foreach ($s in $steps) {
  Push-Location (Join-Path $root $s.Dir)
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $out = cmd /c "$($s.Cmd) 2>&1"
  $code = $LASTEXITCODE
  Pop-Location
  $results += [pscustomobject]@{ Step = $s.Name; Status = $(if ($code -eq 0) { 'PASS' } else { 'FAIL' }); Seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1) }
  if ($code -ne 0) { "---- $($s.Name) output ----"; $out | Select-Object -Last 40 }
  else {
    $summary = $out | Where-Object { $_ -match '^test result:|^. (tests|pass|fail) \d+' }
    if ($summary) { "  $($s.Name): " + (($summary | ForEach-Object { $_.Trim() }) -join ' | ') }
  }
}
$results | Format-Table -AutoSize | Out-String
$fail = @($results | Where-Object Status -eq 'FAIL').Count
"M1 gate: PASS=$(@($results | Where-Object Status -eq 'PASS').Count) FAIL=$fail"
if ($fail) { exit 1 }
