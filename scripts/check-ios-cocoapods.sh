#!/usr/bin/env bash
#
# Typecheck the Flutter and React Native iOS sources the way CocoaPods
# builds them: one pod module per plugin, with PasskeyPRFHelper.h reachable only
# through the pod's umbrella header. `swift build` on the SPM package does not
# cover this, because there PasskeyPRFHelperObjC is a real module.
#
# Requires macOS with Xcode. Skips cleanly elsewhere so `make check` still runs.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Matches `spec.platform = :ios, '13.0'` in the Flutter podspec. Applied to the
# RN pod too: a lower floor only makes the availability checks stricter.
DEPLOYMENT_TARGET="arm64-apple-ios13.0"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "check-ios-cocoapods: skipping, requires macOS (got $(uname -s))"
  exit 0
fi

if ! command -v xcrun >/dev/null 2>&1; then
  echo "check-ios-cocoapods: skipping, Xcode command line tools not found"
  exit 0
fi

IOS_SDK="$(xcrun --sdk iphoneos --show-sdk-path)"

# Flutter.framework ships inside the Flutter SDK. subosito/flutter-action sets
# FLUTTER_ROOT; locally, derive it from whichever flutter is on PATH.
if [[ -z "${FLUTTER_ROOT:-}" ]]; then
  if command -v flutter >/dev/null 2>&1; then
    FLUTTER_ROOT="$(cd "$(dirname "$(readlink -f "$(command -v flutter)" 2>/dev/null || command -v flutter)")/.." && pwd)"
  else
    echo "check-ios-cocoapods: FAIL - flutter not found and FLUTTER_ROOT unset" >&2
    exit 1
  fi
fi

FLUTTER_FW="${FLUTTER_ROOT}/bin/cache/artifacts/engine/ios/Flutter.xcframework/ios-arm64"
if [[ ! -d "$FLUTTER_FW" ]]; then
  echo "check-ios-cocoapods: FAIL - Flutter.xcframework missing at ${FLUTTER_FW}" >&2
  echo "  run: flutter precache --ios" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fail=0

# Stage a plugin's iOS sources into a scratch pod and typecheck them as one module.
#   $1 pod module name, $2 staging subdir, $3 source dir relative to REPO_ROOT,
#   $4 ObjC header relative to REPO_ROOT
#
# Every .swift in the source dir is compiled, so Swift added to either plugin is
# covered without editing this script. The header goes into the umbrella rather
# than the compile list, reproducing the header-visibility path CocoaPods builds.
check_pod() {
  local module="$1" subdir="$2" src_dir="$3" header="$4"
  local dir="${WORK}/${subdir}"
  mkdir -p "$dir"

  if [[ ! -f "${REPO_ROOT}/${header}" ]]; then
    echo "check-ios-cocoapods: FAIL - missing header ${header}" >&2
    fail=1
    return
  fi
  cp "${REPO_ROOT}/${header}" "$dir/"

  local swift_files=()
  local f
  for f in "${REPO_ROOT}/${src_dir}"/*.swift; do
    [[ -f "$f" ]] || continue
    cp "$f" "$dir/"
    swift_files+=("$(basename "$f")")
  done
  if [[ ${#swift_files[@]} -eq 0 ]]; then
    echo "check-ios-cocoapods: FAIL - no .swift found in ${src_dir}" >&2
    fail=1
    return
  fi

  printf '#import "PasskeyPRFHelper.h"\n' >"${dir}/umbrella.h"
  cat >"${dir}/module.modulemap" <<EOF
module ${module} {
  umbrella header "umbrella.h"
  export *
  module * { export * }
}
EOF

  echo "check-ios-cocoapods: ${module} (${#swift_files[@]} swift sources)"
  if ! (cd "$dir" && swiftc \
    -target "$DEPLOYMENT_TARGET" \
    -sdk "$IOS_SDK" \
    -F "$FLUTTER_FW" \
    -I . \
    -I "${WORK}/ReactStub" \
    -module-name "$module" \
    -import-underlying-module \
    -suppress-warnings \
    -typecheck "${swift_files[@]}"); then
    echo "check-ios-cocoapods: FAIL - ${module} did not typecheck" >&2
    fail=1
  fi
}

# The RN module's entire React surface is these two typealiases, so a stub
# stands in for React-Core and no `pod install` is needed. If the module ever
# reaches for another RCT symbol, the typecheck fails rather than silently
# skipping, which is the intended signal to widen this stub.
mkdir -p "${WORK}/ReactStub"
cat >"${WORK}/ReactStub/React.h" <<'EOF'
#import <Foundation/Foundation.h>
typedef void (^RCTPromiseResolveBlock)(id _Nullable result);
typedef void (^RCTPromiseRejectBlock)(NSString *_Nullable code, NSString *_Nullable message, NSError *_Nullable error);
EOF
printf 'module React { header "React.h" export * }\n' >"${WORK}/ReactStub/module.modulemap"

check_pod breez_sdk_spark_flutter flutter \
  packages/flutter/ios/Classes \
  packages/flutter/ios/Classes/PasskeyPRFHelper.h

check_pod BreezSdkSparkReactNative react-native \
  packages/react-native/ios \
  packages/react-native/ios/PasskeyPRFHelper.h

if [[ "$fail" -ne 0 ]]; then
  echo "check-ios-cocoapods: FAILED" >&2
  exit 1
fi

echo "check-ios-cocoapods: OK"
