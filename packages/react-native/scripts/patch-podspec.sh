#!/bin/sh
set -e

# Patch the generated podspec to auto-detect static vs dynamic xcframework linkage.
# This runs after ubrn:build since UBRN regenerates the podspec with a hardcoded path.

PODSPEC="breeztech-breez-sdk-spark-react-native.podspec"

if [ ! -f "$PODSPEC" ]; then
  echo "Error: $PODSPEC not found"
  exit 1
fi

# Replace the vendored_frameworks line with auto-detection logic
ruby -i -e '
lines = ARGF.readlines
output = []
lines.each do |line|
  if line =~ /^\s*s\.vendored_frameworks\s*=/
    indent = line[/^\s*/]
    output << "#{indent}# Select static or dynamic xcframework based on the consumer'\''s linkage setting.\n"
    output << "#{indent}# 1. Check USE_FRAMEWORKS env var (standard RN/CocoaPods convention)\n"
    output << "#{indent}# 2. Try to detect from the Podfile'\''s target definitions\n"
    output << "#{indent}# 3. Default to static (works without use_frameworks!)\n"
    output << "#{indent}use_dynamic = ENV['\''USE_FRAMEWORKS'\''] == '\''dynamic'\''\n"
    output << "#{indent}unless use_dynamic\n"
    output << "#{indent}  begin\n"
    output << "#{indent}    podfile = Pod::Config.instance.podfile\n"
    output << "#{indent}    if podfile\n"
    output << "#{indent}      podfile.target_definition_list.each do |td|\n"
    output << "#{indent}        if td.build_type == Pod::BuildType.dynamic_framework\n"
    output << "#{indent}          use_dynamic = true\n"
    output << "#{indent}          break\n"
    output << "#{indent}        end\n"
    output << "#{indent}      end\n"
    output << "#{indent}    end\n"
    output << "#{indent}  rescue\n"
    output << "#{indent}    # Detection failed, fall back to static\n"
    output << "#{indent}  end\n"
    output << "#{indent}end\n"
    output << "\n"
    output << "#{indent}if use_dynamic\n"
    output << "#{indent}  s.vendored_frameworks = \"build/dynamic/RnBreezSdkSpark.xcframework\"\n"
    output << "#{indent}  # CocoaPods does not auto-link vendored xcframeworks for dynamic framework\n"
    output << "#{indent}  # pods. Explicitly add the framework to the linker flags.\n"
    output << "#{indent}  s.pod_target_xcconfig = {\n"
    output << "#{indent}      \"OTHER_LDFLAGS\" => \"-framework \\\"RnBreezSdkSpark\\\"\"\n"
    output << "#{indent}  }\n"
    output << "#{indent}else\n"
    output << "#{indent}  s.vendored_frameworks = \"build/static/RnBreezSdkSpark.xcframework\"\n"
    output << "#{indent}end\n"
  else
    output << line
  end
end
puts output.join
' "$PODSPEC"

echo "Patched $PODSPEC with xcframework linkage auto-detection"
