import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "de.jarvis.app"
    compileSdk = 36

    defaultConfig {
        applicationId = "de.jarvis.app"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Zum Selbst-Installieren genügt die Debug-Signatur.
            signingConfig = signingConfigs.getByName("debug")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildFeatures { compose = true }
    sourceSets["main"].java.srcDir(layout.buildDirectory.dir("generated/uniffi"))
    sourceSets["main"].jniLibs.srcDir(layout.buildDirectory.dir("generated/jniLibs"))
}

kotlin {
    compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
}

// Rust-Kern (jarvis-mobile) für Android bauen und Kotlin-Anbindung erzeugen.
val repoRoot = rootDir.resolve("../..")
val buildRust by tasks.registering(Exec::class) {
    group = "jarvis"
    description = "Baut libjarvis_mobile.so und die Kotlin-Anbindung (UniFFI)"
    workingDir = repoRoot
    val out = layout.buildDirectory.get().asFile
    commandLine(
        "bash", "-c",
        """
        set -e
        cargo ndk -t arm64-v8a -t x86_64 -o "${out}/generated/jniLibs" build --release -p jarvis-mobile
        # Anbindung aus einem ungestrippten Host-Build erzeugen (Metadaten sind plattformunabhängig).
        cargo build -q -p jarvis-mobile --lib
        lib=$(ls target/debug/libjarvis_mobile.so target/debug/libjarvis_mobile.dylib 2>/dev/null | head -1)
        cargo run -q -p uniffi-bindgen -- generate --library "${'$'}lib" \
          --language kotlin --out-dir "${out}/generated/uniffi" --no-format
        """.trimIndent(),
    )
    inputs.dir(repoRoot.resolve("crates"))
    outputs.dir(layout.buildDirectory.dir("generated"))
}
tasks.named("preBuild") { dependsOn(buildRust) }

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2026.06.01")
    implementation(composeBom)
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.activity:activity-compose:1.12.4")
    implementation("androidx.core:core-ktx:1.17.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")
    // UniFFI-Kotlin braucht JNA
    implementation("net.java.dev.jna:jna:5.19.1@aar")
}
