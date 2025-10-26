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

bindings-csharp: build-release install-uniffi-bindgen-cs
	uniffi-bindgen-cs --library $(BIN_PATH) --no-format -o $(FFI_DIR)csharp -c ./uniffi.toml

bindings-kotlin: build-release install-uniffi-bindgen-gobley
	gobley-uniffi-bindgen --library -o $(FFI_DIR)kotlin \
		--crate-configs breez_sdk_spark_bindings=./uniffi.toml \
		--crate-configs breez_sdk_spark=../core/uniffi.toml \
		--crate-configs breez_sdk_common=../common/uniffi.toml \
		$(BIN_PATH)

bindings-golang: build-release install-uniffi-bindgen-golang
	uniffi-bindgen-go --library $(BIN_PATH) -o $(FFI_DIR)golang -c ./uniffi.toml

bindings-python: build-release
	cargo run --features=uniffi/cli --bin uniffi-bindgen generate --library $(BIN_PATH) --no-format --language python -o $(FFI_DIR)python

bindings-swift: build-release
	cargo run --features=uniffi/cli --bin uniffi-bindgen generate --library $(BIN_PATH) --no-format --language swift -o $(FFI_DIR)swift

build-release:
	cargo build --release

build-release-target-%: install-target-%
	cargo build --release --target $*

install-uniffi-bindgen-cs:
	$(INSTALL_PREFIX) cargo install uniffi-bindgen-cs --git https://github.com/NordSecurity/uniffi-bindgen-cs --tag v0.9.2+v0.28.3

install-uniffi-bindgen-gobley:
	$(INSTALL_PREFIX) cargo install gobley-uniffi-bindgen --git https://github.com/breez/gobley --rev 72753910f54a157de2f84b78dc6f78415e28d4f2

install-uniffi-bindgen-golang:
	$(INSTALL_PREFIX) cargo install uniffi-bindgen-go --git https://github.com/breez/uniffi-bindgen-go --rev 8b8c58f78b2dc16a252f325da91ce409f409fe3c

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
		$(TARGET_DIR)aarch64-linux-android/release/$(BIN_NAME).so

package-android: build-android
	rm $(FFI_DIR)kotlin/jniLibs/**/*common.so  $(FFI_DIR)kotlin/jniLibs/**/*spark.so
	cp -r $(FFI_DIR)kotlin/jniLibs langs/android/lib/src/main
	cp -r $(FFI_DIR)kotlin/main/kotlin/breez_sdk_spark langs/android/lib/src/main/kotlin/
	cd langs/android && ./gradlew assemble
	mkdir -p $(FFI_DIR)android
	cp langs/android/lib/build/outputs/aar/lib-release.aar $(FFI_DIR)android

## Apple
build-ios-universal: build-release-target-aarch64-apple-ios build-release-target-x86_64-apple-ios build-release-target-aarch64-apple-ios-sim
	mkdir -p $(TARGET_DIR)ios-universal/release
	mkdir -p $(TARGET_DIR)ios-universal-sim/release
	# build universal lib for arm device and x86 sim
	lipo -create -output $(TARGET_DIR)ios-universal/release/$(BIN_NAME).a $(TARGET_DIR)aarch64-apple-ios/release/$(BIN_NAME).a $(TARGET_DIR)x86_64-apple-ios/release/$(BIN_NAME).a
	# build universal lib for arm sim and x86 sim
	lipo -create -output $(TARGET_DIR)ios-universal-sim/release/$(BIN_NAME).a $(TARGET_DIR)aarch64-apple-ios-sim/release/$(BIN_NAME).a $(TARGET_DIR)x86_64-apple-ios/release/$(BIN_NAME).a

build-darwin-universal: build-aarch64-apple-darwin build-x86_64-apple-darwin
	mkdir -p $(TARGET_DIR)darwin-universal/release
	lipo -create -output $(TARGET_DIR)darwin-universal/release/$(BIN_NAME).dylib $(TARGET_DIR)aarch64-apple-darwin/release/$(BIN_NAME).dylib $(TARGET_DIR)x86_64-apple-darwin/release/$(BIN_NAME).dylib
	lipo -create -output $(TARGET_DIR)darwin-universal/release/$(BIN_NAME).a $(TARGET_DIR)aarch64-apple-darwin/release/$(BIN_NAME).a $(TARGET_DIR)x86_64-apple-darwin/release/$(BIN_NAME).a

