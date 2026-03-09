plugins {
    kotlin("jvm") version "2.1.0"
    application
}

group = "technology.breez.spark.cli"
version = "1.0-SNAPSHOT"

repositories {
    mavenLocal()
    mavenCentral()
    maven { url = uri("https://mvn.breez.technology/releases") }
}

dependencies {
    // Uses local bindings published to mavenLocal by `make setup`.
    // Version must match libraryVersion in langs/kotlin-multiplatform/gradle.properties.
    implementation("technology.breez.spark:breez-sdk-spark-kmp-jvm:0.1.0")
    implementation("com.ionspin.kotlin:bignum:0.3.10")
    implementation("com.google.code.gson:gson:2.11.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.0")
    implementation("net.java.dev.jna:jna:5.18.0")
    implementation("org.jline:jline:3.26.3")
}

application {
    mainClass.set("MainKt")
}

kotlin {
    jvmToolchain(17)
}

tasks.named<JavaExec>("run") {
    standardInput = System.`in`
}

tasks.jar {
    manifest {
        attributes["Main-Class"] = "MainKt"
    }
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    from(configurations.runtimeClasspath.get().map { if (it.isDirectory) it else zipTree(it) })
}
