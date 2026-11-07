# Spike S6: user-only named pipe + token pairing; Node client as the MCP stand-in.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Add-Type -Path (Join-Path $here 'S6.cs') -ReferencedAssemblies 'System.Core'
[S6]::Start()
Start-Sleep -Milliseconds 200
$client = Join-Path $here 'client.mjs'
"S6 pipe: \\.\pipe\$([S6]::PipeName)"
"DACL (SDDL): $([S6]::Sddl)"
"1) correct token, 500 pings : " + (node $client ([S6]::PipeName) ([S6]::Token) 500)
"2) wrong token              : " + (node $client ([S6]::PipeName) ('0' * 64) 1)
"3) no token                 : " + (node $client ([S6]::PipeName) '-' 1)
"server counters: accepted=$([S6]::Accepted) rejected=$([S6]::Rejected)"
[S6]::Stop()