build-aarch64-apple-darwin: install-target-aarch64-apple-darwin
	cargo lipo --release --targets aarch64-apple-darwin

build-x86_64-apple-darwin: install-target-x86_64-apple-darwin
	cargo lipo --release --targets x86_64-apple-darwin

package-xcframework-no-binaries: bindings-swift
	mkdir -p langs/swift/Sources/BreezSdkSpark
	mv $(FFI_DIR)swift/*.swift langs/swift/Sources/BreezSdkSpark/
	# WA pre 0.29.0: Fix redeclaration of protocol UniffiForeignFutureTask https://github.com/mozilla/uniffi-rs/pull/2341
	find langs/swift/Sources/BreezSdkSpark -name "*.swift" -exec sed -i '' 's/protocol UniffiForeignFutureTask/fileprivate protocol UniffiForeignFutureTask/g' {} \;
	cp $(FFI_DIR)swift/breez_sdk_*FFI.h langs/swift/breez_sdk_sparkFFI.xcframework/ios-arm64/breez_sdk_sparkFFI.framework/Headers
	cp $(FFI_DIR)swift/breez_sdk_*FFI.h langs/swift/breez_sdk_sparkFFI.xcframework/ios-arm64_x86_64-simulator/breez_sdk_sparkFFI.framework/Headers
	cp $(FFI_DIR)swift/breez_sdk_*FFI.h langs/swift/breez_sdk_sparkFFI.xcframework/macos-arm64_x86_64/breez_sdk_sparkFFI.framework/Headers
	cd langs/swift && swift build

package-xcframework: build-ios-universal build-darwin-universal package-xcframework-no-binaries
	cp $(TARGET_DIR)aarch64-apple-ios/release/$(BIN_NAME).a langs/swift/breez_sdk_sparkFFI.xcframework/ios-arm64/breez_sdk_sparkFFI.framework/breez_sdk_sparkFFI
	cp $(TARGET_DIR)ios-universal-sim/release/$(BIN_NAME).a langs/swift/breez_sdk_sparkFFI.xcframework/ios-arm64_x86_64-simulator/breez_sdk_sparkFFI.framework/breez_sdk_sparkFFI
	cp $(TARGET_DIR)darwin-universal/release/$(BIN_NAME).a langs/swift/breez_sdk_sparkFFI.xcframework/macos-arm64_x86_64/breez_sdk_sparkFFI.framework/breez_sdk_sparkFFI

## Kotlin Multiplatform
KMP_BASE := langs/kotlin-multiplatform/breez-sdk-spark-kmp/src
KOTLIN_PACKAGE := kotlin/breez_sdk_spark

# Build only the minimum needed for uniffi-bindgen (one Android arch)
build-kotlin-multiplatform-bindgen-only: install-uniffi-bindgen-gobley build-ndk-release-target-aarch64-linux-android
	gobley-uniffi-bindgen --library -o $(FFI_DIR)kmp \
		--crate-configs breez_sdk_spark_bindings=./uniffi.kotlin-multiplatform.toml \
		--crate-configs breez_sdk_spark=../core/uniffi.kotlin-multiplatform.toml \
		--crate-configs breez_sdk_common=../common/uniffi.kotlin-multiplatform.toml \
		$(TARGET_DIR)aarch64-linux-android/release/$(BIN_NAME).so

# Build all binaries + run bindgen
build-kotlin-multiplatform: build-kotlin-multiplatform-bindgen-only build-ios-universal build-ndk-release-target-armv7-linux-androideabi build-ndk-release-target-i686-linux-android build-ndk-release-target-x86_64-linux-android

# Package with dummy binaries (useful for testing)
package-kotlin-multiplatform-dummy-binaries: build-kotlin-multiplatform-bindgen-only
	mkdir -p $(KMP_BASE)/lib/ios-arm64 $(KMP_BASE)/lib/ios-simulator-arm64 $(KMP_BASE)/lib/ios-simulator-x64
	cp -r $(FFI_DIR)kmp/* $(KMP_BASE)/
	rm -f $(KMP_BASE)/lib/ios-arm64/$(BIN_NAME).a $(KMP_BASE)/lib/ios-simulator-arm64/$(BIN_NAME).a $(KMP_BASE)/lib/ios-simulator-x64/$(BIN_NAME).a
	echo "void dummy() {}" > /tmp/dummy.c
	xcrun -sdk iphoneos clang -c /tmp/dummy.c -o /tmp/dummy_ios.o -arch arm64
	xcrun -sdk iphonesimulator clang -c /tmp/dummy.c -o /tmp/dummy_sim.o -arch arm64
	xcrun -sdk iphonesimulator clang -c /tmp/dummy.c -o /tmp/dummy_sim_x64.o -arch x86_64
	ar crs $(KMP_BASE)/lib/ios-arm64/$(BIN_NAME).a /tmp/dummy_ios.o
	ar crs $(KMP_BASE)/lib/ios-simulator-arm64/$(BIN_NAME).a /tmp/dummy_sim.o
	ar crs $(KMP_BASE)/lib/ios-simulator-x64/$(BIN_NAME).a /tmp/dummy_sim_x64.o
	rm -f /tmp/dummy.c /tmp/dummy_ios.o /tmp/dummy_sim.o /tmp/dummy_sim_x64.o
	cd langs/kotlin-multiplatform && ./gradlew :breez-sdk-spark-kmp:assemble

# Full package with all binaries
package-kotlin-multiplatform: build-kotlin-multiplatform
	mkdir -p $(KMP_BASE)/lib/ios-arm64 $(KMP_BASE)/lib/ios-simulator-arm64 $(KMP_BASE)/lib/ios-simulator-x64
	cp -r $(FFI_DIR)kmp/* $(KMP_BASE)/
	rm -f $(FFI_DIR)kotlin/jniLibs/**/*common.so $(FFI_DIR)kotlin/jniLibs/**/*spark.so
	cp -r $(FFI_DIR)kotlin/jniLibs/ $(KMP_BASE)/androidMain/jniLibs/
	cp $(TARGET_DIR)aarch64-apple-ios/release/$(BIN_NAME).a $(KMP_BASE)/lib/ios-arm64/
	cp $(TARGET_DIR)aarch64-apple-ios-sim/release/$(BIN_NAME).a $(KMP_BASE)/lib/ios-simulator-arm64/
	cp $(TARGET_DIR)x86_64-apple-ios/release/$(BIN_NAME).a $(KMP_BASE)/lib/ios-simulator-x64/
	cd langs/kotlin-multiplatform && ./gradlew :breez-sdk-spark-kmp:assemble

