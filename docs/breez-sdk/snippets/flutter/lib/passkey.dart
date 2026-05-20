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
  // `rpId` is required. Pass your app's domain, or
  // `PasskeyProvider.breezRpId` if your app is Breez-registered.
  final prfProvider = PasskeyProvider(PasskeyProviderOptions(rpId: 'my-app.com'));
  final passkey = PasskeyClient(
    deriveSeeds: prfProvider.deriveSeeds,
    isSupported: prfProvider.isSupported,
    createPasskey: prfProvider.createPasskey,
    getKnownCredentialIds: prfProvider.getKnownCredentialIds,
    removeKnownCredentialId: prfProvider.removeKnownCredentialId,
    clearKnownCredentialIds: prfProvider.clearKnownCredentialIds,
  );

  // checkAvailability collapses isSupported + checkDomainAssociation
  // into a single tagged value. Branch on the variant the host needs.
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
  // Single-CTA onboarding: silent sign-in for a returning user,
  // fall-through to register on a fresh device. Internally pins
  // `preferImmediatelyAvailableCredentials = true` so the silent
  // attempt fast-fails (no UI) when no local credential exists; only
  // `CredentialNotFound` flips to register, all other errors (cancel
  // / timeout / configuration) propagate unchanged.
  final prfProvider = PasskeyProvider(PasskeyProviderOptions(rpId: 'my-app.com'));
  final passkey = PasskeyClient(
    deriveSeeds: prfProvider.deriveSeeds,
    isSupported: prfProvider.isSupported,
    createPasskey: prfProvider.createPasskey,
    getKnownCredentialIds: prfProvider.getKnownCredentialIds,
    removeKnownCredentialId: prfProvider.removeKnownCredentialId,
    clearKnownCredentialIds: prfProvider.clearKnownCredentialIds,
  );

  final response = await passkey.connectWithPasskey(
    request: ConnectWithPasskeyRequest(label: 'personal', excludeCredentialIds: const []),
  );

  // Branch on `flow` to know which path ran.
  switch (response.flow) {
    case ConnectFlow_SignedIn(): // returning user
    case ConnectFlow_Registered(): // new user; credential available on the variant
  }

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
  // dual-salt assert) thanks to the SDK's bulk-PRF path.
  final prfProvider = PasskeyProvider(PasskeyProviderOptions(rpId: 'my-app.com'));
  final passkey = PasskeyClient(
    deriveSeeds: prfProvider.deriveSeeds,
    isSupported: prfProvider.isSupported,
    createPasskey: prfProvider.createPasskey,
    getKnownCredentialIds: prfProvider.getKnownCredentialIds,
    removeKnownCredentialId: prfProvider.removeKnownCredentialId,
    clearKnownCredentialIds: prfProvider.clearKnownCredentialIds,
  );

  final response = await passkey.register(
    request: RegisterRequest(label: 'personal'),
  );

  // Hosts SHOULD persist credential.credentialId (for excludeCredentialIds
  // bookkeeping) and credential.userId (for server-side correlation).
  // The SDK generates userId; it is never host-supplied.
  final _persistedCredentialId = response.credential.credentialId;
  final _persistedUserId = response.credential.userId;

  final config = defaultConfig(network: Network.mainnet);
  final sdk = await connect(
      request: ConnectRequest(
          config: config, seed: response.wallet.seed, storageDir: "./.data"));
  // ANCHOR_END: register-passkey
  return sdk;
}

Future<List<String>> listLabels() async {
  // ANCHOR: list-labels
  final prfProvider = PasskeyProvider(PasskeyProviderOptions(rpId: 'my-app.com'));
  final config = PasskeyConfig(
    // Optional: override the default wallet label used when register /
    // signIn receive `label = null`. Falls back to the SDK's internal
    // "Default" when unset.
    defaultLabel: 'personal',
  );
  // breezApiKey enables authenticated (NIP-42) Breez relay access for
  // label sync; pass null for public-relay-only.
  final passkey = PasskeyClient(
    deriveSeeds: prfProvider.deriveSeeds,
    isSupported: prfProvider.isSupported,
    createPasskey: prfProvider.createPasskey,
    getKnownCredentialIds: prfProvider.getKnownCredentialIds,
    removeKnownCredentialId: prfProvider.removeKnownCredentialId,
    clearKnownCredentialIds: prfProvider.clearKnownCredentialIds,
    breezApiKey: '<breez api key>',
    config: config,
  );

  // signIn with no label runs in discovery mode: it derives the master
  // seed AND lists labels in the same ceremony, so a follow-up
  // labels().list() reads from the cached identity for free.
  final labels = await passkey.labels().list();

  for (final label in labels) {
    print("Found label: $label");
  }
  // ANCHOR_END: list-labels
  return labels;
}

Future<void> storeLabel() async {
  // ANCHOR: store-label
  final prfProvider = PasskeyProvider(PasskeyProviderOptions(rpId: 'my-app.com'));
  final passkey = PasskeyClient(
    deriveSeeds: prfProvider.deriveSeeds,
    isSupported: prfProvider.isSupported,
    createPasskey: prfProvider.createPasskey,
    getKnownCredentialIds: prfProvider.getKnownCredentialIds,
    removeKnownCredentialId: prfProvider.removeKnownCredentialId,
    clearKnownCredentialIds: prfProvider.clearKnownCredentialIds,
    breezApiKey: '<breez api key>',
  );

  // For a new label on an existing identity, call signIn(newLabel)
  // first to warm the SDK's identity cache, THEN
  // labels().store() uses the cached identity for free (1 OS prompt total).
  await passkey.labels().store(label: "personal");
  // ANCHOR_END: store-label
}

Future<void> checkDomain() async {
  // ANCHOR: domain-association
  // Verify Apple AASA / Android Asset Links before the first WebAuthn
  // ceremony. Diagnostic only: never blocks.
  final prfProvider = PasskeyProvider(PasskeyProviderOptions(rpId: 'my-app.com'));
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
  // The OS rejected register because the user's password manager
  // already holds a credential matching `excludeCredentialIds`.
  // Route the user to the sign-in path: the OS picker will surface
  // the existing credential and the SDK's identity cache will warm
  // up on the assertion.
  final prfProvider = PasskeyProvider(PasskeyProviderOptions(rpId: 'my-app.com'));
  final passkey = PasskeyClient(
    deriveSeeds: prfProvider.deriveSeeds,
    isSupported: prfProvider.isSupported,
    createPasskey: prfProvider.createPasskey,
    getKnownCredentialIds: prfProvider.getKnownCredentialIds,
    removeKnownCredentialId: prfProvider.removeKnownCredentialId,
    clearKnownCredentialIds: prfProvider.clearKnownCredentialIds,
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
  // The OS biometric inactivity timeout (~55s+) tore down the prompt
  // without user intent. Distinct from a real cancel: hosts may
  // surface a re-prompt UI without treating it as the user opting
  // out. The SDK fires PasskeyPrfException with code 'userTimedOut'
  // when assertion or register elapsed time crosses 55_000 ms.
  final prfProvider = PasskeyProvider(PasskeyProviderOptions(rpId: 'my-app.com'));
  final passkey = PasskeyClient(
    deriveSeeds: prfProvider.deriveSeeds,
    isSupported: prfProvider.isSupported,
    createPasskey: prfProvider.createPasskey,
    getKnownCredentialIds: prfProvider.getKnownCredentialIds,
    removeKnownCredentialId: prfProvider.removeKnownCredentialId,
    clearKnownCredentialIds: prfProvider.clearKnownCredentialIds,
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
