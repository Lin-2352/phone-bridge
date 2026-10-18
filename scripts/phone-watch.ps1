# Read-only watcher for the test phone: if it freezes or reboots, the minutes
# before are on the PC. Writes to %TEMP%\pb-phone-watch:
#   logcat-<hour>.txt  main/system/crash/events buffers, one file per hour (24 kept)
#   samples.txt        every 30 s: memory pressure (PSI), MemAvailable/swap,
#                      top 10 processes by PSS, Phone Bridge PSS and threads
# Stops after -Hours (default 24). Usage: pwsh scripts/phone-watch.ps1 [-Serial X] [-Hours 24]
param([string]$Serial = "79KRFEJVPVOBQCHQ", [int]$Hours = 24)
$adb = "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe"
$dir = "$env:TEMP\pb-phone-watch"; New-Item -ItemType Directory -Force $dir | Out-Null
(Get-Process -Id $PID).PriorityClass = "BelowNormal"
$end = (Get-Date).AddHours($Hours)
$log = $null; $logHour = -1
function Sample {
    $t = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    $psi = (& $adb -s $Serial shell cat /proc/pressure/memory 2>$null) -join " | "
    $mem = (& $adb -s $Serial shell "grep -E 'MemAvailable|SwapFree' /proc/meminfo" 2>$null) -join " "
    $top = & $adb -s $Serial shell dumpsys meminfo -c 2>$null | Select-String "^proc," |
        ForEach-Object { $f = $_.Line -split ","; [pscustomobject]@{ n = $f[2]; pss = [int]$f[4] } } |
        Sort-Object pss -Descending | Select-Object -First 10 | ForEach-Object { "$($_.n)=$([math]::Round($_.pss/1024))M" }
    $pb = & $adb -s $Serial shell 'p=$(pidof dev.phonebridge.app); [ -n "$p" ] && grep -E "VmRSS|Threads" /proc/$p/status | tr -s " \t" " " | tr "\n" " " || echo not-running' 2>$null
    "$t | $mem | psi: $psi | phonebridge: $pb | top: $($top -join ', ')" | Add-Content "$dir\samples.txt"
}
while ((Get-Date) -lt $end) {
    $h = (Get-Date).Hour
    if ($h -ne $logHour -or ($log -and $log.HasExited)) {
        if ($log -and -not $log.HasExited) { $log.Kill() }
        $file = "$dir\logcat-$(Get-Date -Format 'yyyyMMdd-HH').txt"
        $log = Start-Process $adb -ArgumentList "-s", $Serial, "logcat", "-b", "main,system,crash,events", "-v", "threadtime" `
            -RedirectStandardOutput $file -WindowStyle Hidden -PassThru
        $logHour = $h
        Get-ChildItem $dir -Filter "logcat-*.txt" | Sort-Object Name -Descending | Select-Object -Skip 24 | Remove-Item -ErrorAction SilentlyContinue
    }
    Sample
    Start-Sleep 30
}
if ($log -and -not $log.HasExited) { $log.Kill() }
