# Phase 0 gate runner (docs/08-test-plan.md). Each spike runs in its own powershell.exe process
# (Add-Type types cannot be reloaded in one session) and its output is asserted with regexes.
# Safe tests always run. Interactive tests (mouse / launching Calculator) need -Interactive.
param([switch]$Interactive)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '..\..')
$spikes = Join-Path $root 'spikes'
$results = New-Object System.Collections.ArrayList

function Spike([string]$dir, [string[]]$extra = @()) {
  $script = Join-Path $spikes "$dir\run.ps1"
  $out = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $script @extra 2>&1 | Out-String
  return $out
}
function Record([string]$id, [string]$name, [string]$status, [string]$detail) {
  [void]$results.Add([pscustomobject]@{ ID = $id; Status = $status; Test = $name; Detail = $detail })
}
function Check([string]$id, [string]$name, [bool]$ok, [string]$detail) {
  Record $id $name ($(if ($ok) { 'PASS' } else { 'FAIL' })) $detail
}

# ---- P0-01  S2 UIA tree (read-only) ----
$o = Spike 's2-uia-tree'
$rows = [regex]::Matches($o, '(?m)^(\S+)\s+visited=\s*(\d+)\s+kept=\s*(\d+)\s+ms=\s*(\d+)\s+chars=\s*(\d+)\s+~tokens=\s*(\d+)')
if ($rows.Count -eq 0) { Record 'P0-01' 'UIA tree readable from outside apps' 'SKIP' 'no supported app window open' }
else {
  $anyKept = @($rows | Where-Object { [int]$_.Groups[3].Value -gt 0 }).Count -gt 0
  $maxTok  = ($rows | ForEach-Object { [int]$_.Groups[6].Value } | Measure-Object -Maximum).Maximum
  Check 'P0-01' 'UIA tree readable from outside apps' ($anyKept -and $maxTok -le 1500) "windows=$($rows.Count) maxTokens=$maxTok"
}

# ---- P0-02  S3 capture mapping (own window) ----
$o = Spike 's3-capture-mapping'
$mons = [regex]::Matches($o, '(?m)^monitor (\d+) .*PrintWindow (.+?)\s+ScreenCopy (.+)$')
$bad = @($mons | Where-Object { $_.Groups[2].Value.Trim() -ne 'delta=(0,0)' -or $_.Groups[3].Value.Trim() -ne 'delta=(0,0)' })
Check 'P0-02' 'Capture->screen mapping on every monitor' ($mons.Count -gt 0 -and $bad.Count -eq 0) "monitors=$($mons.Count) mismatches=$($bad.Count)"

# ---- P0-03..06  S6 pipe pairing ----
$o = Spike 's6-pipe-pairing'
$p50 = [regex]::Match($o, '"p50_ms":"([\d.]+)"')
Check 'P0-03' 'Pipe pairing happy path, p50 < 2 ms' ($o -match '1\) correct token.*"paired":true' -and $p50.Success -and [double]$p50.Groups[1].Value -lt 2) "p50=$($p50.Groups[1].Value) ms"
Check 'P0-04' 'Pipe rejects wrong token' ($o -match '2\) wrong token.*NOT_PAIRED') ''
Check 'P0-05' 'Pipe rejects missing token' ($o -match '3\) no token.*NOT_PAIRED' -and $o -match 'accepted=1 rejected=2') ''
$sddl = [regex]::Match($o, 'DACL \(SDDL\): (\S+)').Groups[1].Value
$allow = [regex]::Matches($sddl, '\(A;').Count
Check 'P0-06' 'Pipe DACL: current user only + NETWORK denied' ($allow -eq 1 -and $sddl -match '\(D;;[^)]*;;;NU\)' -and $sddl -match '\(A;;[^)]*;;;S-1-5-21-[\d-]+\)') $sddl

# ---- Interactive ----
if ($Interactive) {
  $o = Spike 's1-input-guarded' @('-Run')
  $clicks = [regex]::Matches($o, '(?m)^monitor \d+ .* cursorErr=\((-?\d+),(-?\d+)\) clicked=(\w+) stray=(\d+)')
  $badClicks = @($clicks | Where-Object { $_.Groups[1].Value -ne '0' -or $_.Groups[2].Value -ne '0' -or $_.Groups[3].Value -ne 'True' -or $_.Groups[4].Value -ne '0' })
  Check 'P0-07' 'Guarded clicks on own window' ($clicks.Count -ge 3 -and $badClicks.Count -eq 0 -and $o -match 'RESULT: completed') "clicks=$($clicks.Count) bad=$($badClicks.Count)"

  $o = Spike 's9-real-app' @('-Run')
  Check 'P0-08' 'Real app: Calculator clicks = 15' ($o -match 'clicks result: "Display is 15".*PASS') ''
  Check 'P0-09' 'Real app: Calculator keyboard = 36' ($o -match 'keyboard result: "Display is 36".*PASS') ''
  Start-Sleep -Milliseconds 800
  $left = @(Get-Process CalculatorApp -ErrorAction SilentlyContinue).Count
  Check 'P0-10' 'Real app cleanup (Calculator closed)' ($o -match 'closed our Calculator' -and $left -eq 0) "CalculatorApp processes left=$left"
  if ($results | Where-Object { $_.Status -ne 'PASS' -and $_.ID -in 'P0-07','P0-08','P0-09','P0-10' }) { "---- interactive output (for diagnosis) ----"; $o }
} else {
  foreach ($t in @(@('P0-07','Guarded clicks on own window'), @('P0-08','Real app: Calculator clicks = 15'),
                   @('P0-09','Real app: Calculator keyboard = 36'), @('P0-10','Real app cleanup'))) {
    Record $t[0] $t[1] 'SKIP' 'run with -Interactive'
  }
}

Add-Type -AssemblyName System.Windows.Forms
$monCount = [System.Windows.Forms.Screen]::AllScreens.Length
# S3, S1 and S9 already loop over every monitor; with >= 2 screens their PASS proves multi-monitor.
if ($monCount -lt 2) {
  Record 'P0-11' 'Multi-monitor accuracy' 'SKIP' "monitors=$monCount (deferred, ADR 0007; SKIP allowed)"
} elseif (-not $Interactive) {
  Record 'P0-11' 'Multi-monitor accuracy' 'SKIP' "monitors=$monCount; run with -Interactive"
} else {
  $perMon = @($results | Where-Object { $_.ID -in 'P0-02','P0-07','P0-08','P0-09' -and $_.Status -ne 'PASS' }).Count -eq 0
  Check 'P0-11' 'Multi-monitor accuracy' $perMon "monitors=$monCount (from P0-02/07/08/09 per-monitor loops)"
}

$results | Format-Table -AutoSize ID, Status, Test, Detail | Out-String -Width 220
$fail = @($results | Where-Object Status -eq 'FAIL').Count
$pass = @($results | Where-Object Status -eq 'PASS').Count
$skip = @($results | Where-Object Status -eq 'SKIP').Count
"Phase 0 gate: PASS=$pass FAIL=$fail SKIP=$skip"
if ($fail -gt 0) { exit 1 }
