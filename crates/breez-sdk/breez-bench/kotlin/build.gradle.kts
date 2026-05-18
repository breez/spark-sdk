plugins {
    kotlin("jvm") version "2.1.0"
    kotlin("plugin.serialization") version "2.1.0"
    application
}

group = "technology.breez.spark.benchmarks"
version = "1.0-SNAPSHOT"

repositories {
    mavenLocal()
    mavenCentral()
    maven { url = uri("https://mvn.breez.technology/releases") }
}

dependencies {
    // Local KMP bindings published to mavenLocal by `make setup`.
    // Version must match libraryVersion in
    // bindings/langs/kotlin-multiplatform/gradle.properties.
    implementation("technology.breez.spark:breez-sdk-spark-kmp-jvm:0.1.0")
    implementation("com.ionspin.kotlin:bignum:0.3.10")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("net.java.dev.jna:jna:5.18.0")

    // Phase 2: HTTP server.
    implementation("io.ktor:ktor-server-core-jvm:2.3.13")
    implementation("io.ktor:ktor-server-netty-jvm:2.3.13")
    implementation("io.ktor:ktor-server-content-negotiation-jvm:2.3.13")
    implementation("io.ktor:ktor-serialization-kotlinx-json-jvm:2.3.13")
    implementation("ch.qos.logback:logback-classic:1.5.6")

    // Phase 5: MySQL JDBC for sampling INFORMATION_SCHEMA.PROCESSLIST.
    // Used only by the metrics sampler — the SDK has its own MySQL path.
    implementation("com.mysql:mysql-connector-j:9.1.0")

    // HdrHistogram (latency aggregation) will be added with the Phase 6/9
    // aggregator script when those phases land.
}

application {
    mainClass.set("MainKt")
}

kotlin {
    jvmToolchain(17)
}

// `make setup` builds the Rust dylib at <workspace-root>/target/release/
// (libbreez_sdk_spark_bindings.{dylib,so}) but the published JVM JAR doesn't
// bundle it — JNA loads it from jna.library.path at runtime. Point JNA there.
val workspaceRoot: File = rootProject.projectDir.resolve("../../../..").canonicalFile
val nativeLibPath: String = workspaceRoot.resolve("target/release").absolutePath

tasks.named<JavaExec>("run") {
    standardInput = System.`in`
    systemProperty("jna.library.path", nativeLibPath)
}

tasks.jar {
    manifest {
        attributes["Main-Class"] = "MainKt"
    }
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    from(configurations.runtimeClasspath.get().map { if (it.isDirectory) it else zipTree(it) })
}
