UNAME := $(shell uname)
OS_NAME=$(shell uname | tr '[:upper:]' '[:lower:]')
FFI_DIR := ffi/
BIN_NAME := libbreez_sdk_spark_bindings
TARGET_DIR := ../../../target/
RELEASE_DIR := $(TARGET_DIR)release

ifeq ($(UNAME), Darwin)
	CLANG_PREFIX += AR=$(shell brew --prefix llvm)/bin/llvm-ar CC=$(shell brew --prefix llvm)/bin/clang
	BIN_EXT := dylib
else ifeq ($(UNAME), Linux)
	BIN_EXT := so
endif

BIN_PATH := $(RELEASE_DIR)/$(BIN_NAME).$(BIN_EXT)
INSTALL_PREFIX := CARGO_TARGET_DIR="$(TARGET_DIR)"

bindings-kotlin: build-release install-uniffi-bindgen-gobley
	gobley-uniffi-bindgen --library -o $(FFI_DIR)kotlin \
		--crate-configs breez_sdk_spark_bindings=./uniffi.toml \
		--crate-configs breez_sdk_spark=../core/uniffi.toml \
		--crate-configs breez_sdk_common=../common/uniffi.toml \
		$(BIN_PATH)

bindings-python: build-release
	cargo run --features=uniffi/cli --bin uniffi-bindgen generate --library $(BIN_PATH) --no-format --language python -o $(FFI_DIR)python

bindings-swift: build-release
	cargo run --features=uniffi/cli --bin uniffi-bindgen generate --library $(BIN_PATH) --no-format --language swift -o $(FFI_DIR)swift

build-release:
	cargo build --release

build-release-target-%: install-target-%
	cargo build --release --target $*

install-uniffi-bindgen-gobley:
	$(INSTALL_PREFIX) cargo install gobley-uniffi-bindgen --git https://github.com/breez/gobley --rev f920c275e4f60e22080a6cd280d9562d64eb6ac9

install-target-%:
	$(CLANG_PREFIX) rustup target add $*

## Android
check-ndk:
	@if [ ! -d "${ANDROID_NDK_HOME}" ] ; then \
		echo "Error: Please, set the ANDROID_NDK_HOME env variable to point to your NDK folder" ; \
		exit 1 ; \
	fi

build-ndk-release-target-%: check-ndk install-target-%
	cargo ndk -t $* -o $(FFI_DIR)kotlin/jniLibs build --release --link-libcxx-shared

build-android: install-uniffi-bindgen-gobley build-ndk-release-target-aarch64-linux-android build-ndk-release-target-armv7-linux-androideabi build-ndk-release-target-i686-linux-android build-ndk-release-target-x86_64-linux-android
	gobley-uniffi-bindgen --library -o $(FFI_DIR)kotlin \
		--crate-configs breez_sdk_spark_bindings=./uniffi.toml \
		--crate-configs breez_sdk_spark=../core/uniffi.toml \
		--crate-configs breez_sdk_common=../common/uniffi.toml \
		$(TARGET_DIR)/aarch64-linux-android/release/$(BIN_NAME).so

package-android: build-android
	rm $(FFI_DIR)kotlin/jniLibs/**/*common.so  $(FFI_DIR)kotlin/jniLibs/**/*spark.so
	cp -r $(FFI_DIR)kotlin/jniLibs langs/android/lib/src/main
	cp -r $(FFI_DIR)kotlin/main/kotlin/breez_sdk_spark langs/android/lib/src/main/kotlin/
	cd langs/android && ./gradlew assemble
	mkdir -p $(FFI_DIR)android
	cp langs/android/lib/build/outputs/aar/lib-release.aar $(FFI_DIR)android