package-nuget: bindings-csharp build-aarch64-apple-darwin build-x86_64-apple-darwin build-release-target-aarch64-unknown-linux-gnu build-release-target-x86_64-unknown-linux-gnu build-release-target-i686-pc-windows-msvc build-release-target-x86_64-pc-windows-msvc
	mkdir -p langs/csharp/src/runtimes/osx-arm64/native
	mkdir -p langs/csharp/src/runtimes/osx-x64/native
	mkdir -p langs/csharp/src/runtimes/linux-x64/native
	mkdir -p langs/csharp/src/runtimes/linux-arm64/native
	mkdir -p langs/csharp/src/runtimes/win-x86/native
	mkdir -p langs/csharp/src/runtimes/win-x64/native
	cp $(TARGET_DIR)aarch64-apple-darwin/release/$(BIN_NAME).dylib langs/csharp/src/runtimes/osx-arm64/native/$(BIN_NAME).dylib
	cp $(TARGET_DIR)x86_64-apple-darwin/release/$(BIN_NAME).dylib langs/csharp/src/runtimes/osx-x64/native/$(BIN_NAME).dylib
	cp $(TARGET_DIR)x86_64-unknown-linux-gnu/release/$(BIN_NAME).so langs/csharp/src/runtimes/linux-x64/native/$(BIN_NAME).so
	cp $(TARGET_DIR)aarch64-unknown-linux-gnu/release/$(BIN_NAME).so langs/csharp/src/runtimes/linux-arm64/native/$(BIN_NAME).so
	cp $(TARGET_DIR)i686-pc-windows-msvc/release/$(BIN_NAME).dll langs/csharp/src/runtimes/win-x86/native/$(BIN_NAME).dll
	cp $(TARGET_DIR)x86_64-pc-windows-msvc/release/$(BIN_NAME).dll langs/csharp/src/runtimes/win-x64/native/$(BIN_NAME).dll
	cp -r $(FFI_DIR)csharp/*.cs langs/csharp/src/
	cd langs/csharp/src && dotnet pack -c Release
