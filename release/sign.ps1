# Code signing (ADR 0071): signs XtraOS's own executables with signtool, SHA-256 and an RFC 3161
# timestamp. Certificate from the store (-CertThumbprint) or a .pfx file (-PfxPath; password in
# the environment variable XTRAOS_SIGN_PFX_PASSWORD, never on the command line or in a file).
# -DryRun prints what would be signed and checks the inputs, without signing.
param(
  [Parameter(Mandatory)][string[]]$Files,
  [string]$CertThumbprint = '',
  [string]$PfxPath = '',
  [string]$TimestampUrl = 'http://timestamp.digicert.com',
  [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
if (-not $CertThumbprint -and -not $PfxPath) { throw 'give -CertThumbprint or -PfxPath' }
if ($CertThumbprint -and $PfxPath) { throw 'give only one of -CertThumbprint and -PfxPath' }
if ($CertThumbprint -and $CertThumbprint -notmatch '^[0-9A-Fa-f]{40}$') { throw 'the thumbprint is 40 hex characters (SHA-1)' }
if ($PfxPath -and -not (Test-Path $PfxPath)) { throw "no certificate file at $PfxPath" }
foreach ($f in $Files) { if (-not (Test-Path $f)) { throw "nothing to sign at $f" } }

$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
  Sort-Object FullName | Select-Object -Last 1
if (-not $signtool) { throw 'signtool.exe (Windows SDK) not found' }

$cert = if ($CertThumbprint) { @('/sha1', $CertThumbprint) } else {
  if (-not $env:XTRAOS_SIGN_PFX_PASSWORD) { throw 'set XTRAOS_SIGN_PFX_PASSWORD for the .pfx' }
  @('/f', $PfxPath, '/p', $env:XTRAOS_SIGN_PFX_PASSWORD)
}
foreach ($f in $Files) {
  $shown = ($cert | ForEach-Object { if ($_ -eq $env:XTRAOS_SIGN_PFX_PASSWORD) { '****' } else { $_ } }) -join ' '
  Write-Host "    sign $f ($shown, timestamp $TimestampUrl)"
  if ($DryRun) { continue }
  & $signtool.FullName sign @cert /fd SHA256 /tr $TimestampUrl /td SHA256 $f
  if ($LASTEXITCODE) { throw "signing $f failed" }
  $s = Get-AuthenticodeSignature $f
  if ($s.Status -ne 'Valid') { throw "after signing, $f reports $($s.Status)" }
  Write-Host "    ok: $($s.SignerCertificate.Subject)"
}
if ($DryRun) { Write-Host "    dry run: signtool at $($signtool.FullName); nothing signed" }
