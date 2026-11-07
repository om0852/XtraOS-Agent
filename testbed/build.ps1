# Builds testbed\bin\testbed.exe with the .NET Framework C# compiler that ships with Windows.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (-not (Test-Path $csc)) { throw "csc.exe not found at $csc" }
$bin = Join-Path $here 'bin'
New-Item -ItemType Directory -Force $bin | Out-Null
$out = Join-Path $bin 'testbed.exe'
$src = Join-Path $here 'Testbed.cs'
& $csc /nologo /target:winexe /optimize+ /utf8output "/out:$out" /r:System.Windows.Forms.dll /r:System.Drawing.dll "$src"
if ($LASTEXITCODE -ne 0) { throw "testbed build failed ($LASTEXITCODE)" }
"built $out"
