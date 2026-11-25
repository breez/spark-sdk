#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint breez_sdk_spark_flutter.podspec` to validate before publishing.
#
# The pod only carries the passkey plugin. The SDK's native library is bundled
# by the package's build hook (hook/build.dart) as a framework.
#
Pod::Spec.new do |spec|
  spec.name                = "breez_sdk_spark_flutter"
  spec.version             = "0.1.0"
  spec.license             = { :file => '../LICENSE' }
  spec.summary             = "Passkey plugin for the Breez Spark SDK Flutter package"
  spec.homepage            = "https://breez.technology"
  spec.authors             = { "Breez" => "contact@breez.technology" }
  spec.documentation_url   = "https://docs.breez.technology"

  spec.source              = { :path => '.' }
  spec.source_files        = 'Classes/**/*'
  spec.public_header_files = 'Classes/**/*.h'
  spec.dependency 'Flutter'

  spec.platform            = :ios, '15.0'
  spec.swift_version       = '5.0'
  spec.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    # Flutter.framework does not contain a i386 slice.
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386',
  }
end
