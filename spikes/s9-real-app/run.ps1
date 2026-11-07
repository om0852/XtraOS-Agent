# Spike S9: operate a real app (a fresh Windows Calculator) on every monitor with guarded real input.
param([switch]$Run)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Add-Type -Path (Join-Path $here '..\common\TestWindow.cs'), (Join-Path $here 'S9.cs') `
  -ReferencedAssemblies 'System.Windows.Forms','System.Drawing','UIAutomationClient','UIAutomationTypes','WindowsBase'
if (-not $Run) { "S9 compiled OK (dry run). Run with -Run: opens Calculator ~5 s, clicks/types into it, closes it."; return }
"S9 real-app test: Calculator will open for a few seconds; hands off mouse and keyboard..."
[S9]::Run()
