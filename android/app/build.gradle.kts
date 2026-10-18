plugins {
    id("com.android.application")
}

android {
    namespace = "dev.phonebridge.app"
    compileSdk = 36

    defaultConfig {
        applicationId = "dev.phonebridge.app"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.0.1"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")
    // QR scanning: CameraX preview + on-device ZXing decoding (Apache-2.0, no Play Services).
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("androidx.camera:camera-camera2:1.4.2")
    implementation("androidx.camera:camera-lifecycle:1.4.2")
    implementation("androidx.camera:camera-view:1.4.2")
    implementation("com.google.zxing:core:3.5.3")
}
