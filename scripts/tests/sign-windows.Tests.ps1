#Requires -Version 7
# Checks scripts/sign-windows.ps1 without a certificate: it skips cleanly, refuses half a configuration,
# and never prints a secret.
#   pwsh scripts/tests/sign-windows.Tests.ps1
$ErrorActionPreference = "Stop"
$script = Join-Path $PSScriptRoot "..\sign-windows.ps1"
$file = Join-Path ([IO.Path]::GetTempPath()) "relay-sign-test-$(Get-Random).exe"
Set-Content -Path $file -Value "not a real exe"
$fail = @()
function Run([hashtable]$env_) {
    $names = "WIN_SIGN_PFX_BASE64", "WIN_SIGN_PFX_PASSWORD", "WIN_SIGN_TIMESTAMP_URL"
    $saved = @{}
    foreach ($n in $names) { $saved[$n] = [Environment]::GetEnvironmentVariable($n); [Environment]::SetEnvironmentVariable($n, $env_[$n]) }
    try {
        $out = & pwsh -NoProfile -File $script -Path $file 2>&1 | Out-String
        return @{ Code = $LASTEXITCODE; Out = $out }
    } finally {
        foreach ($n in $names) { [Environment]::SetEnvironmentVariable($n, $saved[$n]) }
    }
}
try {
    $r = Run @{}
    if ($r.Code -ne 0) { $fail += "no certificate: exit $($r.Code), want 0" }
    if ($r.Out -notmatch "Signing skipped") { $fail += "no certificate: no 'Signing skipped' line" }

    $secret = "S3cretPassw0rd-do-not-print"
    $r = Run @{ WIN_SIGN_PFX_BASE64 = "AAAA" }
    if ($r.Code -eq 0) { $fail += "certificate without a password was accepted" }

    $r = Run @{ WIN_SIGN_PFX_BASE64 = "AAAA"; WIN_SIGN_PFX_PASSWORD = $secret }
    if ($r.Code -eq 0) { $fail += "a bad certificate was accepted" }
    if ($r.Out -match [regex]::Escape($secret)) { $fail += "the password was printed" }
    if ($r.Out -match "AAAA\b" -and $r.Out -match "WIN_SIGN_PFX_BASE64=") { $fail += "the certificate was printed" }

    $leftovers = Get-ChildItem ([IO.Path]::GetTempPath()) -Filter "relay-sign-*.pfx" -ErrorAction SilentlyContinue
    if ($leftovers) { $fail += "a temporary .pfx was left behind" }
} finally {
    try { Remove-Item -Force $file -ErrorAction Stop } catch { }
}
if ($fail.Count) { $fail | ForEach-Object { "FAIL $_" }; exit 1 }
"all passed"
