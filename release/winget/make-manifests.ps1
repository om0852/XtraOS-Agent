# winget manifests for a release zip (ADR 0071), schema 1.6: version, installer, defaultLocale.
# The zip is installed as "portable": winget extracts it and puts xtraos-core.exe on PATH as
# `xtraos-core` (the command-line bridge). Submitting to github.com/microsoft/winget-pkgs is a
# separate, public step for the owner.
param(
  [Parameter(Mandatory)][string]$Version,
  [Parameter(Mandatory)][string]$ZipUrl,
  [Parameter(Mandatory)][string]$Sha256,
  [Parameter(Mandatory)][string]$Out,
  [string]$Id = 'XtraOS.Agent'
)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "bad version '$Version'" }
if ($Sha256 -notmatch '^[0-9A-Fa-f]{64}$') { throw 'Sha256 must be 64 hex characters' }
if ($ZipUrl -notmatch '^https://') { throw 'ZipUrl must be https' }
New-Item -ItemType Directory -Force $Out | Out-Null
$schema = '1.6.0'
$common = @"
PackageIdentifier: $Id
PackageVersion: $Version
"@
@"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.version.$schema.schema.json
$common
DefaultLocale: en-US
ManifestType: version
ManifestVersion: $schema
"@ | Set-Content -Encoding utf8 (Join-Path $Out "$Id.yaml")
@"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.installer.$schema.schema.json
$common
InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
  - RelativeFilePath: xtraos-$Version-win-x64\bin\xtraos-core.exe
    PortableCommandAlias: xtraos-core
Installers:
  - Architecture: x64
    InstallerUrl: $ZipUrl
    InstallerSha256: $($Sha256.ToUpper())
MinimumOSVersion: 10.0.19041.0
ManifestType: installer
ManifestVersion: $schema
"@ | Set-Content -Encoding utf8 (Join-Path $Out "$Id.installer.yaml")
@"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.defaultLocale.$schema.schema.json
$common
PackageLocale: en-US
Publisher: Om Sachin Salunke
PublisherUrl: https://github.com/om0852
PackageName: XtraOS Agent
PackageUrl: https://github.com/om0852/XtraOS-Agent
License: MIT
LicenseUrl: https://github.com/om0852/XtraOS-Agent/blob/main/LICENSE
ShortDescription: Lets AI agents see and operate Windows safely (UI Automation, approvals, kill switch).
Moniker: xtraos
Tags:
  - ai-agent
  - automation
  - mcp
  - ui-automation
ManifestType: defaultLocale
ManifestVersion: $schema
"@ | Set-Content -Encoding utf8 (Join-Path $Out "$Id.locale.en-US.yaml")
Write-Host "winget manifests for $Id $Version in $Out"
