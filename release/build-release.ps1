# Builds the Release 0.1 bundle (ADR 0019): dist\xtraos-<ver>-win-x64\ and .zip + SHA256SUMS.txt.
#   -CertThumbprint <sha1>  sign bin\xtraos-core.exe with that certificate (Windows SDK signtool)
# Everything is free and local: release build of the core, the MCP server with production
# dependencies, and the official signed Node 24 runtime copied from this machine.
#   -PfxPath <file.pfx>     or sign with a .pfx (password in XTRAOS_SIGN_PFX_PASSWORD) (ADR 0071)
param([string]$CertThumbprint = '', [string]$PfxPath = '', [string]$Version = '0.1.1')
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$name = "xtraos-$Version-win-x64"
$dist = Join-Path $root 'dist'
$out = Join-Path $dist $name

function Step($m) { Write-Host ">>> $m" }

Step 'core: cargo build --release'
Push-Location (Join-Path $root 'core')
cmd /c "cargo build --release 2>&1" | Select-Object -Last 2
if ($LASTEXITCODE) { throw 'cargo build --release failed' }
Pop-Location

# Clear the old bundle's contents rather than the folder itself: a process that has the folder as
# its working directory (or OneDrive syncing it) keeps the folder, but never its files.
if (Test-Path $out) { Get-ChildItem -Force $out | Remove-Item -Recurse -Force }
New-Item -ItemType Directory -Force (Join-Path $out 'bin'), (Join-Path $out 'node'), (Join-Path $out 'mcp') | Out-Null
Copy-Item (Join-Path $root 'core\target\release\xtraos-core.exe') (Join-Path $out 'bin')

Step 'mcp: one bundled file (esbuild) instead of thousands of node_modules files'
Push-Location (Join-Path $root 'mcp')
cmd /c "npm run --silent bundle 2>&1" | Select-Object -Last 2
if ($LASTEXITCODE) { throw 'mcp bundle failed' }
$npmProd = cmd /c "npm ls --omit=dev --all --json 2>nul" | ConvertFrom-Json
Pop-Location
Copy-Item (Join-Path $root 'mcp\build\xtraos-mcp.mjs') (Join-Path $out 'mcp')

Step 'node: official runtime (must be Node 24, signed by the OpenJS Foundation)'
$nodeSrc = (Get-Command node).Source
$nodeVer = (& $nodeSrc --version).Trim()
if ($nodeVer -notmatch '^v24\.') { throw "Node 24 required to bundle, found $nodeVer" }
$sig = Get-AuthenticodeSignature $nodeSrc
if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'OpenJS Foundation') { throw "node.exe signature not valid: $($sig.Status)" }
Copy-Item $nodeSrc (Join-Path $out 'node\node.exe')
$nodeHash = (Get-FileHash (Join-Path $out 'node\node.exe') -Algorithm SHA256).Hash
"Node.js $nodeVer (official build, Authenticode: $($sig.SignerCertificate.Subject))`nSHA-256 $nodeHash" |
  Set-Content -Encoding utf8 (Join-Path $out 'node\VERSION.txt')

Step 'launcher, scripts, docs'
# The MCP launcher every client config points at.
"@echo off`r`n`"%~dp0node\node.exe`" `"%~dp0mcp\xtraos-mcp.mjs`" %*`r`n" |
  Set-Content -Encoding ascii -NoNewline (Join-Path $out 'xtraos-mcp.cmd')
# `xtraos setup` / `xtraos doctor` (ADR 0045): the same launcher under a shorter name.
Copy-Item (Join-Path $out 'xtraos-mcp.cmd') (Join-Path $out 'xtraos.cmd')
foreach ($f in 'install.ps1', 'uninstall.ps1', 'INSTALL.md') { Copy-Item (Join-Path $PSScriptRoot $f) $out }
"$Version" | Set-Content -Encoding ascii -NoNewline (Join-Path $out 'VERSION')

# Third-party notices: Rust crates in the release build + npm packages shipped + Node.
$tp = @("# Third-party software in XtraOS Agent $Version", '', '## Node.js runtime', '',
  "Node.js $nodeVer — MIT licence (https://github.com/nodejs/node/blob/main/LICENSE), official signed build.", '',
  '## Rust crates (xtraos-core.exe)', '', '| Crate | Version | Licence |', '|---|---|---|')
Push-Location (Join-Path $root 'core')
$meta = cmd /c "cargo metadata --format-version 1 --filter-platform x86_64-pc-windows-msvc 2>nul" | ConvertFrom-Json
Pop-Location
$used = @{}
foreach ($n in $meta.resolve.nodes) { $used[$n.id] = $true }
$tp += $meta.packages | Where-Object { $used[$_.id] -and $_.name -ne 'xtraos-core' } | Sort-Object name, version |
  ForEach-Object { "| $($_.name) | $($_.version) | $($_.license) |" }
$tp += @('', '## npm packages bundled into mcp/xtraos-mcp.mjs', '', '| Package | Version | Licence |', '|---|---|---|')
$pkgs = @{}
function Walk($deps) { if (-not $deps) { return }; foreach ($p in $deps.PSObject.Properties) { $pkgs["$($p.Name)@$($p.Value.version)"] = $p; Walk $p.Value.dependencies } }
Walk $npmProd.dependencies
foreach ($k in ($pkgs.Keys | Sort-Object)) {
  $p = $pkgs[$k]
  $pj = Join-Path $root "mcp\node_modules\$($p.Name)\package.json"
  $lic = if (Test-Path $pj) { (Get-Content $pj -Raw | ConvertFrom-Json).license } else { '?' }
  $tp += "| $($p.Name) | $($p.Value.version) | $lic |"
}
$tp | Set-Content -Encoding utf8 (Join-Path $out 'THIRD-PARTY.md')

if ($CertThumbprint -or $PfxPath) {
  Step 'signing XtraOS executables (ADR 0071)'
  $sign = @{ Files = @(Join-Path $out 'bin\xtraos-core.exe') }
  if ($CertThumbprint) { $sign.CertThumbprint = $CertThumbprint } else { $sign.PfxPath = $PfxPath }
  & (Join-Path $PSScriptRoot 'sign.ps1') @sign
} else {
  Write-Host '    (not signed: developer preview — runs where Smart App Control is off; ADR 0019)'
}

Step 'zip + checksums'
$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item $zip }
Compress-Archive -Path $out -DestinationPath $zip -CompressionLevel Optimal
$sums = foreach ($f in @($zip, (Join-Path $out 'bin\xtraos-core.exe'))) { "$((Get-FileHash $f -Algorithm SHA256).Hash.ToLower())  $(Split-Path $f -Leaf)" }
$sums | Set-Content -Encoding ascii (Join-Path $dist 'SHA256SUMS.txt')
$mb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host "built $zip ($mb MB)"
$sums
