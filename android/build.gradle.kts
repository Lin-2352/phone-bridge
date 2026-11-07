plugins {
    id("com.android.application") version "9.4.1" apply false
    // Must match the Kotlin version bundled with AGP 9.4.1 (2.2.10).
    id("org.jetbrains.kotlin.plugin.compose") version "2.2.10" apply false
    // Screenshot tests (M6.1): Roborazzi captures the Compose screens on the JVM.
    id("io.github.takahirom.roborazzi") version "1.77.0" apply false
}
