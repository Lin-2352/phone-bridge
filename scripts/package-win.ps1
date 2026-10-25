# Register Relay as a Windows share target (per user, no admin).
#
# Uses a sparse package: packaging/windows/AppxManifest.xml gives the existing
# pb-tray.exe a package identity; the exe is not copied. Loose registration
# needs Windows Developer Mode; a signed .msix for normal installs comes with
# the installer.
#
#   pwsh scripts/package-win.ps1            # register (exe in target\spike)
#   pwsh scripts/package-win.ps1 -Remove    # unregister
param([switch]$Remove, [string]$ExeDir = "$PSScriptRoot\..\target\spike")
$ErrorActionPreference = "Stop"
$pkgDir = Resolve-Path "$PSScriptRoot\..\packaging\windows"
# Check before unregistering anything, so a missing build never leaves the
# user with no share target at all.
if (-not $Remove -and -not (Test-Path "$ExeDir\pb-tray.exe")) { throw "pb-tray.exe not found in $ExeDir (build it first)" }

$existing = Get-AppxPackage -Name "PhoneBridge.Tray"
if ($existing) { Remove-AppxPackage -Package $existing.PackageFullName; "Removed $($existing.PackageFullName)" }
if ($Remove) { return }

# Logos: the app glyph on our cobalt, rendered at the sizes the manifest names.
Add-Type -AssemblyName System.Drawing
New-Item -ItemType Directory -Force "$pkgDir\Assets" | Out-Null
foreach ($spec in @(@("StoreLogo", 50), @("Square44x44Logo", 44), @("Square150x150Logo", 150))) {
    $n = $spec[1]; $bmp = New-Object System.Drawing.Bitmap $n, $n
    $g = [System.Drawing.Graphics]::FromImage($bmp); $g.SmoothingMode = "AntiAlias"; $g.Clear([System.Drawing.Color]::Transparent)
    $u = $n / 24.0
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $r = 5 * $u; $path.AddArc(0, 0, 2 * $r, 2 * $r, 180, 90); $path.AddArc($n - 2 * $r - 1, 0, 2 * $r, 2 * $r, 270, 90)
    $path.AddArc($n - 2 * $r - 1, $n - 2 * $r - 1, 2 * $r, 2 * $r, 0, 90); $path.AddArc(0, $n - 2 * $r - 1, 2 * $r, 2 * $r, 90, 90); $path.CloseFigure()
    $g.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 0x24, 0x46, 0xC9))), $path)
    $white = [System.Drawing.Color]::FromArgb(255, 0xF5, 0xF7, 0xFF)
    $g.FillRectangle((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(128, $white))), 10 * $u, 6 * $u, 9 * $u, 7 * $u)
    $g.FillRectangle((New-Object System.Drawing.SolidBrush $white), 6 * $u, 9 * $u, 5 * $u, 9 * $u)
    $bmp.Save("$pkgDir\Assets\$($spec[0]).png", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose()
}

$exeDir = Resolve-Path $ExeDir
if (-not (Test-Path "$exeDir\pb-tray.exe")) { throw "pb-tray.exe not found in $exeDir (build it first)" }
Add-AppxPackage -Register "$pkgDir\AppxManifest.xml" -ExternalLocation $exeDir
$p = Get-AppxPackage -Name "PhoneBridge.Tray"
"Registered $($p.PackageFullName) -> $exeDir"
