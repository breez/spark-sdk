# Android/Kotlin

We recommend integrating the Breez SDK as Gradle dependency from [our Maven repository](https://mvn.breez.technology/#/releases).

To do so, add the following to your Gradle dependencies:

```gradle
repositories {
  maven {
      url("https://mvn.breez.technology/releases")
  }
}

dependencies {
  implementation("breez_sdk_spark:bindings-android:{VERSION}")
}
```

## Example App

For a full working example app, see the [Kotlin CLI example app](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/kotlin-multiplatform).
