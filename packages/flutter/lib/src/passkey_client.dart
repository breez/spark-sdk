import 'dart:async';
import 'dart:typed_data';

import 'rust/models.dart'
    show
        ConnectWithPasskeyRequest,
        ConnectWithPasskeyResponse,
        DeriveSeedsRequest,
        PasskeyAvailability,
        PasskeyConfig,
        RegisterRequest,
        RegisterResponse,
        RegisteredCredential,
        SignInRequest,
        SignInResponse;
import 'rust/passkey.dart' as rust;
import 'passkey_prf_provider.dart' show PasskeyProvider, PasskeyProviderOptions;

/// Public-facing PasskeyClient entry point for Flutter. Wraps the
/// FRB-generated client so callers can pass a [PasskeyProvider]
/// directly, matching the `(provider, breezApiKey, config)` shape used
/// by every other binding (Swift, Kotlin, RN, WASM).
///
/// The FRB-generated class requires six individual callbacks because
/// `flutter_rust_bridge` cannot pass a trait object across FFI. This
/// wrapper translates a [PasskeyProvider] into those callbacks once,
/// instead of asking every call site to repeat the wiring. Custom PRF
/// implementations that don't go through the built-in
/// [PasskeyProvider] can use [PasskeyClient.fromCallbacks].
class PasskeyClient {
  final rust.PasskeyClient _inner;

  PasskeyClient(
    PasskeyProvider provider, {
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
    required FutureOr<List<Uint8List>> Function(DeriveSeedsRequest) deriveSeeds,
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

  /// Client wired to the built-in [PasskeyProvider]. Defaults to the
  /// Breez shared RP (`keys.breez.technology`), so a Breez-registered
  /// app needs only its relay key; pass [rpId] / [rpName] to use your own
  /// RP. Apps that need a credential registry or a custom backend build
  /// the provider themselves and inject it via [PasskeyClientBuilder] (or
  /// use [fromCallbacks]).
  PasskeyClient.builtIn({
    String? breezApiKey,
    String rpId = PasskeyProvider.breezRpId,
    String rpName = PasskeyProvider.defaultRpName,
    PasskeyConfig? config,
  }) : this(
          PasskeyProvider(
            PasskeyProviderOptions(rpId: rpId, rpName: rpName),
          ),
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
/// zero-config Breez-RP case, use [PasskeyClient.builtIn]; for a fully
/// custom PRF backend, use [PasskeyClient.fromCallbacks].
///
/// ```dart
/// final provider = PasskeyProvider(PasskeyProviderOptions(rpId: rpId, rpName: rpName));
/// final client = PasskeyClientBuilder(breezApiKey: apiKey)
///     .withPrfProvider(provider)
///     .build();
/// ```
class PasskeyClientBuilder {
  PasskeyClientBuilder({
    this.breezApiKey,
    this.rpId = PasskeyProvider.breezRpId,
    this.rpName = PasskeyProvider.defaultRpName,
    this.config,
  });

  /// Breez relay key for authenticated (NIP-42) label storage. Pass
  /// `null` for public relays only.
  final String? breezApiKey;

  /// Relying Party ID for the default provider. Defaults to
  /// [PasskeyProvider.breezRpId]. Ignored when a provider is injected
  /// via [withPrfProvider].
  final String rpId;

  /// Relying Party name for the default provider. Defaults to
  /// [PasskeyProvider.defaultRpName]. Ignored when a provider is
  /// injected.
  final String rpName;

  /// Optional [PasskeyConfig] (e.g. a default label).
  final PasskeyConfig? config;

  PasskeyProvider? _provider;

  /// Inject the [PasskeyProvider] the client derives seeds through.
  /// Supersedes [rpId] / [rpName] (the injected provider owns its RP).
  PasskeyClientBuilder withPrfProvider(PasskeyProvider provider) {
    _provider = provider;
    return this;
  }

  /// Construct the client. Falls back to a default [PasskeyProvider] on
  /// the configured [rpId] / [rpName] when no provider was injected.
  PasskeyClient build() {
    final provider = _provider ??
        PasskeyProvider(PasskeyProviderOptions(rpId: rpId, rpName: rpName));
    return PasskeyClient(provider, breezApiKey: breezApiKey, config: config);
  }
}
