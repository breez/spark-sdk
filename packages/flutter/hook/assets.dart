/// The native libraries the generated bindings load, keyed by the asset name
/// in their `@Native(assetId: 'package:breez_sdk_spark_flutter/<name>')`.
///
/// The library names come from `cdylib_name` in crates/breez-sdk/core/uniffi.toml
/// and the relay crate's package name in native/dispatch/Cargo.toml.
const bindingsAsset = 'uniffi:breez_sdk_spark_bindings';
const relayAsset = 'uniffi:breez_sdk_spark_bindings_dispatch';

const libraryNames = {
  bindingsAsset: 'breez_sdk_spark_bindings',
  relayAsset: 'breez_sdk_spark_bindings_dart_dispatch',
};
