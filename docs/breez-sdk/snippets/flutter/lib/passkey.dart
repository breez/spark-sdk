import 'dart:typed_data';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

// ANCHOR: implement-prf-provider
// Implement custom callbacks if the built-in PasskeyPrfProvider doesn't fit your needs.
Future<Uint8List> derivePrfSeed(String salt) async {
  // Call platform passkey API with PRF extension
  // Returns 32-byte PRF output
  throw UnimplementedError('Implement using platform passkey APIs');
}

Future<bool> isPrfAvailable() async {
  // Check if PRF-capable passkey exists
  throw UnimplementedError('Check platform passkey availability');
}
// ANCHOR_END: implement-prf-provider

Future<void> checkAvailability() async {
  // ANCHOR: check-availability
  final prfProvider = PasskeyPrfProvider();
  if (await prfProvider.isPrfAvailable()) {
    // Show passkey as primary option
  } else {
    // Fall back to mnemonic flow
  }
  // ANCHOR_END: check-availability
}

Future<BreezSdk> connectWithPasskey() async {
  // ANCHOR: connect-with-passkey
  // Use the built-in platform PRF provider (or pass custom callbacks)
  final prfProvider = PasskeyPrfProvider();
  final passkey = Passkey(
    derivePrfSeed: prfProvider.derivePrfSeed,
    isPrfAvailable: prfProvider.isPrfAvailable,
  );

  // Derive the wallet from the passkey (pass null for the default wallet)
  final wallet = await passkey.getWallet(label: "personal");

  final config = defaultConfig(network: Network.mainnet);
  final sdk = await connect(
      request: ConnectRequest(
          config: config, seed: wallet.seed, storageDir: "./.data"));
  // ANCHOR_END: connect-with-passkey
  return sdk;
}

Future<List<String>> listLabels() async {
  // ANCHOR: list-labels
  final prfProvider = PasskeyPrfProvider();
  final relayConfig = NostrRelayConfig(
    breezApiKey: '<breez api key>',
  );
  final passkey = Passkey(
    derivePrfSeed: prfProvider.derivePrfSeed,
    isPrfAvailable: prfProvider.isPrfAvailable,
    relayConfig: relayConfig,
  );

  // Query Nostr for labels associated with this passkey
  final labels = await passkey.listLabels();

  for (final label in labels) {
    print("Found label: $label");
  }
  // ANCHOR_END: list-labels
  return labels;
}

Future<void> storeLabel() async {
  // ANCHOR: store-label
  final prfProvider = PasskeyPrfProvider();
  final relayConfig = NostrRelayConfig(
    breezApiKey: '<breez api key>',
  );
  final passkey = Passkey(
    derivePrfSeed: prfProvider.derivePrfSeed,
    isPrfAvailable: prfProvider.isPrfAvailable,
    relayConfig: relayConfig,
  );

  // Publish the label to Nostr for later discovery
  await passkey.storeLabel(label: "personal");
  // ANCHOR_END: store-label
}
