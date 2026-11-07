# XtraOS Agent installer (per user, no admin; ADR 0019). Run from the extracted bundle:
#   powershell -ExecutionPolicy Bypass -File install.ps1 [-StartAtLogin] [-ConfigureClaudeCode]
# It copies files to -Prefix and prints the MCP configuration. It never changes security
# settings (Smart App Control, Defender, UAC). Opt-in only: a login entry, Claude Code config.
param(
  [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\XtraOS'),
  [switch]$StartAtLogin,
  [switch]$ConfigureClaudeCode,
  [string]$RunValueName = 'XtraOSCore'
)
$ErrorActionPreference = 'Stop'
$src = $PSScriptRoot
$version = (Get-Content (Join-Path $src 'VERSION') -Raw).Trim()

# Requirements: 64-bit Windows 10 2004 (build 19041) or later (overlay capture exclusion).
$build = [Environment]::OSVersion.Version.Build
if (-not [Environment]::Is64BitOperatingSystem) { throw 'XtraOS Agent needs 64-bit Windows.' }
if ($build -lt 19041) { throw "XtraOS Agent needs Windows 10 2004 (build 19041) or later; this is build $build." }

# Upgrade: stop a running core from this prefix first (gracefully via its tray command).
& (Join-Path $src 'uninstall.ps1') -Prefix $Prefix -StopOnly

Write-Host "Installing XtraOS Agent $version to $Prefix"
New-Item -ItemType Directory -Force $Prefix | Out-Null
foreach ($item in 'bin', 'mcp', 'node', 'xtraos-mcp.cmd', 'uninstall.ps1', 'INSTALL.md', 'THIRD-PARTY.md', 'VERSION') {
  $from = Join-Path $src $item
  $to = Join-Path $Prefix $item
  if (Test-Path $to) { Remove-Item -Recurse -Force $to }
  Copy-Item -Recurse $from $to
}
$mcp = Join-Path $Prefix 'xtraos-mcp.cmd'
$core = Join-Path $Prefix 'bin\xtraos-core.exe'

if ($StartAtLogin) {
  New-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name $RunValueName `
    -Value "`"$core`"" -PropertyType String -Force | Out-Null
  Write-Host "  start at login: on ($RunValueName)"
}

if ($ConfigureClaudeCode) {
  if (Get-Command claude -ErrorAction SilentlyContinue) {
    & claude mcp add --scope user xtraos -- $mcp
    Write-Host '  Claude Code: added MCP server "xtraos" (user scope)'
  } else {
    Write-Host '  Claude Code CLI not found; add it later with the command below.'
  }
}

$json = ($mcp -replace '\\', '\\')
Write-Host @"

Done. The MCP server starts the XtraOS core by itself when an agent first uses it
(tray icon appears; Ctrl+Alt+Esc or tray Stop halts the agent at any time).

Claude Code:
  claude mcp add --scope user xtraos -- "$mcp"

Claude Desktop (claude_desktop_config.json) / other MCP clients:
  { "mcpServers": { "xtraos": { "command": "$json" } } }

Note: this is an unsigned developer preview. If Smart App Control is on, Windows blocks
xtraos-core.exe (signed builds come later). Uninstall: "$Prefix\uninstall.ps1" (-Purge also
removes your XtraOS data: pairing token, logs, policy).
"@
