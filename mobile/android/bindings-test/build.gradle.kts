// Prüft die erzeugte Kotlin-Anbindung auf der JVM gegen den echten Rust-Kern
// (Host-Build) und einen echt laufenden jarvisd – ohne Emulator.
plugins {
    id("org.jetbrains.kotlin.jvm")
}

val repoRoot = rootDir.resolve("../..")

kotlin { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }
java { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }

sourceSets["main"].kotlin.srcDir(project(":app").layout.buildDirectory.dir("generated/uniffi"))

dependencies {
    implementation("net.java.dev.jna:jna:5.19.1")
    testImplementation(kotlin("test"))
}

val buildHost by tasks.registering(Exec::class) {
    workingDir = repoRoot
    commandLine("cargo", "build", "-q", "-p", "jarvis-mobile", "-p", "jarvis-node")
}

tasks.named("compileKotlin") { dependsOn(":app:buildRust") }

tasks.test {
    dependsOn(buildHost)
    useJUnitPlatform()
    systemProperty("jna.library.path", repoRoot.resolve("target/debug").absolutePath)
    systemProperty("jarvis.bin", repoRoot.resolve("target/debug").absolutePath)
    testLogging { events("passed", "failed"); showStandardStreams = true; exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL }
}
