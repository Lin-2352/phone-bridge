# Builds the Android smoke app: Rust cdylib -> UniFFI Kotlin bindings -> debug APK.
# Needs: rustup targets aarch64-linux-android, cargo-ndk, ANDROID_HOME, ANDROID_NDK_HOME, JAVA_HOME (17+).
# Usage: pwsh scripts/android-smoke.ps1 [-Profile spike-abort]
param([string]$Profile = "spike-abort")
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot
Set-Location $root

$jni = "android/app/src/main/jniLibs"
$env:RUSTFLAGS = "-C link-arg=-Wl,-z,max-page-size=16384"   # 16 KB pages (Android 15+)
cargo ndk -t arm64-v8a --platform 29 -o $jni build --profile $Profile -p ffi-smoke
# cargo-ndk also copies stub cdylibs from iroh crates; keep only ours.
Get-ChildItem "$jni/arm64-v8a" -Filter "lib*.so" | Where-Object { $_.Name -ne "libpb_smoke.so" } | Remove-Item

# Bindings are generated from the host build, because release profiles strip UniFFI metadata.
cargo build -p ffi-smoke
cargo run -q -p uniffi-bindgen -- generate --library target/debug/pb_smoke.dll --language kotlin --out-dir android/app/src/main/java

Push-Location android
./gradlew.bat assembleDebug --console=plain
Pop-Location
Write-Host "APK: android/app/build/outputs/apk/debug/app-debug.apk"
