# Build the portable Relay folder from the spike build (no installer, no admin
# except the one UAC prompt the camera setup raises on first use).
#
#   cargo build --profile spike -p pb-tray -p pb-cli
#   pwsh scripts/make-portable.ps1                 # -> dist\Relay
param([string]$OutDir = "$PSScriptRoot\..\dist", [string]$BinDir = "$PSScriptRoot\..\target\spike", [switch]$Zip)
$ErrorActionPreference = "Stop"
foreach ($f in "pb-tray.exe", "pb.exe", "pb_camera_source.dll") {
    if (-not (Test-Path "$BinDir\$f")) { throw "$f not found in $BinDir (build it first)" }
}
New-Item -ItemType Directory -Force $OutDir | Out-Null
$dir = Join-Path (Resolve-Path $OutDir) "Relay"
if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
New-Item -ItemType Directory -Force "$dir\data" | Out-Null
Copy-Item "$BinDir\pb-tray.exe" "$dir\Relay.exe"
Copy-Item "$BinDir\pb.exe" "$dir\relay-cli.exe"
# The tray looks for the camera source next to its own exe.
Copy-Item "$BinDir\pb_camera_source.dll" "$dir\pb_camera_source.dll"
Copy-Item "$PSScriptRoot\..\packaging\portable\README.txt" "$dir\README.txt"
# Everything it keeps (pairing, settings, log) stays in data\ beside the exe.
Set-Content -Path "$dir\Relay.cmd" -Encoding ascii -Value '@echo off', 'start "" "%~dp0Relay.exe" --dir "%~dp0data" --name "%COMPUTERNAME%"'
if ($Zip) {
    $zipPath = Join-Path (Split-Path $dir) "Relay-portable.zip"
    if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
    Compress-Archive -Path $dir -DestinationPath $zipPath
    "Zipped $zipPath"
}
"Built $dir"
