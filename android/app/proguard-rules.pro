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
# WorkManager (pulled in by the Glance widget) creates its Room database
# through reflection on the generated WorkDatabase_Impl. R8 full mode keeps
# the class but removes its default constructor, so the app crashed at start
# ("Failed to create an instance of androidx.work.impl.WorkDatabase"). Keep it
# for every Room database.
-keep class * extends androidx.room.RoomDatabase { <init>(); }

# The audio helper is started by name with app_process; nothing in the app calls its entry point.
-keep class dev.phonebridge.helper.Main { public static void main(java.lang.String[]); }
