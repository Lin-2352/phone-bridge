# Builds the portable Relay folder and zip (no installer, no admin except the one
# UAC prompt the camera setup raises on first use).
#
#   cargo build --release -p pb-tray -p pb-camera-source
#   pwsh scripts/make-portable.ps1 -Zip              # -> dist\Relay, dist\Relay-<version>-windows-x64-portable.zip
param([string]$OutDir = "$PSScriptRoot\..\dist", [string]$BinDir = "$PSScriptRoot\..\target\release", [switch]$Zip)
$ErrorActionPreference = "Stop"
foreach ($f in "pb-tray.exe", "pb_camera_source.dll") {
    if (-not (Test-Path "$BinDir\$f")) { throw "$f not found in $BinDir (build it first, see the top of this script)" }
}
New-Item -ItemType Directory -Force $OutDir | Out-Null
$dir = Join-Path (Resolve-Path $OutDir) "Relay"
if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
# A data folder beside the exe is what makes it portable: pairing, settings and logs stay in it.
New-Item -ItemType Directory -Force "$dir\data" | Out-Null
Copy-Item "$BinDir\pb-tray.exe" "$dir\Relay.exe"
# No relay-cli.exe: the command-line tool is for development and tests, not users.
# The tray looks for the camera source next to its own exe.
Copy-Item "$BinDir\pb_camera_source.dll" "$dir\pb_camera_source.dll"
Copy-Item "$PSScriptRoot\..\packaging\portable\README.txt" "$dir\README.txt"
Copy-Item "$PSScriptRoot\..\LICENSE-APACHE", "$PSScriptRoot\..\LICENSE-MIT" $dir
if ($Zip) {
    $version = (Select-String -Path "$PSScriptRoot\..\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
    $zipPath = Join-Path (Split-Path $dir) "Relay-$version-windows-x64-portable.zip"
    if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
    Compress-Archive -LiteralPath $dir -DestinationPath $zipPath
    "Zipped $zipPath"
}
"Built $dir"
