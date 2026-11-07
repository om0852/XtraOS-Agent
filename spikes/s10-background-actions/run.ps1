# Spike S10 — background actions (ADR 0014): can the agent operate a real app WITHOUT moving
# the user's cursor, sending keystrokes or changing the foreground window?
# Calculator (native UIA provider) is driven only through UI Automation control patterns
# (Invoke, Window.Close) while the user may keep working. It opens one Calculator window and
# closes it again; nothing else is touched.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct P { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct LII { public uint cbSize, dwTime; }
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out P p);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetLastInputInfo(ref LII i);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  public static uint LastInput() { var i = new LII(); i.cbSize = 8; GetLastInputInfo(ref i); return i.dwTime; }
}
'@
$ae = [Windows.Automation.AutomationElement]
$TS = [Windows.Automation.TreeScope]
function ById($root, $id) {
  $root.FindFirst($TS::Descendants, (New-Object Windows.Automation.PropertyCondition($ae::AutomationIdProperty, $id)))
}
function CalcWindows {
  @($ae::RootElement.FindAll($TS::Children, (New-Object Windows.Automation.PropertyCondition($ae::NameProperty, 'Calculator'))))
}

$userFg = [W]::GetForegroundWindow()
$before = CalcWindows | ForEach-Object { $_.Current.NativeWindowHandle }
Start-Process calc.exe
$calc = $null; $deadline = (Get-Date).AddSeconds(15)
while (-not $calc -and (Get-Date) -lt $deadline) {
  Start-Sleep -Milliseconds 300
  $calc = CalcWindows | Where-Object { $before -notcontains $_.Current.NativeWindowHandle } | Select-Object -First 1
}
if (-not $calc) { throw 'Calculator window did not appear' }
# Launching takes the foreground; give it back to the user's window, so the measurement covers
# only the actions themselves.
Start-Sleep -Milliseconds 1500   # let Calculator finish its own activation
[void][W]::SetForegroundWindow($userFg); Start-Sleep -Milliseconds 500

$fgBefore = [W]::GetForegroundWindow()
$c0 = New-Object W+P; [void][W]::GetCursorPos([ref]$c0); $in0 = [W]::LastInput()

$sw = [Diagnostics.Stopwatch]::StartNew()
$fgTrail = @()
foreach ($id in 'clearButton', 'num7Button', 'num8Button', 'plusButton', 'num4Button', 'equalButton') {
  (ById $calc $id).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
  $fgTrail += "$id->$([W]::GetForegroundWindow())"
}
$invokeMs = $sw.Elapsed.TotalMilliseconds
Start-Sleep -Milliseconds 300
$display = (ById $calc 'CalculatorResults').Current.Name

$c1 = New-Object W+P; [void][W]::GetCursorPos([ref]$c1); $in1 = [W]::LastInput()
$fgAfter = [W]::GetForegroundWindow()
# mitigation under test: hand the foreground straight back to the user's window
$sw2 = [Diagnostics.Stopwatch]::StartNew()
$restoreCall = [W]::SetForegroundWindow($fgBefore)
$restored = $false; for ($k = 0; $k -lt 20 -and -not $restored; $k++) { Start-Sleep -Milliseconds 10; $restored = ([W]::GetForegroundWindow() -eq $fgBefore) }; $restoreMs = $sw2.Elapsed.TotalMilliseconds

# graceful close, also through UIA (no click on the X)
$calc.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close()
Start-Sleep -Milliseconds 800
$closed = -not (CalcWindows | Where-Object { $_.Current.NativeWindowHandle -eq $calc.Current.NativeWindowHandle })

$userInput = $in1 -ne $in0          # GetLastInputInfo changes only for (real or injected) input
$cursorMoved = ($c0.X -ne $c1.X) -or ($c0.Y -ne $c1.Y)
[pscustomobject]@{
  'display'                  = $display
  '6 invokes (ms)'           = [math]::Round($invokeMs, 1)
  'foreground unchanged'     = ($fgBefore -eq $fgAfter)
  'cursor moved'             = $cursorMoved
  'any input event meanwhile'= $userInput
  'closed via WindowPattern' = $closed
  'handles'                  = "user=$userFg before=$fgBefore after=$fgAfter calc=$($calc.Current.NativeWindowHandle)"
  'foreground after each'    = ($fgTrail -join ' ')
  'foreground restored'      = "$restored (call=$restoreCall, $([math]::Round($restoreMs,1)) ms)"

} | Format-List | Out-String
$ok = ($display -eq 'Display is 82') -and $closed -and (-not $cursorMoved -or $userInput)
"S10 result correct + cursor untouched: $(if ($ok) { 'PASS' } else { 'FAIL' })"
"S10 foreground untouched by the action: $(if ($fgBefore -eq $fgAfter) { 'YES' } else { 'NO (provider activated its window); restored by agent: ' + $restored })"
