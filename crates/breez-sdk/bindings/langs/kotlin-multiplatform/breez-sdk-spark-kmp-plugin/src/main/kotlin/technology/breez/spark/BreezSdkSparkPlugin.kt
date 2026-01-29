package technology.breez.spark

import org.gradle.api.Plugin
import org.gradle.api.Project
import org.jetbrains.kotlin.gradle.dsl.KotlinMultiplatformExtension
import org.jetbrains.kotlin.gradle.plugin.mpp.KotlinNativeTarget
import org.jetbrains.kotlin.gradle.plugin.mpp.Framework

class BreezSdkSparkPlugin : Plugin<Project> {
    override fun apply(project: Project) {
        project.afterEvaluate { proj ->
            val kotlin = proj.extensions.findByType(KotlinMultiplatformExtension::class.java)
                ?: return@afterEvaluate

            val buildDir = System.getenv("BUILD_DIR") ?: return@afterEvaluate
            val platformName = System.getenv("PLATFORM_NAME") ?: return@afterEvaluate

            val searchPaths = mutableListOf<String>()

            // xcframework slice based on target platform
            val slice = if (platformName == "iphonesimulator")
                "ios-arm64_x86_64-simulator" else "ios-arm64"

            // SPM artifacts path
            val derivedDataRoot = buildDir.substringBefore("/Build/")
            val spmSearchPath = "$derivedDataRoot/SourcePackages/artifacts/" +
                "breez-sdk-spark-swift/breez_sdk_sparkFFI/" +
                "breez_sdk_sparkFFI.xcframework/$slice"
            if (java.io.File(spmSearchPath).exists()) {
                searchPaths.add(spmSearchPath)
            }

            // CocoaPods path (PODS_ROOT is set by Xcode from CocoaPods)
            val podsRoot = System.getenv("PODS_ROOT")
            if (podsRoot != null) {
                val podsXcfwSlice = java.io.File(
                    "$podsRoot/breez_sdk_sparkFFI/breez_sdk_sparkFFI.xcframework/$slice"
                )
                if (podsXcfwSlice.exists()) {
                    searchPaths.add(podsXcfwSlice.absolutePath)
                }
            }

            // Xcode build products (covers both SPM and CocoaPods at link time)
            val builtProductsDir = System.getenv("BUILT_PRODUCTS_DIR")
            if (builtProductsDir != null && java.io.File(builtProductsDir).exists()) {
                searchPaths.add(builtProductsDir)
            }

            if (searchPaths.isEmpty()) {
                proj.logger.warn(
                    "breez-sdk-spark: Could not find breez_sdk_sparkFFI framework. " +
                    "Install it via Swift Package Manager or CocoaPods. " +
                    "See: https://breez.technology"
                )
                return@afterEvaluate
            }

            kotlin.targets.withType(KotlinNativeTarget::class.java).all { target ->
                target.binaries.withType(Framework::class.java).all { framework ->
                    searchPaths.forEach { path ->
                        framework.linkerOpts("-F", path)
                    }
                }
            }
        }
    }
}
