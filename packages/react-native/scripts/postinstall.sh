#!/bin/sh
set -e

# Download prebuilt binary artifacts from the release
# Skip if running in Expo managed workflow (prebuild will handle it)

# Skip if artifacts already exist (they may have been downloaded by the Expo plugin)
if [ -d "android/src/main/jniLibs" ] && [ -d "build/RnBreezSdkSpark.xcframework" ]; then
  exit 0
fi

# Check for explicit skip flag
if [ -n "$EXPO_PUBLIC_SKIP_POSTINSTALL" ]; then
  exit 0
fi

REPO=https://github.com/breez/breez-sdk-spark-react-native
TAG=$(node -p "require('./package.json').version")
ANDROID_CHECKSUM=$(node -p "require('./package.json').checksums.android")
IOS_CHECKSUM=$(node -p "require('./package.json').checksums.ios")

# Download and verify Android
ANDROID_URL=$REPO/releases/download/$TAG/android-artifacts.zip
curl -L $ANDROID_URL --output android-artifacts.zip

ACTUAL_ANDROID=$(shasum -a 256 android-artifacts.zip | cut -d' ' -f1)
if [ "$ACTUAL_ANDROID" != "$ANDROID_CHECKSUM" ]; then
  echo "Error: Android artifacts checksum mismatch"
  echo "Expected: $ANDROID_CHECKSUM"
  echo "Got: $ACTUAL_ANDROID"
  rm -f android-artifacts.zip
  exit 1
fi

unzip -o android-artifacts.zip
rm -rf android-artifacts.zip

# Download and verify iOS
IOS_URL=$REPO/releases/download/$TAG/ios-artifacts.zip
curl -L $IOS_URL --output ios-artifacts.zip

ACTUAL_IOS=$(shasum -a 256 ios-artifacts.zip | cut -d' ' -f1)
if [ "$ACTUAL_IOS" != "$IOS_CHECKSUM" ]; then
  echo "Error: iOS artifacts checksum mismatch"
  echo "Expected: $IOS_CHECKSUM"
  echo "Got: $ACTUAL_IOS"
  rm -f ios-artifacts.zip
  exit 1
fi

unzip -o ios-artifacts.zip
rm -rf ios-artifacts.zip