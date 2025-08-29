plugins {
    alias(libs.plugins.kotlinMultiplatform)
    alias(libs.plugins.androidLibrary)
}

kotlin {
    androidTarget {
        compilations.all {
            kotlinOptions {
                jvmTarget = "17"
            }
        }
    }

    jvm()

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

    sourceSets {
        commonMain.dependencies {
            implementation(libs.breez)
            implementation(libs.kotlinx.coroutines.core)
        }
        androidMain.dependencies {
            implementation("androidx.core:core-ktx:+")
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
