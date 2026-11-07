# Spike S2 (read-only): walk UI Automation trees of common apps; print counts/timings only.
param(
  [string[]]$Processes = @('explorer','chrome','firefox','msedge','WindowsTerminal','notepad','Code','Claude'),
  [int]$MaxDepth = 12, [int]$MaxKeep = 120, [int]$BudgetMs = 4000
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$out  = Join-Path $here '..\out'
Add-Type -TypeDefinition (Get-Content -Raw (Join-Path $here 'S2.cs')) -Language CSharp `
  -ReferencedAssemblies 'UIAutomationClient','UIAutomationTypes','WindowsBase','System.Core'
"S2 UIA walk  depth<=$MaxDepth keep<=$MaxKeep budget=${BudgetMs}ms"
[S2]::Run($Processes, $MaxDepth, $MaxKeep, $BudgetMs, $out)
