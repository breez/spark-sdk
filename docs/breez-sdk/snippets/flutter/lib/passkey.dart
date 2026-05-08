import 'dart:typed_data';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

// ANCHOR: implement-prf-provider
// Implement custom callbacks if the built-in PasskeyProvider doesn't
// fit your needs. Three callbacks: deriveSeeds for derivation,
// createPasskey for registration, isSupported for availability.
// Single-salt derivation is the trivial 1-element bulk case.
Future<List<Uint8List>> deriveSeeds(List<String> salts) async {
  // Call platform passkey API with PRF extension. Use the dual-salt
  // ceremony when the authenticator supports it (one OS prompt for N
  // salts) and fall back to per-salt assertions otherwise. Returns
  // one 32-byte PRF output per salt in input order.
  throw UnimplementedError('Implement using platform passkey APIs');
}

Future<RegisteredCredential> createPasskey(CreatePasskeyRequest request) async {
  // Register a new credential and return its ID + AAGUID + BE flag.
  throw UnimplementedError('Implement registration via native passkey API');
}

Future<bool> isSupported() async {
  // Check if a PRF-capable authenticator is reachable from this
  // platform / device.
  throw UnimplementedError('Check platform passkey availability');
}
// ANCHOR_END: implement-prf-provider

Future<void> checkAvailability() async {
  // ANCHOR: check-availability
  final prfProvider = PasskeyProvider();
  if (await prfProvider.isSupported()) {
    // Show passkey as primary option
  } else {
    // Fall back to mnemonic flow
  }
  // ANCHOR_END: check-availability
}

Future<BreezSdk> connectWithPasskey() async {
  // ANCHOR: connect-with-passkey
  // Use the built-in platform PRF provider (or pass custom callbacks).
  final prfProvider = PasskeyProvider();
  final passkey = PasskeyClient(
    deriveSeeds: prfProvider.deriveSeeds,
    isSupported: prfProvider.isSupported,
    createPasskey: prfProvider.createPasskey,
  );

  // signIn derives the wallet seed for an existing credential. With
  // bulk PRF on iOS+Android this is a single OS prompt that derives
  // master + label seeds in one ceremony.
  final response = await passkey.signIn(
    request: SignInRequest(label: 'personal', extraSalts: []),
  );

  final config = defaultConfig(network: Network.mainnet);
  final sdk = await connect(
      request: ConnectRequest(
          config: config, seed: response.wallet.seed, storageDir: "./.data"));
  // ANCHOR_END: connect-with-passkey
  return sdk;
}

Future<BreezSdk> registerNewPasskey() async {
  // ANCHOR: register-passkey
  // For a brand-new user with no existing passkey: register() creates
  // the credential AND derives the wallet seed in one orchestrated
  // call. On iOS+Android this is 2 OS prompts total (1 create + 1
  // dual-salt assert) thanks to the SDK's bulk-PRF setup_wallet path.
  final prfProvider = PasskeyProvider();
  final passkey = PasskeyClient(
    deriveSeeds: prfProvider.deriveSeeds,
    isSupported: prfProvider.isSupported,
    createPasskey: prfProvider.createPasskey,
  );

  final response = await passkey.register(
    request: RegisterRequest(
      label: 'personal',
      extraSalts: [],
      excludeCredentialIds: [],
    ),
  );

  final config = defaultConfig(network: Network.mainnet);
  final sdk = await connect(
      request: ConnectRequest(
          config: config, seed: response.wallet.seed, storageDir: "./.data"));
  // ANCHOR_END: register-passkey
  return sdk;
}

Future<List<String>> listLabels() async {
  // ANCHOR: list-labels
  final prfProvider = PasskeyProvider();
  final relayConfig = NostrRelayConfig(
    breezApiKey: '<breez api key>',
  );
  final passkey = PasskeyClient(
    deriveSeeds: prfProvider.deriveSeeds,
    isSupported: prfProvider.isSupported,
    createPasskey: prfProvider.createPasskey,
    relayConfig: relayConfig,
  );

  // signIn with no label runs in discovery mode: it derives the master
  // seed AND lists labels in the same ceremony, so a follow-up
  // listLabels() reads from the cached identity for free.
  final labels = await passkey.listLabels();

  for (final label in labels) {
    print("Found label: $label");
  }
  // ANCHOR_END: list-labels
  return labels;
}

Future<void> storeLabel() async {
  // ANCHOR: store-label
  final prfProvider = PasskeyProvider();
  final relayConfig = NostrRelayConfig(
    breezApiKey: '<breez api key>',
  );
  final passkey = PasskeyClient(
    deriveSeeds: prfProvider.deriveSeeds,
    isSupported: prfProvider.isSupported,
    createPasskey: prfProvider.createPasskey,
    relayConfig: relayConfig,
  );

  // For a new label on an existing identity, call signIn(newLabel)
  // first to seed the SDK's identity cache via setup_wallet, THEN
  // storeLabel uses the cached identity for free (1 OS prompt total).
  await passkey.storeLabel(label: "personal");
  // ANCHOR_END: store-label
}
