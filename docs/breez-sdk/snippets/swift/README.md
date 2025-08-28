# How to run
```
cd BreezSDKExamples

swift package clean

swift build

swift run
``` 

## To reference locally-built bindings:
- In the local `breez-sdk-liquid/lib/bindings` run `make swift-bindings`
- Edit `Package.swift`
  - Follow the instructions indicated by "To use a local version of breez-sdk-liquid"