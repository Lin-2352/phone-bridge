# UniFFI bindings call into Rust through JNA by reflection: keep JNA and the
# generated bindings (structures, callbacks) exactly as they are.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-dontwarn java.awt.**
-keep class uniffi.** { *; }
# JNI entry point called from Rust by name.
-keep class dev.phonebridge.app.NativeInit { *; }
# Callback interfaces implemented in Kotlin and invoked from Rust.
-keep class dev.phonebridge.app.KeystoreWrapper { *; }
