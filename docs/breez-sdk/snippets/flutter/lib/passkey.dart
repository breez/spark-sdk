import 'dart:typed_data';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

// ANCHOR: implement-prf-provider
// Implement these functions using platform passkey APIs.
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

Future<BreezSdk> connectWithPasskey() async {
  // ANCHOR: connect-with-passkey
  final passkey = Passkey(
    derivePrfSeed: derivePrfSeed,
    isPrfAvailable: isPrfAvailable,
  );

  // Derive the wallet from the passkey (pass null for the default wallet)
  final wallet = await passkey.getWallet(walletName: "personal");

  final config = defaultConfig(network: Network.mainnet);
  final sdk = await connect(
      request: ConnectRequest(
          config: config, seed: wallet.seed, storageDir: "./.data"));
  // ANCHOR_END: connect-with-passkey
  return sdk;
}

Future<List<String>> listWalletNames() async {
  // ANCHOR: list-wallet-names
  final relayConfig = NostrRelayConfig(
    breezApiKey: '<breez api key>',
  );
  final passkey = Passkey(
    derivePrfSeed: derivePrfSeed,
    isPrfAvailable: isPrfAvailable,
    relayConfig: relayConfig,
  );

  // Query Nostr for wallet names associated with this passkey
  final walletNames = await passkey.listWalletNames();

  for (final walletName in walletNames) {
    print("Found wallet: $walletName");
  }
  // ANCHOR_END: list-wallet-names
  return walletNames;
}

Future<void> storeWalletName() async {
  // ANCHOR: store-wallet-name
  final relayConfig = NostrRelayConfig(
    breezApiKey: '<breez api key>',
  );
  final passkey = Passkey(
    derivePrfSeed: derivePrfSeed,
    isPrfAvailable: isPrfAvailable,
    relayConfig: relayConfig,
  );

  // Publish the wallet name to Nostr for later discovery
  await passkey.storeWalletName(walletName: "personal");
  // ANCHOR_END: store-wallet-name
}
