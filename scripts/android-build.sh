#!/usr/bin/env bash
# Builds the Android app on Linux: Rust cdylib -> UniFFI Kotlin bindings -> APK.
# The bash twin of android-build.ps1, used by CI.
# Needs: cargo-ndk; rustup targets aarch64-linux-android, armv7-linux-androideabi and
# x86_64-linux-android; ANDROID_HOME; ANDROID_NDK_HOME; JDK 17+.
# Usage: scripts/android-build.sh [--variant debug|release|qa] [--profile android]
set -euo pipefail
variant=qa
profile=android
while [ $# -gt 0 ]; do
    case "$1" in
        --variant) variant="$2"; shift 2 ;;
        --profile) profile="$2"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done
case "$variant" in
    debug | release | qa) ;;
    *) echo "--variant must be debug, release or qa" >&2; exit 2 ;;
esac
cd "$(dirname "$0")/.."
jni=android/app/src/main/jniLibs

# Device libraries, linked for 16 KB pages (Android 15+).
RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384" \
    cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 --platform 29 -o "$jni" \
    build --profile "$profile" -p pb-ffi
# cargo-ndk also copies stub cdylibs from iroh crates; keep only ours.
find "$jni" -name 'lib*.so' ! -name 'libpb_ffi.so' -delete

# Bindings come from the host build, because release profiles strip UniFFI metadata.
cargo build -p pb-ffi
cargo run -q -p uniffi-bindgen -- generate --library target/debug/libpb_ffi.so \
    --language kotlin --out-dir android/app/src/main/java

task="assemble$(tr '[:lower:]' '[:upper:]' <<<"${variant:0:1}")${variant:1}"
(cd android && bash gradlew "$task" --no-daemon --console=plain)
echo "APK: android/app/build/outputs/apk/$variant/"
