# Builds the Android app: Rust cdylib -> UniFFI Kotlin bindings -> debug APK.
# Needs: rustup targets aarch64-linux-android, cargo-ndk, ANDROID_HOME, ANDROID_NDK_HOME, JAVA_HOME (17+).
# Usage: pwsh scripts/android-build.ps1 [-Profile spike-abort] [-Debug]  (default: size-optimised library, minified release APK)
param([string]$Profile = "android", [switch]$Debug)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot

# Native tools do not throw on failure: stop at the first one that fails.
function Run([scriptblock]$step) {
    & $step
    if ($LASTEXITCODE -ne 0) { throw "failed (exit $LASTEXITCODE): $step" }
}

$jni = "android/app/src/main/jniLibs"
$self = Get-Process -Id $PID
$oldPriority = $self.PriorityClass
$oldRustflags = $env:RUSTFLAGS
Push-Location $root
try {
    $env:RUSTFLAGS = "-C link-arg=-Wl,-z,max-page-size=16384"   # 16 KB pages (Android 15+)
    # Keep the PC usable while building: half the cores, below-normal priority.
    $jobs = [int][Math]::Max(2, [Math]::Floor([Environment]::ProcessorCount / 2))
    $self.PriorityClass = "BelowNormal"
    Run { cargo ndk -t arm64-v8a -t x86_64 --platform 29 -o $jni build --profile $Profile -p pb-ffi -j $jobs }
    # cargo-ndk also copies stub cdylibs from iroh crates; keep only ours.
    Get-ChildItem $jni -Recurse -Filter "lib*.so" | Where-Object { $_.Name -ne "libpb_ffi.so" } | Remove-Item

    # Bindings are generated from the host build, because release profiles strip
    # UniFFI metadata. Host build without the Android linker flag.
    $env:RUSTFLAGS = $oldRustflags
    Run { cargo build -p pb-ffi }
    Run { cargo run -q -p uniffi-bindgen -- generate --library target/debug/pb_ffi.dll --language kotlin --out-dir android/app/src/main/java }

    Push-Location android
    try {
        # --no-daemon: no 2-3 GB Gradle/Kotlin daemons left running after the build.
        if ($Debug) { Run { ./gradlew.bat assembleDebug --no-daemon --console=plain }; $apk = "debug/app-debug.apk" }
        else { Run { ./gradlew.bat assembleRelease --no-daemon --console=plain }; $apk = "release/app-release.apk" }
    } finally { Pop-Location }
    Write-Host "APK: android/app/build/outputs/apk/$apk"
} finally {
    $env:RUSTFLAGS = $oldRustflags
    $self.PriorityClass = $oldPriority
    Pop-Location
}
