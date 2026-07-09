import org.gradle.internal.os.OperatingSystem

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    // Enable once google-services.json is present (FCM):
    // id("com.google.gms.google-services")
}

android {
    namespace = "com.remotecoder"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.remotecoder"
        minSdk = 28 // StrongBox + BiometricPrompt
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            abiFilters += project.property("rcoder.rustAbis").toString().split(",")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    // The Rust cdylib is staged into jniLibs by :cargoNdkBuild below.
    sourceSets["main"].jniLibs.srcDir(layout.buildDirectory.dir("rustJniLibs"))
    // Generated UniFFI Kotlin bindings.
    sourceSets["main"].java.srcDir(layout.buildDirectory.dir("generated/uniffi"))

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { compose = true }
    packaging {
        resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.09.02")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.activity:activity-compose:1.9.2")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.6")
    implementation("androidx.lifecycle:lifecycle-service:2.8.6")
    implementation("androidx.navigation:navigation-compose:2.8.1")

    // UniFFI Kotlin bindings runtime.
    implementation("net.java.dev.jna:jna:5.14.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")

    // Biometric-gated keystore.
    implementation("androidx.biometric:biometric:1.2.0-alpha05")

    // QR scanning for pairing (CameraX + ML Kit barcode).
    implementation("androidx.camera:camera-camera2:1.3.4")
    implementation("androidx.camera:camera-lifecycle:1.3.4")
    implementation("androidx.camera:camera-view:1.3.4")
    implementation("com.google.mlkit:barcode-scanning:17.3.0")

    // FCM (needs google-services.json + the plugin above).
    implementation(platform("com.google.firebase:firebase-bom:33.3.0"))
    implementation("com.google.firebase:firebase-messaging")

    implementation("com.google.code.gson:gson:2.11.0")
}

// --- Rust: cross-build the engine cdylib + generate Kotlin bindings ------
// Mirrors `just android-so`; requires cargo-ndk + ANDROID_NDK_HOME. Wired as
// a preBuild dependency so a normal `./gradlew assembleDebug` produces the
// .so and bindings automatically (DESIGN.md §11).

val rustRoot = rootProject.projectDir.parentFile // repo root
val abis = project.property("rcoder.rustAbis").toString().split(",")

val cargoNdkBuild by tasks.registering(Exec::class) {
    group = "rust"
    description = "Cross-build libremotecoder_engine.so into jniLibs via cargo-ndk"
    workingDir = rustRoot
    val jniOut = layout.buildDirectory.dir("rustJniLibs").get().asFile
    val args = mutableListOf("ndk")
    abis.forEach { args += listOf("-t", it) }
    args += listOf("-o", jniOut.absolutePath, "build", "-p", "engine-ffi")
    if (project.hasProperty("release")) args += "--release"
    commandLine(listOf(if (OperatingSystem.current().isWindows) "cargo.exe" else "cargo") + args)
}

val uniffiBindgen by tasks.registering(Exec::class) {
    group = "rust"
    description = "Generate UniFFI Kotlin bindings from the built cdylib"
    dependsOn(cargoNdkBuild)
    workingDir = rustRoot
    val outDir = layout.buildDirectory.dir("generated/uniffi").get().asFile
    // Any built ABI's .so carries the same metadata; use the first.
    val so = layout.buildDirectory
        .file("rustJniLibs/${abis.first()}/libremotecoder_engine.so").get().asFile
    commandLine(
        "cargo", "run", "-q", "-p", "engine-ffi", "--bin", "uniffi-bindgen", "--",
        "generate", "--library", so.absolutePath, "--language", "kotlin", "--out-dir", outDir.absolutePath
    )
}

tasks.named("preBuild") { dependsOn(uniffiBindgen) }
