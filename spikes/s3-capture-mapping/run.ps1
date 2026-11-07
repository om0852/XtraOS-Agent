# Spike S3: capture our own non-activating window on each monitor; verify capture->screen mapping.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Add-Type -Path (Join-Path $here '..\common\TestWindow.cs'), (Join-Path $here 'S3.cs') `
  -ReferencedAssemblies 'System.Windows.Forms','System.Drawing'
"S3 capture mapping (delta = found marker - expected marker, physical px; 0,0 is perfect)"
[S3]::Run((Join-Path $here '..\out'))
