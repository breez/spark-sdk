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

/// Relying Party name used by [PasskeyClient.builtIn] / the
/// zero-provider [PasskeyClientBuilder.build] path. Surfaces in some
/// credential-manager UIs. Apps that want their own RP name build a
/// [PasskeyProvider] explicitly and inject it through
/// [PasskeyClientBuilder.withPrfProvider].
const String _defaultRpName = 'Breez';

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

  /// Zero-config client wired to the built-in [PasskeyProvider] on the
  /// Breez shared RP (`keys.breez.technology`), so a Breez-registered
  /// app needs only its relay key. Apps with their own RP, a credential
  /// registry, or a custom backend build the provider themselves and
  /// inject it via [PasskeyClientBuilder] (or use [fromCallbacks]).
  PasskeyClient.builtIn({String? breezApiKey, PasskeyConfig? config})
      : this(
          PasskeyProvider(
            PasskeyProviderOptions(
              rpId: PasskeyProvider.breezRpId,
              rpName: _defaultRpName,
            ),
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
  PasskeyClientBuilder({this.breezApiKey, this.config});

  /// Breez relay key for authenticated (NIP-42) label storage. Pass
  /// `null` for public relays only.
  final String? breezApiKey;

  /// Optional [PasskeyConfig] (e.g. a default label).
  final PasskeyConfig? config;

  PasskeyProvider? _provider;

  /// Inject the [PasskeyProvider] the client derives seeds through.
  PasskeyClientBuilder withPrfProvider(PasskeyProvider provider) {
    _provider = provider;
    return this;
  }

  /// Construct the client. Falls back to a default [PasskeyProvider] on
  /// the Breez RP when no provider was injected.
  PasskeyClient build() {
    final provider = _provider ??
        PasskeyProvider(
          PasskeyProviderOptions(
            rpId: PasskeyProvider.breezRpId,
            rpName: _defaultRpName,
          ),
        );
    return PasskeyClient(provider, breezApiKey: breezApiKey, config: config);
  }
}
