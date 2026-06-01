#!/usr/bin/env bash
#
# Guard against the Xcode 26 passkey-PRF regression. The AuthenticationServices
# PRF types are NS_REFINED_FOR_SWIFT, so their initializers and accessors are
# not reachable from Swift on Xcode 26. The passkey layers must build and read
# PRF through the PasskeyPRFHelper ObjC bridge, never directly in Swift. ObjC
# (.h/.m) is allowed to use them: that bridge is the whole point.
#
# This is a fast static guard, not a full compile. A complete check would build
# the React Native pod's Swift with `pod install` + `xcodebuild` on an Xcode 26
# runner (see the PR description); that needs an example app the repo does not
# ship yet, so this guard covers the specific known regression in the meantime.
set -euo pipefail

dirs=(
  packages/react-native/ios
  packages/flutter/ios
  crates/breez-sdk/bindings/langs/swift
)

# The refined-for-Swift PRF input/value types, plus a direct `.prf` property
# read. `\.prf([^A-Za-z]|$)` matches a real `.prf` access but skips the
# `.prfNotSupported` / `.prfEvaluationFailed` error enum cases.
pattern='ASAuthorizationPublicKeyCredentialPRF(AssertionInput|RegistrationInput|Values)|\.prf([^A-Za-z]|$)'

hits=""
for d in "${dirs[@]}"; do
  [ -d "$d" ] || continue
  while IFS= read -r f; do
    m="$(grep -nE "$pattern" "$f" || true)"
    [ -n "$m" ] && hits+="$f"$'\n'"$m"$'\n'
  done < <(find "$d" -name '*.swift')
done

if [ -n "$hits" ]; then
  echo "ERROR: iOS Swift uses the NS_REFINED_FOR_SWIFT AuthenticationServices PRF API directly." >&2
  echo "This does not compile on Xcode 26. Route PRF through the PasskeyPRFHelper ObjC bridge." >&2
  echo >&2
  printf '%s' "$hits" >&2
  exit 1
fi

echo "check-passkey-prf-bridge: OK (no direct refined-for-swift PRF use in iOS Swift)."
