# Builds the Android app: Rust cdylib -> UniFFI Kotlin bindings -> debug APK.
# Needs: rustup targets aarch64-linux-android, cargo-ndk, ANDROID_HOME, ANDROID_NDK_HOME, JAVA_HOME (17+).
# Usage: pwsh scripts/android-build.ps1 [-Profile spike-abort] [-Debug]  (default: size-optimised library, minified release APK)
param([string]$Profile = "android", [switch]$Debug)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot
Set-Location $root

$jni = "android/app/src/main/jniLibs"
$env:RUSTFLAGS = "-C link-arg=-Wl,-z,max-page-size=16384"   # 16 KB pages (Android 15+)
cargo ndk -t arm64-v8a -t x86_64 --platform 29 -o $jni build --profile $Profile -p pb-ffi
# cargo-ndk also copies stub cdylibs from iroh crates; keep only ours.
Get-ChildItem $jni -Recurse -Filter "lib*.so" | Where-Object { $_.Name -ne "libpb_ffi.so" } | Remove-Item

# Bindings are generated from the host build, because release profiles strip UniFFI metadata.
cargo build -p pb-ffi
cargo run -q -p uniffi-bindgen -- generate --library target/debug/pb_ffi.dll --language kotlin --out-dir android/app/src/main/java

Push-Location android
if ($Debug) { ./gradlew.bat assembleDebug --console=plain; $apk = "debug/app-debug.apk" }
else { ./gradlew.bat assembleRelease --console=plain; $apk = "release/app-release.apk" }
Pop-Location
Write-Host "APK: android/app/build/outputs/apk/$apk"
