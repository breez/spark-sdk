# How to run

```
cd BreezSDKExamples

swift package clean

swift build

swift run
```

## To reference locally-built bindings:

- In the local `crates/breez-sdk/bindings` run `make package-xcframework`

## To reference published bindings:

- Edit `Package.swift`
  - Follow the instructions indicated by "To use a published version of BreezSdkSpark"
