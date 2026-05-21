import 'dart:typed_data';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

// ANCHOR: implement-prf-provider
// Implement custom callbacks if the built-in PasskeyProvider doesn't
// fit your needs. Six callbacks: deriveSeeds for derivation,
// createPasskey for registration, isSupported for availability, and
// get/remove/clearKnownCredentialIds for the credentials() sub-object.
Future<List<Uint8List>> deriveSeeds(DeriveSeedsRequest request) async {
  // Call platform passkey API with PRF extension. Use the dual-salt
  // ceremony when the authenticator supports it (one OS prompt for N
  // salts) and fall back to per-salt assertions otherwise. Returns
  // one 32-byte PRF output per salt in input order.
  throw UnimplementedError('Implement using platform passkey APIs');
}

Future<RegisteredCredential> createPasskey(List<Uint8List> excludeCredentialIds) async {
  // Register a new credential and return its ID, the WebAuthn user.id
  // the native plugin minted for it (returned for host-side
  // correlation, never host-supplied), AAGUID, and BE flag.
  throw UnimplementedError('Implement registration via native passkey API');
}

Future<bool> isSupported() async {
  // Check if a PRF-capable authenticator is reachable from this
  // platform / device.
  throw UnimplementedError('Check platform passkey availability');
}

Future<List<Uint8List>> getKnownCredentialIds() async => const [];
Future<void> removeKnownCredentialId(Uint8List _) async {}
Future<void> clearKnownCredentialIds() async {}
// ANCHOR_END: implement-prf-provider

Future<void> checkAvailability() async {
  // ANCHOR: check-availability
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: '<breez api key>');
  final passkey = createPasskeyClient(
    rpId: 'my-app.com',
    rpName: 'My App',
    sdkConfig: config,
  );

  final availability = await passkey.checkAvailability();
  if (availability is PasskeyAvailability_Available) {
    // Show passkey as primary option.
  } else if (availability is PasskeyAvailability_PrfUnsupported) {
    // Fall back to mnemonic flow.
  } else if (availability is PasskeyAvailability_NotAssociated) {
    print("Domain association failed (source=${availability.source}): ${availability.reason}");
  } else if (availability is PasskeyAvailability_Skipped) {
    // No verification source on this platform; proceed normally.
  }
  // ANCHOR_END: check-availability
}

Future<BreezSdk> connectWithPasskey() async {
  // ANCHOR: connect-with-passkey
  // Single-CTA onboarding: silent sign-in, fall through to register.
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: '<breez api key>');
  final passkey = createPasskeyClient(
    rpId: 'my-app.com',
    rpName: 'My App',
    sdkConfig: config,
  );

  final response = await passkey.connectWithPasskey(
    request: ConnectWithPasskeyRequest(label: 'personal', excludeCredentialIds: const []),
  );

  // `registeredCredential` is the path discriminator (null on sign-in).
  final credential = response.registeredCredential;
  if (credential != null) {
    final _ = credential.credentialId;
  }

  final sdk = await connect(
      request: ConnectRequest(
          config: config, seed: response.wallet.seed, storageDir: "./.data"));
  // ANCHOR_END: connect-with-passkey
  return sdk;
}

Future<BreezSdk> registerNewPasskey() async {
  // ANCHOR: register-passkey
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: '<breez api key>');
  final passkey = createPasskeyClient(
    rpId: 'my-app.com',
    rpName: 'My App',
    sdkConfig: config,
  );

  final response = await passkey.register(
    request: RegisterRequest(label: 'personal'),
  );

  // Persist credentialId for future excludeCredentialIds.
  final _persistedCredentialId = response.credential.credentialId;
  final _persistedUserId = response.credential.userId;

  final sdk = await connect(
      request: ConnectRequest(
          config: config, seed: response.wallet.seed, storageDir: "./.data"));
  // ANCHOR_END: register-passkey
  return sdk;
}

Future<List<String>> listLabels() async {
  // ANCHOR: list-labels
  final sdkConfig = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: '<breez api key>');
  final passkey = createPasskeyClient(
    rpId: 'my-app.com',
    rpName: 'My App',
    sdkConfig: sdkConfig,
    // Default wallet label when register / signIn receive no label.
    passkeyConfig: PasskeyConfig(defaultLabel: 'personal'),
  );

  final labels = await passkey.labels().list();

  for (final label in labels) {
    print("Found label: $label");
  }
  // ANCHOR_END: list-labels
  return labels;
}

Future<void> storeLabel() async {
  // ANCHOR: store-label
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: '<breez api key>');
  final passkey = createPasskeyClient(
    rpId: 'my-app.com',
    rpName: 'My App',
    sdkConfig: config,
  );

  // For a new label on an existing identity, sign in with that label first.
  await passkey.labels().store(label: "personal");
  // ANCHOR_END: store-label
}

Future<void> checkDomain() async {
  // ANCHOR: domain-association
  // Lower-level provider call. Most hosts use `checkAvailability` instead.
  final prfProvider = PasskeyProvider(PasskeyProviderOptions(rpId: 'my-app.com', rpName: 'My App'));
  final result = await prfProvider.checkDomainAssociation();

  if (result is DomainAssociationAssociated) {
    // Safe to proceed.
  } else if (result is DomainAssociationNotAssociated) {
    print("Domain association failed (source=${result.source}): ${result.reason}");
    return;
  } else if (result is DomainAssociationSkipped) {
    // Verification could not be performed; proceed normally.
  }
  // ANCHOR_END: domain-association
}

Future<Wallet?> recoverFromAlreadyExists() async {
  // ANCHOR: recover-already-exists
  // Recovery: flip to sign-in so the OS picker surfaces the existing credential.
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: '<breez api key>');
  final passkey = createPasskeyClient(
    rpId: 'my-app.com',
    rpName: 'My App',
    sdkConfig: config,
  );

  try {
    final response = await passkey.register(
      request: RegisterRequest(
        label: 'personal',
        excludeCredentialIds: [
          // app-persisted credential IDs from prior registrations
        ],
      ),
    );
    return response.wallet;
  } on PasskeyPrfException catch (e) {
    if (e.code != 'credentialAlreadyExists') rethrow;
    final response = await passkey.signIn(
      request: SignInRequest(label: 'personal'),
    );
    return response.wallet;
  }
  // ANCHOR_END: recover-already-exists
}

Future<SignInResponse> handleTimeout() async {
  // ANCHOR: handle-timeout
  // Timeout is distinct from a cancel: surface a re-prompt UI.
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: '<breez api key>');
  final passkey = createPasskeyClient(
    rpId: 'my-app.com',
    rpName: 'My App',
    sdkConfig: config,
  );

  try {
    return await passkey.signIn(
      request: SignInRequest(label: 'personal'),
    );
  } on PasskeyPrfException catch (e) {
    if (e.code == 'userTimedOut') {
      print("Sign-in timed out: show \"Try Again\" UI.");
    }
    rethrow;
  }
  // ANCHOR_END: handle-timeout
}
