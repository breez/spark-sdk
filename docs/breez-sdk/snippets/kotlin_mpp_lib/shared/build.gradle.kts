plugins {
    alias(libs.plugins.kotlinMultiplatform)
    alias(libs.plugins.androidLibrary)
}

val enableIosTargets = findProperty("enableIosTargets")?.toString()?.toBoolean() ?: true

kotlin {
    androidTarget {
        compilations.all {
            kotlinOptions {
                jvmTarget = "17"
            }
        }
    }

    jvm()

    if (enableIosTargets) {
        listOf(
            iosX64(),
            iosArm64(),
            iosSimulatorArm64()
        ).forEach {
            it.binaries.framework {
                baseName = "shared"
                isStatic = true
            }
        }
    }

    sourceSets {
        commonMain.dependencies {
            implementation(platform("org.kotlincrypto.hash:bom:0.6.0"))
            implementation("org.kotlincrypto.hash:sha2")
            implementation(libs.breez)
            implementation(libs.kotlinx.coroutines.core)
        }
        androidMain.dependencies {
            implementation("androidx.core:core-ktx:1.15.0")
        }
        commonTest.dependencies {
            implementation(libs.kotlin.test)
        }
    }

    tasks.matching { it.name == "compileCommonMainKotlinMetadata" }.all {
        enabled = false
    }
}

android {
    namespace = "com.example.kotlinmpplib"
    compileSdk = 34
    defaultConfig {
        minSdk = 24
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
