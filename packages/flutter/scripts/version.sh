#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/../../.."

log() {
  echo -e "\033[1;34m[INFO]\033[0m $*"
}

# Get version from workspace Cargo.toml
TAG_NAME=$(awk -F' = ' '/^version =/{gsub(/"/,"",$2); print $2}' "$ROOT/Cargo.toml")

log "Detected version: $TAG_NAME"

# Update Flutter plugin pubspec.yaml version
log "Updating pubspec.yaml version..."
sed -i.bak -E "s/^(version: ).*/\1$TAG_NAME/" \
  "$ROOT/packages/flutter/pubspec.yaml"
rm "$ROOT/packages/flutter/pubspec.yaml.bak"

# iOS podspec
log "Updating iOS podspec..."
sed -i.bak -E "s/^([ ]*spec\.version[ ]*=[ ]*).*/\1\"$TAG_NAME\"/" \
  "$ROOT/packages/flutter/ios/breez_sdk_spark_flutter.podspec"
rm "$ROOT/packages/flutter/ios/"*.bak

# Android Gradle
GRADLE_HEADER="version '$TAG_NAME' // generated; do not edit"
log "Updating Android Gradle build.gradle..."
sed -i.bak -E "s/^version .*/version '$TAG_NAME' \/\/ generated; do not edit/" \
  "$ROOT/packages/flutter/android/build.gradle"
rm "$ROOT/packages/flutter/android/"*.bak

# Stage changes for commit
log "Staging updated files for git..."
git add \
  "$ROOT/packages/flutter/pubspec.yaml" \
  "$ROOT/packages/flutter/ios/breez_sdk_spark_flutter.podspec" \
  "$ROOT/packages/flutter/android/build.gradle"

log "✅ Version bump to $TAG_NAME completed successfully."