## Apple
build-ios-universal: build-release-target-aarch64-apple-ios build-release-target-x86_64-apple-ios build-release-target-aarch64-apple-ios-sim
	mkdir -p $(TARGET_DIR)/ios-universal/release
	mkdir -p $(TARGET_DIR)/ios-universal-sim/release
	# build universal lib for arm device and x86 sim
	lipo -create -output $(TARGET_DIR)/ios-universal/release/$(BIN_NAME).a $(TARGET_DIR)/aarch64-apple-ios/release/$(BIN_NAME).a $(TARGET_DIR)/x86_64-apple-ios/release/$(BIN_NAME).a
	# build universal lib for arm sim and x86 sim
	lipo -create -output $(TARGET_DIR)/ios-universal-sim/release/$(BIN_NAME).a $(TARGET_DIR)/aarch64-apple-ios-sim/release/$(BIN_NAME).a $(TARGET_DIR)/x86_64-apple-ios/release/$(BIN_NAME).a

build-darwin-universal: install-target-aarch64-apple-darwin install-target-x86_64-apple-darwin
	mkdir -p $(TARGET_DIR)/darwin-universal/release
	cargo lipo --release --targets aarch64-apple-darwin
	cargo lipo --release --targets x86_64-apple-darwin
	lipo -create -output $(TARGET_DIR)/darwin-universal/release/$(BIN_NAME).dylib $(TARGET_DIR)/aarch64-apple-darwin/release/$(BIN_NAME).dylib $(TARGET_DIR)/x86_64-apple-darwin/release/$(BIN_NAME).dylib
	lipo -create -output $(TARGET_DIR)/darwin-universal/release/$(BIN_NAME).a $(TARGET_DIR)/aarch64-apple-darwin/release/$(BIN_NAME).a $(TARGET_DIR)/x86_64-apple-darwin/release/$(BIN_NAME).a

package-xcframework: build-ios-universal build-darwin-universal
	cargo run --features=uniffi/cli --bin uniffi-bindgen generate --library $(BIN_PATH) --no-format --language swift -o $(FFI_DIR)swift
	mkdir -p langs/swift/Sources/BreezSdkSpark
	mv $(FFI_DIR)swift/*.swift langs/swift/Sources/BreezSdkSpark/
	# WA pre 0.29.0: Fix redeclaration of protocol UniffiForeignFutureTask https://github.com/mozilla/uniffi-rs/pull/2341
	find langs/swift/Sources/BreezSdkSpark -name "*.swift" -exec sed -i '' 's/protocol UniffiForeignFutureTask/fileprivate protocol UniffiForeignFutureTask/g' {} \;
	cp $(FFI_DIR)swift/breez_sdk_*FFI.h langs/swift/breez_sdk_sparkFFI.xcframework/ios-arm64/breez_sdk_sparkFFI.framework/Headers
	cp $(FFI_DIR)swift/breez_sdk_*FFI.h langs/swift/breez_sdk_sparkFFI.xcframework/ios-arm64_x86_64-simulator/breez_sdk_sparkFFI.framework/Headers
	cp $(FFI_DIR)swift/breez_sdk_*FFI.h langs/swift/breez_sdk_sparkFFI.xcframework/macos-arm64_x86_64/breez_sdk_sparkFFI.framework/Headers
	cp $(TARGET_DIR)/aarch64-apple-ios/release/$(BIN_NAME).a langs/swift/breez_sdk_sparkFFI.xcframework/ios-arm64/breez_sdk_sparkFFI.framework/breez_sdk_sparkFFI
	cp $(TARGET_DIR)/ios-universal-sim/release/$(BIN_NAME).a langs/swift/breez_sdk_sparkFFI.xcframework/ios-arm64_x86_64-simulator/breez_sdk_sparkFFI.framework/breez_sdk_sparkFFI
	cp $(TARGET_DIR)/darwin-universal/release/$(BIN_NAME).a langs/swift/breez_sdk_sparkFFI.xcframework/macos-arm64_x86_64/breez_sdk_sparkFFI.framework/breez_sdk_sparkFFI
	cd langs/swift && swift build
