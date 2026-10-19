# One row of the transfer speed matrix: how fast Phone Bridge moves files for
# the way the phone and PC are connected right now (same Wi-Fi, phone hotspot,
# PC hotspot, Wi-Fi Direct, USB...). The user sets up the connection; this
# script only measures.
#
#   pwsh scripts/speed-matrix.ps1 -Scenario "phone hotspot 5GHz"
#
# Needs: pb-tray running (test dir below), Phone Bridge on and paired, adb.
# Sends 300 MB PC -> phone (Phone Bridge in the background) and 2 x 100 MB
# phone -> PC, checks MD5, records link facts, appends to %TEMP%\pb-speed-matrix.csv.
param(
    [Parameter(Mandatory)] [string]$Scenario,
    [string]$Serial = "79KRFEJVPVOBQCHQ",
    [string]$TrayDir = "$env:TEMP\pbm2_trayPhone",
    [string]$InboxDir = "$env:TEMP\pbm3in"
)
$adb = "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe"
function A { & $adb -s $Serial @args }
$tray = Join-Path $PSScriptRoot "..\target\spike\pb-tray.exe"
$log = Join-Path $TrayDir "tray.log"
function TrayCount($pattern) { (Get-Content $log | Select-String $pattern).Count }
# One CSV row; fields quoted (SSIDs can contain commas and quotes).
function Row([object[]]$fields) {
    ($fields | ForEach-Object { '"' + ("$_" -replace '"', '""') + '"' }) -join ","
}
$csv = "$env:TEMP\pb-speed-matrix.csv"
$stamp = Get-Date -Format "HHmmss"

"== $Scenario"
# Link facts on both sides.
$wi = (A shell dumpsys wifi | Select-String "mWifiInfo" | Select-Object -First 1).Line
$phoneLink = if ($wi -and $wi -match '(?<!B)SSID: "?([^",]*)') {
    $ssid = $Matches[1]
    $freq = if ($wi -match 'Frequency: (\d+)') { $Matches[1] } else { "?" }
    $rate = if ($wi -match 'Tx Link speed: (\d+)') { $Matches[1] } else { "?" }
    "wifi $ssid ${freq}MHz tx${rate}Mbps"
} else { "no wifi" }
$pcIf = netsh wlan show interfaces | Out-String
$pcLink = if ($pcIf -match 'SSID\s*:\s*(.+)') {
    $ssid = $Matches[1].Trim()
    $band = if ($pcIf -match 'Band\s*:\s*(.+)') { $Matches[1].Trim() } else { "" }
    $ch = if ($pcIf -match 'Channel\s*:\s*(\d+)') { $Matches[1] } else { "?" }
    $rx = if ($pcIf -match 'Receive rate \(Mbps\)\s*:\s*([\d.]+)') { $Matches[1] } else { "?" }
    "wifi $ssid $band ch$ch rx${rx}Mbps"
} else { "no wifi" }
"phone: $phoneLink"
"pc:    $pcLink"

# Reachability: a tiny file (does not depend on Share clipboard being on;
# rediscovery finds the new address if the network changed).
$ping = "$env:TEMP\pbping-$stamp.txt"
Set-Content $ping "matrix ping $stamp"
$reach = $false
for ($i = 0; $i -lt 4 -and -not $reach; $i++) {
    $p0 = TrayCount "FILE sent `"pbping-$stamp"
    Start-Process $tray -ArgumentList "--send", "`"$ping`""
    $deadline = (Get-Date).AddSeconds(25)
    do { Start-Sleep 2 } until ((TrayCount "FILE sent `"pbping-$stamp") -gt $p0 -or (Get-Date) -gt $deadline)
    $reach = (TrayCount "FILE sent `"pbping-$stamp") -gt $p0
}
"reachable: $reach"
if (-not $reach) {
    Row @((Get-Date).ToString('s'), $Scenario, $phoneLink, $pcLink, "unreachable", "", "", "", "", "") | Add-Content $csv
    return
}

