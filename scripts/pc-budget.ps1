# Measures the PC tray against the budgets after M5 (docs/DECISIONS.md, "PC budgets after M5").
#
# Runs its own copy of pb-tray with a temporary data folder and its own port, so it never touches the
# running Relay. pb-tray is a single instance (a named mutex): the script refuses to start while another
# tray runs, so quit Relay first.
#
# Usage (from the repo root, after a release build):
#   cargo build --release -p pb-tray
#   pwsh scripts/pc-budget.ps1 -Exe target\release\pb-tray.exe
#
# Exits 1 if any budget is over; prints the table either way.

param(
    [string]$Exe = "target\release\pb-tray.exe",
    [int]$Port = 47250
)

$ErrorActionPreference = "Stop"

if (Get-Process -Name "pb-tray" -ErrorAction SilentlyContinue) {
    Write-Error "A pb-tray is already running (Relay?). Quit it, then run this again."
    exit 2
}

$dir = Join-Path $env:TEMP ("relay-budget-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $dir | Out-Null

function Sample($p) {
    $p.Refresh()
    [pscustomobject]@{
        WorkingSetMB = [math]::Round($p.WorkingSet64 / 1MB, 2)
        PrivateMB    = [math]::Round($p.PrivateMemorySize64 / 1MB, 2)
        CpuSeconds   = $p.TotalProcessorTime.TotalSeconds
    }
}

$proc = Start-Process -FilePath $Exe -ArgumentList @("--dir", $dir, "--port", $Port) -PassThru
try {
    # 1. Idle for 60 s; the CPU delta is measured over that window.
    Start-Sleep -Seconds 5
    $cpu0 = (Sample $proc).CpuSeconds
    Start-Sleep -Seconds 55
    $idle = Sample $proc
    $idleCpu = $idle.CpuSeconds - $cpu0

    # 2. The main window open. There is no pipe command to open it yet (see the M5.11 ruling), so the
    #    owner opens it from the tray menu while the script waits; the sample is taken 10 s later.
    Write-Host "Open the main window from the tray now (waiting 10 s)..."
    Start-Sleep -Seconds 10
    $window = Sample $proc

    # 3. Close the window and wait 40 s for the device release (it happens 30 s after the last surface).
    Write-Host "Close the main window now (waiting 40 s)..."
    Start-Sleep -Seconds 40

    $row = [pscustomobject]@{
        idle_working_set_mb = $idle.WorkingSetMB
        idle_private_mb     = $idle.PrivateMB
        window_private_mb   = $window.PrivateMB
        idle_cpu_seconds    = [math]::Round($idleCpu, 3)
    }
    $row | Format-Table -AutoSize

    $over = @()
    if ($row.idle_working_set_mb -gt 40) { $over += "idle working set $($row.idle_working_set_mb) MB > 40" }
    if ($row.idle_private_mb -gt 15) { $over += "idle private $($row.idle_private_mb) MB > 15" }
    if ($row.window_private_mb -gt 70) { $over += "window private $($row.window_private_mb) MB > 70" }
    if ($row.idle_cpu_seconds -ge 0.05) { $over += "idle CPU $($row.idle_cpu_seconds) s >= 0.05" }
    if ($over.Count -gt 0) {
        $over | ForEach-Object { Write-Host "OVER: $_" -ForegroundColor Red }
        exit 1
    }
    Write-Host "All PC budgets met." -ForegroundColor Green
}
finally {
    if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
}
