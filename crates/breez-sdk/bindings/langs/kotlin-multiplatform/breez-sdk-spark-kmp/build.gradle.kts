plugins {
    kotlin("multiplatform")
    id("com.android.library")
    id("maven-publish")
}

apply(plugin = "kotlinx-atomicfu")

kotlin {
    // Enable the default target hierarchy
    applyDefaultHierarchyTemplate()

    androidTarget {
        compilations.all {
            kotlinOptions {
                jvmTarget = JavaVersion.VERSION_17.majorVersion
            }
        }

        publishLibraryVariants("release")
    }

    jvm {
        compilations.all {
            kotlinOptions.jvmTarget = JavaVersion.VERSION_17.majorVersion
        }
    }

    listOf(
        iosX64(),
        iosArm64(),
        iosSimulatorArm64()
    ).forEach {
        val platform = when (it.targetName) {
            "iosSimulatorArm64" -> "ios_simulator_arm64"
            "iosArm64" -> "ios_arm64"
            "iosX64" -> "ios_x64"
            else -> error("Unsupported target $name")
        }

        it.compilations["main"].cinterops {
            create("breezSdkCommonCInterop") {
                defFile(project.file("src/nativeInterop/cinterop/breez_sdk_common.def"))
                includeDirs(project.file("src/nativeInterop/cinterop/headers/breez_sdk_common"), project.file("src/lib/$platform"))
            }
            create("breezSdkSparkCInterop") {
                defFile(project.file("src/nativeInterop/cinterop/breez_sdk_spark.def"))
                includeDirs(project.file("src/nativeInterop/cinterop/headers/breez_sdk_spark"), project.file("src/lib/$platform"))
            }
            create("breezSdkSparkBindingsCInterop") {
                defFile(project.file("src/nativeInterop/cinterop/breez_sdk_spark_bindings.def"))
                includeDirs(project.file("src/nativeInterop/cinterop/headers/breez_sdk_spark_bindings"), project.file("src/lib/$platform"))
            }
        }
    }

    sourceSets {
        all {
            languageSettings.apply {
                optIn("kotlinx.cinterop.ExperimentalForeignApi")
            }
        }

        val commonMain by getting {
            dependencies {
                implementation("com.squareup.okio:okio:3.6.0")
                implementation("org.jetbrains.kotlinx:kotlinx-datetime:0.5.0")
                implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.0")
                implementation("com.ionspin.kotlin:bignum:0.3.10")
            }
        }

        val jvmMain by getting {
            dependsOn(commonMain)
            dependencies {
                implementation("net.java.dev.jna:jna:5.13.0")
            }
        }

        val androidMain by getting {
            dependsOn(commonMain)
            dependencies {
                implementation("net.java.dev.jna:jna:5.13.0@aar")
                implementation("org.jetbrains.kotlinx:atomicfu:0.23.1")
                implementation("androidx.annotation:annotation:1.7.1")
            }
        }
    }
}

android {
    namespace = "technology.breez.spark"
    compileSdk = 34

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

val libraryVersion: String by project

group = "technology.breez.spark"
version = libraryVersion

publishing {
    repositories {
        maven {
            name = "breezReposilite"
            url = uri("https://mvn.breez.technology/releases")
            credentials(PasswordCredentials::class)
            authentication {
                create<BasicAuthentication>("basic")
            }
        }
    }

    publications {
        this.forEach {
            (it as MavenPublication).apply {
                pom {
                    name.set("breez-sdk-spark-kmp")
                    description.set("The Breez Spark SDK enables mobile developers to integrate Spark into their apps with a very shallow learning curve.")
                    url.set("https://breez.technology")
                    licenses {
                        license {
                            name.set("MIT")
                            url.set("https://github.com/breez/spark-sdk/blob/main/LICENSE")
                        }
                    }
                    scm {
                        connection.set("scm:git:github.com/breez/spark-sdk.git")
                        developerConnection.set("scm:git:ssh://github.com/breez/spark-sdk.git")
                        url.set("https://github.com/breez/spark-sdk")
                    }
                }
            }
        }
    }
}