# PC -> phone, 300 MB, Phone Bridge in the background.
$src = "$env:TEMP\pbbg_300MB.bin"
if (-not (Test-Path $src)) { $b = New-Object byte[] (300MB); (New-Object Random 33).NextBytes($b); [IO.File]::WriteAllBytes($src, $b); $b = $null }
$md5big = (Get-FileHash $src -Algorithm MD5).Hash.ToLower()
# A name unique to this run (hard link, no copy), so the check below can never
# match a copy left by an earlier run.
$bigName = "pbbg_300MB-$stamp.bin"
$big = "$env:TEMP\$bigName"
New-Item -ItemType HardLink -Path $big -Target $src | Out-Null
A shell input keyevent KEYCODE_HOME
$s0 = TrayCount "FILE sent `"$bigName"
Start-Process $tray -ArgumentList "--send", "`"$big`""
$deadline = (Get-Date).AddMinutes(4)
do { Start-Sleep 3 } until ((TrayCount "FILE sent `"$bigName") -gt $s0 -or (Get-Date) -gt $deadline)
$sent = Get-Content $log | Select-String "FILE sent `"$bigName" | Select-Object -Last 1
$down = if ($sent -match '\(([\d.]+) MB/s\)') { [double]$Matches[1] } else { 0 }
Start-Sleep 3
$path = A shell "find /storage/emulated/0/ -name '$bigName' 2>/dev/null" | Select-Object -First 1
$downOk = [bool]$path -and ((((A shell "md5sum '$path'") -split ' ')[0]) -eq $md5big)
"PC -> phone 300 MB: $down MB/s, md5 ok=$downOk"

# Phone -> PC, 2 x 100 MB (a file Phone Bridge owns in Downloads).
$q = A shell "content query --uri content://media/external/downloads --projection _id:_display_name --where `"_display_name='pbm3_100MB.bin'`""
$id = ([regex]::Match(($q -join " "), "_id=(\d+)")).Groups[1].Value
$md5up = ((A shell "md5sum '/sdcard/Download/Phone Bridge/pbm3_100MB.bin'") -split ' ')[0]
if (-not $id -or -not $md5up -or $md5up -match "No such") {
    "phone -> PC skipped: Download/Phone Bridge/pbm3_100MB.bin is not on the phone (send it there once first)"
    Row @((Get-Date).ToString('s'), $Scenario, $phoneLink, $pcLink, "ok", $down, $downOk, "", "skipped", "") | Add-Content $csv
    return
}
$r0 = TrayCount "FILE received .*pbm3_100MB"
$t0 = Get-Date
for ($i = 1; $i -le 2; $i++) {
    A shell am start -a android.intent.action.SEND -t application/octet-stream --eu android.intent.extra.STREAM "content://media/external/downloads/$id" -n dev.phonebridge.app/.ShareActivity | Out-Null
    $deadline = (Get-Date).AddMinutes(3)
    do { Start-Sleep 2 } until ((TrayCount "FILE received .*pbm3_100MB") -ge ($r0 + $i) -or (Get-Date) -gt $deadline)
}
$ups = Get-Content $log | Select-String "FILE received .*pbm3_100MB" | Select-Object -Skip $r0 | ForEach-Object { if ($_ -match '([\d.]+) MB/s') { [double]$Matches[1] } }
$up = if ($ups) { [math]::Round(($ups | Measure-Object -Average).Average, 1) } else { 0 }
$upOk = (Get-ChildItem $InboxDir -Filter "pbm3_100MB*" | Where-Object { $_.LastWriteTime -gt $t0 -and (Get-FileHash $_.FullName -Algorithm MD5).Hash.ToLower() -eq $md5up }).Count
"phone -> PC 2 x 100 MB: $($ups -join ' / ') MB/s (avg $up), md5 ok=$upOk/2"

# Health.
$p = A shell pidof dev.phonebridge.app
$adj = if ($p) { A shell "cat /proc/$p/oom_score_adj" } else { "dead" }
$free = [math]::Round([int]((A shell "grep MemAvailable /proc/meminfo") -replace '\D', '') / 1024)
"Phone Bridge priority $adj, phone free ${free} MB, crashes: " + (A logcat -d -b crash | Select-String "Process: dev.phonebridge").Count

Row @((Get-Date).ToString('s'), $Scenario, $phoneLink, $pcLink, "ok", $down, $downOk, $up, "$upOk/2", $adj) | Add-Content $csv
"Test files left on the phone (delete when done): $bigName and pbping-$stamp.txt in Download/Phone Bridge"
