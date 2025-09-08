#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint breez_sdk_spark_flutter.podspec` to validate before publishing.
#
Pod::Spec.new do |spec|
  spec.name                = "breez_sdk_spark_flutter"
  spec.version             = "0.1.3"
  spec.license             = { :file => '../LICENSE' }
  spec.summary             = "Swift bindings to the Breez Spark SDK"
  spec.homepage            = "https://breez.technology"
  spec.authors             = { "Breez" => "contact@breez.technology" }
  spec.documentation_url   = "https://sdk-doc-spark.breez.technology"

  # This will ensure the source files in Classes/ are included in the native
  # builds of apps using this FFI plugin. Podspec does not support relative
  # paths, so Classes contains a forwarder C file that relatively imports
  # `../src/*` so that the C sources can be shared among all target platforms.
  spec.source              = { :path => '.' }
  spec.source_files        = 'Classes/**/*'
  spec.dependency 'Flutter'

  spec.platform            = :ios, '11.0'
  spec.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES' }
  spec.swift_version       = '5.0'

  spec.script_phase = {
    :name => 'Build Rust library',
    # First argument is relative path to the `rust` folder, second is name of rust library
    :script => 'sh "$PODS_TARGET_SRCROOT/../cargokit/build_pod.sh" ../rust breez_sdk_spark_flutter',
    :execution_position => :before_compile,
    :input_files => ['${BUILT_PRODUCTS_DIR}/cargokit_phony'],
    # Let XCode know that the static library referenced in -force_load below is
    # created by this build step.
    :output_files => ["${BUILT_PRODUCTS_DIR}/libbreez_sdk_spark_flutter.a"],
  }
  spec.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    # Flutter.framework does not contain a i386 slice.
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386',
    'OTHER_LDFLAGS' => '-force_load ${BUILT_PRODUCTS_DIR}/libbreez_sdk_spark_flutter.a',
  }
end