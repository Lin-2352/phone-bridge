import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
    id("io.github.takahirom.roborazzi")
}

// The release key lives outside the repo (never committed). Without it a
// release build stops: signed with the debug key it could not update an
// installed release. qa is the release build signed with the debug key.
val releaseKeys = Properties().apply {
    val f = file(System.getenv("PB_KEYSTORE_PROPERTIES") ?: "${System.getProperty("user.home")}/.phonebridge-release/keystore.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}

android {
    namespace = "dev.phonebridge.app"
    compileSdk = 36

    testOptions {
        unitTests.isIncludeAndroidResources = true
    }

    defaultConfig {
        applicationId = "dev.phonebridge.app"
        minSdk = 29
        targetSdk = 36
        versionCode = 5
        versionName = "0.2.0"
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
    }

    signingConfigs {
        if (releaseKeys.containsKey("storeFile")) {
            create("release") {
                storeFile = file(releaseKeys.getProperty("storeFile"))
                storePassword = releaseKeys.getProperty("storePassword")
                keyAlias = releaseKeys.getProperty("keyAlias")
                keyPassword = releaseKeys.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            // R8: drop unused Compose/Kotlin code so less is mapped into memory.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release")
        }
        // The release build, signed so it installs over a debug-signed app: how to try R8's output on a real phone.
        create("qa") {
            initWith(getByName("release"))
            signingConfig = signingConfigs.getByName("debug")
            matchingFallbacks += listOf("release")
        }
    }

    buildFeatures { compose = true }

    // qa is the release build signed with the debug key: same code, so the same helper-less stand-ins.
    sourceSets.getByName("qa") {
        kotlin.srcDir("src/release/java")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

// Haze's dependencies pull a newer Kotlin standard library than this project's compiler (2.2.10) can
// optimise; keep the standard library on the compiler's version.
configurations.configureEach {
    resolutionStrategy.eachDependency {
        if (requested.group == "org.jetbrains.kotlin" && requested.name.startsWith("kotlin-stdlib")) {
            useVersion("2.2.10")
        }
    }
}

dependencies {
    // Screenshot tests only (M6.1): Robolectric runs the Android framework on the JVM for Roborazzi.
    testImplementation("org.robolectric:robolectric:4.17")
    testImplementation("io.github.takahirom.roborazzi:roborazzi:1.77.0")
    testImplementation("io.github.takahirom.roborazzi:roborazzi-compose:1.77.0")
    testImplementation("io.github.takahirom.roborazzi:roborazzi-junit-rule:1.77.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
    // Backdrop blur for glass panels (M6.2; Apache-2.0, pinned).
    implementation("dev.chrisbanes.haze:haze:1.6.10")
    // Unlock gate (M7.1): BiometricPrompt needs a FragmentActivity.
    implementation("androidx.biometric:biometric:1.1.0")
    implementation("androidx.fragment:fragment-ktx:1.9.1")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")

    // UI: Jetpack Compose + Material 3 (design v2).
    implementation(platform("androidx.compose:compose-bom:2025.06.01"))
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.navigation:navigation-compose:2.9.0")
    implementation("androidx.glance:glance-appwidget:1.1.1")
    implementation("androidx.glance:glance-material3:1.1.1")
    implementation("androidx.core:core-ktx:1.16.0")

    // QR scanning: CameraX preview + on-device ZXing decoding (Apache-2.0, no Play Services).
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("androidx.camera:camera-camera2:1.4.2")
    implementation("androidx.camera:camera-lifecycle:1.4.2")
    implementation("androidx.camera:camera-view:1.4.2")
    implementation("com.google.zxing:core:3.5.3")
    // The audio helper's one-time setup: talks to this phone's own adb daemon. Debug builds only
    // (src/debug); release and qa use the no-op stand-ins in src/release. Versions pinned, and
    // the JitPack artifacts' hashes are checked in gradle/verification-metadata.xml.
    debugImplementation("com.github.MuntashirAkon:libadb-android:3.1.1")
    debugImplementation("org.conscrypt:conscrypt-android:2.5.3")
    debugImplementation("org.bouncycastle:bcpkix-jdk15to18:1.81")

    testImplementation("junit:junit:4.13.2")
    // android.jar's org.json is a stub in JVM tests; the real one for Timeline encode/decode.
    testImplementation("org.json:json:20240303")
}

// Configuration and the debug and qa builds stay green without the key; only a
// real release refuses, when it runs.
tasks.configureEach {
    if (name == "assembleRelease" || name == "bundleRelease" || name == "packageRelease") {
        doFirst {
            if (!releaseKeys.containsKey("storeFile")) {
                throw GradleException("Release builds need the release key (see README, Build from source)")
            }
        }
    }
}
