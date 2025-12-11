#!/bin/sh
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

ANDROID_URL=$REPO/releases/download/$TAG/android-artifacts.zip
curl -L $ANDROID_URL --output android-artifacts.zip
unzip -o android-artifacts.zip
rm -rf android-artifacts.zip

IOS_URL=$REPO/releases/download/$TAG/ios-artifacts.zip
curl -L $IOS_URL --output ios-artifacts.zip
unzip -o ios-artifacts.zip
rm -rf ios-artifacts.zip