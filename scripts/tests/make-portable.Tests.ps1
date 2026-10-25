# Builds the Relay folder into a temp dir and checks what is in it.
#   pwsh scripts/tests/make-portable.Tests.ps1
$ErrorActionPreference = "Stop"
$out = Join-Path ([IO.Path]::GetTempPath()) "relay test $(Get-Random)"
try {
    & "$PSScriptRoot\..\make-portable.ps1" -OutDir $out
    $dir = Join-Path $out "Relay"
    $fail = @()
    foreach ($f in "Relay.exe", "relay-cli.exe", "pb_camera_source.dll", "README.txt", "Relay.cmd") {
        if (-not (Test-Path (Join-Path $dir $f))) { $fail += "missing $f" }
    }
    if (-not (Test-Path (Join-Path $dir "data") -PathType Container)) { $fail += "missing data folder" }
    $cmd = Get-Content (Join-Path $dir "Relay.cmd") -Raw
    if ($cmd -notmatch [regex]::Escape('%~dp0data')) { $fail += "Relay.cmd does not use a relative data folder" }
    $repo = (Resolve-Path "$PSScriptRoot\..\..").Path
    if ($cmd -match [regex]::Escape($repo)) { $fail += "Relay.cmd holds the repo path" }
    if ($fail) { $fail | ForEach-Object { Write-Error $_ -ErrorAction Continue }; exit 1 }
    "OK: $dir"
} finally {
    try { if (Test-Path $out) { Remove-Item -Recurse -Force $out } } catch { Write-Warning "left $out behind: $_" }
}
