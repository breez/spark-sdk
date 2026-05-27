import 'dart:async';
import 'dart:typed_data';

import 'rust/models.dart'
    show
        ConnectWithPasskeyRequest,
        ConnectWithPasskeyResponse,
        DeriveSeedsOutput,
        DeriveSeedsRequest,
        PasskeyAvailability,
        PasskeyConfig,
        RegisterRequest,
        RegisterResponse,
        RegisteredCredential,
        SignInRequest,
        SignInResponse;
import 'rust/passkey.dart' as rust;
import 'passkey_prf_provider.dart' show PasskeyProvider, PasskeyProviderOptions, PrfProvider;

/// Public-facing PasskeyClient entry point for Flutter. The default
/// constructor wires the built-in [PasskeyProvider] on the Breez shared
/// RP, so a Breez-registered app needs only its relay key.
///
/// The FRB-generated client takes six individual callbacks because
/// `flutter_rust_bridge` cannot pass a trait object across FFI; this
/// wrapper translates a [PasskeyProvider] into those callbacks once.
class PasskeyClient {
  final rust.PasskeyClient _inner;

  /// Zero-config client wired to the built-in [PasskeyProvider]. Defaults
  /// to the Breez shared RP (`keys.breez.technology`), so a
  /// Breez-registered app needs only its relay key; set `rpId` / `rpName`
  /// on the [config] to use your own RP.
  PasskeyClient({String? breezApiKey, PasskeyConfig? config})
      : this._fromProvider(
          PasskeyProvider(
            PasskeyProviderOptions(
              rpId: config?.rpId ?? PasskeyProvider.breezRpId,
              rpName: config?.rpName ?? PasskeyProvider.defaultRpName,
            ),
          ),
          breezApiKey: breezApiKey,
          config: config,
        );

  /// Shared wiring that translates a [PrfProvider]'s six methods into the
  /// FRB client's six callbacks. The zero-config constructor and
  /// [PasskeyClientBuilder.build] both redirect here; public provider
  /// injection goes through [PasskeyClientBuilder.withPrfProvider], aligning
  /// Flutter with the other bindings (no provider-taking public constructor).
  PasskeyClient._fromProvider(
    PrfProvider provider, {
    String? breezApiKey,
    PasskeyConfig? config,
  }) : _inner = rust.PasskeyClient(
          deriveSeeds: provider.deriveSeeds,
          isSupported: provider.isSupported,
          createPasskey: provider.createPasskey,
          getKnownCredentialIds: provider.getKnownCredentialIds,
          removeKnownCredentialId: provider.removeKnownCredentialId,
          clearKnownCredentialIds: provider.clearKnownCredentialIds,
          breezApiKey: breezApiKey,
          config: config,
        );

  /// Construct from raw PRF callbacks. Use this when the built-in
  /// [PasskeyProvider] doesn't fit (hardware key, FIDO2 transport,
  /// air-gapped backup file, etc.).
  PasskeyClient.fromCallbacks({
    required FutureOr<DeriveSeedsOutput> Function(DeriveSeedsRequest) deriveSeeds,
    required FutureOr<bool> Function() isSupported,
    required FutureOr<RegisteredCredential> Function(List<Uint8List>) createPasskey,
    required FutureOr<List<Uint8List>> Function() getKnownCredentialIds,
    required FutureOr<void> Function(Uint8List) removeKnownCredentialId,
    required FutureOr<void> Function() clearKnownCredentialIds,
    String? breezApiKey,
    PasskeyConfig? config,
  }) : _inner = rust.PasskeyClient(
          deriveSeeds: deriveSeeds,
          isSupported: isSupported,
          createPasskey: createPasskey,
          getKnownCredentialIds: getKnownCredentialIds,
          removeKnownCredentialId: removeKnownCredentialId,
          clearKnownCredentialIds: clearKnownCredentialIds,
          breezApiKey: breezApiKey,
          config: config,
        );

  Future<PasskeyAvailability> checkAvailability() => _inner.checkAvailability();

  Future<ConnectWithPasskeyResponse> connectWithPasskey({
    required ConnectWithPasskeyRequest request,
  }) =>
      _inner.connectWithPasskey(request: request);

  rust.PasskeyCredentials credentials() => _inner.credentials();

  rust.PasskeyLabels labels() => _inner.labels();

  Future<RegisterResponse> register({required RegisterRequest request}) =>
      _inner.register(request: request);

  Future<SignInResponse> signIn({required SignInRequest request}) =>
      _inner.signIn(request: request);
}

/// Builder for a [PasskeyClient] backed by a caller-supplied
/// [PasskeyProvider].
///
/// Use this when you need a configured provider (custom `rpId` /
/// `rpName`, a credential registry, rotating `userName`). For the
/// zero-config Breez-RP case, use the default [PasskeyClient] constructor;
/// for a fully custom PRF backend, use [PasskeyClient.fromCallbacks].
///
/// ```dart
/// final provider = PasskeyProvider(PasskeyProviderOptions(rpId: rpId, rpName: rpName));
/// final client = PasskeyClientBuilder(breezApiKey: apiKey)
///     .withPrfProvider(provider)
///     .build();
/// ```
class PasskeyClientBuilder {
  PasskeyClientBuilder({this.breezApiKey, this.config});

  /// Breez relay key for authenticated (NIP-42) label storage. Pass
  /// `null` for public relays only.
  final String? breezApiKey;

  /// Passkey client config. `rpId` / `rpName` configure the default
  /// provider (ignored when a provider is injected via [withPrfProvider]);
  /// `defaultLabel` is the label-store default.
  final PasskeyConfig? config;

  PrfProvider? _provider;

  /// Inject the [PrfProvider] the client derives seeds through: the
  /// built-in [PasskeyProvider] or any custom PRF backend. Supersedes
  /// the config's `rpId` / `rpName` (the injected provider owns its RP).
  PasskeyClientBuilder withPrfProvider(PrfProvider provider) {
    _provider = provider;
    return this;
  }

  /// Construct the client. Falls back to a default [PasskeyProvider] on
  /// the config's `rpId` / `rpName` (default: the Breez RP) when no
  /// provider was injected.
  PasskeyClient build() {
    final provider = _provider ??
        PasskeyProvider(
          PasskeyProviderOptions(
            rpId: config?.rpId ?? PasskeyProvider.breezRpId,
            rpName: config?.rpName ?? PasskeyProvider.defaultRpName,
          ),
        );
    return PasskeyClient._fromProvider(provider, breezApiKey: breezApiKey, config: config);
  }
}
