import 'dart:typed_data';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

// ANCHOR: implement-prf-provider
// Flutter uses callbacks instead of a trait object.
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

Future<BreezSdk> createSeed() async {
  // ANCHOR: create-seed
  final seedless = SeedlessRestore(
    derivePrfSeed: derivePrfSeed,
    isPrfAvailable: isPrfAvailable,
  );

  // Create a new seed with user-chosen salt
  // The salt is published to Nostr for later discovery
  final seed = await seedless.createSeed(salt: "personal");

  // Use the seed to initialize the SDK
  final config = defaultConfig(network: Network.mainnet);
  final builder = SdkBuilder(config: config, seed: seed);
  builder.withDefaultStorage(storageDir: "./.data");
  final sdk = await builder.build();
  // ANCHOR_END: create-seed
  return sdk;
}

Future<List<String>> listSalts() async {
  // ANCHOR: list-salts
  final seedless = SeedlessRestore(
    derivePrfSeed: derivePrfSeed,
    isPrfAvailable: isPrfAvailable,
  );

  // Query Nostr for salts associated with this passkey
  final salts = await seedless.listSalts();

  for (final salt in salts) {
    print("Found wallet: $salt");
  }
  // ANCHOR_END: list-salts
  return salts;
}

Future<BreezSdk> restoreSeed() async {
  // ANCHOR: restore-seed
  final seedless = SeedlessRestore(
    derivePrfSeed: derivePrfSeed,
    isPrfAvailable: isPrfAvailable,
  );

  // Restore seed using a known salt
  final seed = await seedless.restoreSeed(salt: "personal");

  // Use the seed to initialize the SDK
  final config = defaultConfig(network: Network.mainnet);
  final builder = SdkBuilder(config: config, seed: seed);
  builder.withDefaultStorage(storageDir: "./.data");
  final sdk = await builder.build();
  // ANCHOR_END: restore-seed
  return sdk;
}
