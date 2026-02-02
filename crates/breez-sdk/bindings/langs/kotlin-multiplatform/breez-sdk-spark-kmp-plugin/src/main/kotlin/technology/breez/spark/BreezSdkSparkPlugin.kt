package technology.breez.spark

import org.gradle.api.Plugin
import org.gradle.api.Project
import org.jetbrains.kotlin.gradle.dsl.KotlinMultiplatformExtension
import org.jetbrains.kotlin.gradle.plugin.KotlinMultiplatformPluginWrapper
import org.jetbrains.kotlin.gradle.plugin.mpp.KotlinNativeTarget
import org.jetbrains.kotlin.gradle.plugin.mpp.Framework

/**
 * Gradle plugin that auto-configures iOS framework search paths for breez_sdk_sparkFFI.
 *
 * The plugin detects the framework location from Xcode environment variables (BUILD_DIR,
 * PLATFORM_NAME) and searches SPM artifacts, CocoaPods, and Xcode build products directories.
 *
 * ## Properties
 *
 * - **breezSdkSparkFrameworkPath** — Set this project property to override the automatic
 *   framework search with an explicit path to the directory containing
 *   `breez_sdk_sparkFFI.framework`. Example usage in `gradle.properties`:
 *   ```
 *   breezSdkSparkFrameworkPath=/path/to/breez_sdk_sparkFFI.framework/..
 *   ```
 *   Or via command line:
 *   ```
 *   ./gradlew build -PbreezSdkSparkFrameworkPath=/path/to/framework/dir
 *   ```
 */
class BreezSdkSparkPlugin : Plugin<Project> {
    override fun apply(project: Project) {
        project.plugins.withType(KotlinMultiplatformPluginWrapper::class.java) {
            val kotlin = project.extensions.findByType(KotlinMultiplatformExtension::class.java)
                ?: return@withType

            val buildDir = System.getenv("BUILD_DIR")
            if (buildDir == null) {
                project.logger.debug("breez-sdk-spark: BUILD_DIR not set, skipping framework search (expected outside Xcode)")
                return@withType
            }
            val platformName = System.getenv("PLATFORM_NAME")
            if (platformName == null) {
                project.logger.debug("breez-sdk-spark: PLATFORM_NAME not set, skipping framework search (expected outside Xcode)")
                return@withType
            }

            val searchPaths = mutableListOf<String>()

            // Check for explicit override via project property
            val overridePath = project.findProperty("breezSdkSparkFrameworkPath") as? String
            if (overridePath != null) {
                if (java.io.File(overridePath).exists()) {
                    searchPaths.add(overridePath)
                } else {
                    project.logger.warn("breez-sdk-spark: breezSdkSparkFrameworkPath set to '$overridePath' but path does not exist")
                }
            }

            if (searchPaths.isEmpty()) {
                // xcframework slice based on target platform
                // The simulator slice is a universal binary (arm64 + x86_64), so both
                // iosSimulatorArm64 and iosX64 targets are covered by the same slice.
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
            }

            if (searchPaths.isEmpty()) {
                project.logger.warn(
                    "breez-sdk-spark: Could not find breez_sdk_sparkFFI framework. " +
                    "Install it via Swift Package Manager or CocoaPods, " +
                    "or set the breezSdkSparkFrameworkPath project property. " +
                    "See: https://breez.technology"
                )
                return@withType
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
