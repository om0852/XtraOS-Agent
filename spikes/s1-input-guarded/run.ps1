# Spike S1: guarded real clicks on our own non-activating test window (one per monitor).
# Moves your cursor for ~1-2 s, then puts it back. Keep hands off the mouse while it runs.
param([switch]$Run)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Add-Type -Path (Join-Path $here '..\common\TestWindow.cs'), (Join-Path $here 'S1.cs') `
  -ReferencedAssemblies 'System.Windows.Forms','System.Drawing'
if (-not $Run) {
  "S1 compiled OK. Dry run only: nothing was clicked."
  "It will: open a small top-most test window per monitor (never takes focus), move the cursor"
  "to 3 buttons on it, click each once, then restore your cursor. Aborts if you move the mouse,"
  "hold a button, or anything other than its own button is under the cursor."
  "Run for real with:  .\run.ps1 -Run"
  return
}
"S1 guarded input: hands off the mouse for ~2 seconds..."
[S1]::Run()
