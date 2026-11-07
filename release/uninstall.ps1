# XtraOS Agent uninstaller (ADR 0019).
#   uninstall.ps1 [-Prefix <dir>] [-Purge] [-DataDir <dir>]
# Stops the core started from -Prefix (tray "Quit" command first, then a forced stop only for
# that exact binary), removes the login entry and the install folder. -Purge also deletes the
# XtraOS data folder (pairing token, logs, policy.json). -StopOnly is used by install.ps1.
param(
  [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\XtraOS'),
  [switch]$Purge,
  [string]$DataDir = $(if ($env:XTRAOS_HOME) { $env:XTRAOS_HOME } else { Join-Path $env:LOCALAPPDATA 'XtraOS' }),
  [string]$RunValueName = 'XtraOSCore',
  [switch]$StopOnly
)
$ErrorActionPreference = 'Stop'

if (-not ('XtraOSUninstall.Win' -as [type])) {
  Add-Type -Namespace XtraOSUninstall -Name Win -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, UIntPtr w, IntPtr l);
'@
}

function Stop-Core([string]$exe) {
  $procs = @(Get-Process xtraos-core -ErrorAction SilentlyContinue | Where-Object { $_.Path -and ($_.Path -ieq $exe) })
  foreach ($p in $procs) {
    # Graceful: post the tray "Quit" command (ID 1003) to that process's XtraOS UI window.
    $h = [IntPtr]::Zero
    while (($h = [XtraOSUninstall.Win]::FindWindowExW([IntPtr]::Zero, $h, 'XtraOSCoreUi', [NullString]::Value)) -ne [IntPtr]::Zero) {
      [uint32]$wpid = 0
      [void][XtraOSUninstall.Win]::GetWindowThreadProcessId($h, [ref]$wpid)
      if ($wpid -eq $p.Id) { [void][XtraOSUninstall.Win]::PostMessageW($h, 0x0111, [UIntPtr]::new([uint64]1003), [IntPtr]::Zero) }
    }
    if ($p.WaitForExit(3000)) {
      Write-Host "  stopped xtraos-core (pid $($p.Id)) - closed via its tray Quit command"
    } else {
      Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
      Write-Host "  stopped xtraos-core (pid $($p.Id)) - forced (it did not answer Quit)"
    }
  }
}

$coreExe = Join-Path $Prefix 'bin\xtraos-core.exe'
Stop-Core $coreExe
# An elevated admin helper (ADR 0030) exits by itself when its core stops; its path is not
# readable from here, so just wait for it (it never outlives the core).
$t0 = Get-Date
while (@(Get-Process xtraos-core -ErrorAction SilentlyContinue | Where-Object { -not $_.Path }).Count -gt 0 -and ((Get-Date) - $t0).TotalSeconds -lt 5) {
  Start-Sleep -Milliseconds 200
}
# MCP servers started by AI clients from this install (stateless; the client restarts them).
$nodeExe = Join-Path $Prefix 'node\node.exe'
foreach ($p in @(Get-Process node -ErrorAction SilentlyContinue | Where-Object { $_.Path -and ($_.Path -ieq $nodeExe) })) {
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  [void]$p.WaitForExit(3000)
  Write-Host "  stopped an XtraOS MCP server (pid $($p.Id)); close or restart your AI client"
}
if ($StopOnly) { return }

$run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
if (Get-ItemProperty -Path $run -Name $RunValueName -ErrorAction SilentlyContinue) {
  Remove-ItemProperty -Path $run -Name $RunValueName
  Write-Host "  removed login entry $RunValueName"
}
if (Test-Path (Join-Path $Prefix 'bin\xtraos-core.exe')) {
  Remove-Item -Recurse -Force $Prefix
  Write-Host "  removed $Prefix"
} elseif (Test-Path $Prefix) {
  Write-Host "  $Prefix does not look like an XtraOS install; left untouched"
}
if ($Purge) {
  if ((Test-Path (Join-Path $DataDir 'pairing.token')) -or (Test-Path (Join-Path $DataDir 'connection.json'))) {
    Remove-Item -Recurse -Force $DataDir
    Write-Host "  removed XtraOS data $DataDir"
  } elseif (Test-Path $DataDir) {
    Write-Host "  $DataDir does not look like XtraOS data; left untouched"
  }
}
Write-Host 'XtraOS Agent uninstalled. If you added it to Claude Code: claude mcp remove xtraos'
