# How to run

```
cd BreezSDKExamples

swift package clean

swift build

swift run
```

## To reference locally-built bindings:

- In the local `spark-sdk/crates/breez-sdk/bindings` run `make package-xcframework`
- Edit `Package.swift`
  - Follow the instructions indicated by "To use a local version of BreezSdkSpark"
