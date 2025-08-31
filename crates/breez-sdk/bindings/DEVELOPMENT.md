# Development guide - Bindings crate
This crate is responsible for building UniFFI bindings.

## Prerequisites
To build you need to first install:
- [Protobuf](https://protobuf.dev/installation/)
```bash
brew install protobuf
```

Set the ANDROID_NDK_HOME env variable to your Android NDK directory:
```bash
export ANDROID_NDK_HOME=<your android ndk directory>
```

## Building
To build bindings for individual languages please see the available [Makefile tasks](crates/breez-sdk/bindings/makefile). For example:
```bash
make bindings-python
```
