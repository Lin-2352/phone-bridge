# Builds the Relay folder into a temp dir and checks what is in it.
#   pwsh scripts/tests/make-portable.Tests.ps1
$ErrorActionPreference = "Stop"
$out = Join-Path ([IO.Path]::GetTempPath()) "relay test $(Get-Random)"
try {
    & "$PSScriptRoot\..\make-portable.ps1" -OutDir $out -Zip
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
    $zip = Join-Path $out "Relay-portable.zip"
    if (-not (Test-Path $zip)) { $fail += "missing Relay-portable.zip" } else {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $names = [IO.Compression.ZipFile]::OpenRead($zip).Entries | ForEach-Object { $_.FullName }
        if ($names -notcontains "Relay/Relay.exe") { $fail += "zip lacks Relay/Relay.exe" }
    }
    if ($fail) { $fail | ForEach-Object { Write-Error $_ -ErrorAction Continue }; exit 1 }
    "OK: $dir"
} finally {
    try { if (Test-Path $out) { Remove-Item -Recurse -Force $out } } catch { Write-Warning "left $out behind: $_" }
}
