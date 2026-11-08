# Signs Windows files with the owner's code-signing certificate, when one is configured.
#
#   pwsh scripts/sign-windows.ps1 -Path dist\Relay\Relay.exe [-Path more...]
#
# The certificate comes from secrets, by name only (set them as repository or environment secrets,
# never in a file):
#   WIN_SIGN_PFX_BASE64     the .pfx file, base64-encoded
#   WIN_SIGN_PFX_PASSWORD   its password
#   WIN_SIGN_TIMESTAMP_URL  optional, an RFC 3161 timestamp server (default below)
#
# Without WIN_SIGN_PFX_BASE64 the script prints one line saying signing was skipped and exits 0, so a
# fork, a pull request or a repository without a certificate still builds an unsigned file. The
# secrets are never printed, and the temporary .pfx is deleted even when signing fails.
param([Parameter(Mandatory)][string[]]$Path)
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($env:WIN_SIGN_PFX_BASE64)) {
    "Signing skipped: no code-signing certificate is configured (WIN_SIGN_PFX_BASE64 is not set). The files stay unsigned."
    exit 0
}
if ([string]::IsNullOrWhiteSpace($env:WIN_SIGN_PFX_PASSWORD)) {
    throw "WIN_SIGN_PFX_PASSWORD is not set"
}
foreach ($p in $Path) {
    if (-not (Test-Path -LiteralPath $p)) { throw "Nothing to sign at $p" }
}
$stamp = if ($env:WIN_SIGN_TIMESTAMP_URL) { $env:WIN_SIGN_TIMESTAMP_URL } else { "http://timestamp.digicert.com" }

$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $signtool) { throw "signtool.exe was not found (install the Windows SDK)" }

$pfx = Join-Path ([IO.Path]::GetTempPath()) ("relay-sign-" + [Guid]::NewGuid().ToString("N") + ".pfx")
try {
    [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WIN_SIGN_PFX_BASE64))
    foreach ($p in $Path) {
        & $signtool.FullName sign /fd SHA256 /f $pfx /p $env:WIN_SIGN_PFX_PASSWORD /tr $stamp /td SHA256 $p | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "signtool failed on $p (exit $LASTEXITCODE)" }
        & $signtool.FullName verify /pa $p | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "the signature on $p does not verify (exit $LASTEXITCODE)" }
        "Signed $p"
    }
}
finally {
    if (Test-Path -LiteralPath $pfx) { Remove-Item -LiteralPath $pfx -Force }
}
