# Kotlin Multiplatform

We recommend integrating the Breez SDK as Gradle dependency from [our Maven repository](https://mvn.breez.technology/#/releases).

Add the Breez Maven repository to your `settings.gradle.kts`:

```gradle
pluginManagement {
    repositories {
        maven("https://mvn.breez.technology/releases")
    }
}

dependencyResolutionManagement {
    repositories {
        maven("https://mvn.breez.technology/releases")
    }
}
```

Then add the dependency in your module's `build.gradle.kts`:

```gradle
kotlin {
    sourceSets {
        commonMain.dependencies {
            implementation("technology.breez.spark:breez-sdk-spark-kmp:{VERSION}")
        }
    }
}
```

## iOS Setup

When targeting iOS, you must also install the native binary framework. This is the same framework used by the Swift SDK and can be installed via Swift Package Manager or CocoaPods. The Gradle plugin automatically configures the framework search path from Xcode's build environment.

Add the Gradle plugin to your module's `build.gradle.kts` and update the iOS framework binaries to use a dynamic framework:

```gradle
plugins {
    id("technology.breez.spark.kmp") version "{VERSION}"
}

kotlin {
    listOf(
        iosArm64(),
        iosSimulatorArm64(),
        iosX64(),
    ).forEach {
        it.binaries.framework {
            baseName = "shared"
            isStatic = false
        }
    }
}
```

### Swift Package Manager

#### Installation via Xcode

Via `File > Add Packages...`, add

```
https://github.com/breez/breez-sdk-spark-swift.git
```

as a package dependency in Xcode.

#### Installation via Swift Package Manifest

Add the following to the dependencies array of your `Package.swift`:

``` swift
.package(url: "https://github.com/breez/breez-sdk-spark-swift.git", from: "{VERSION}"),
```

### CocoaPods

Add the Breez SDK to your `Podfile` like so and run `pod install`:

``` ruby
target '<YourApp>' do
  use_frameworks!
  pod 'breez_sdk_sparkFFI'
end
```
