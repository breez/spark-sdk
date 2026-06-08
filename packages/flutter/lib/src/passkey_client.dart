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
        PasskeyCredential,
        PasskeyProviderOptions,
        RegisterRequest,
        RegisterResponse,
        SignInRequest,
        SignInResponse;
import 'rust/passkey.dart' as rust;
import 'passkey_prf_provider.dart' show PasskeyProvider, PrfProvider;

/// Passkey-based wallet client. The default constructor wires the built-in
/// [PasskeyProvider] on the Breez shared RP, so a Breez-registered app needs
/// only its relay key.
class PasskeyClient {
  final rust.PasskeyClient _inner;

  /// Zero-config client on the Breez shared RP (`keys.breez.technology`); set
  /// `providerOptions` on the [config] to use your own RP.
  PasskeyClient({String? breezApiKey, PasskeyConfig? config})
    : this._fromProvider(
        PasskeyProvider(config?.providerOptions ?? const PasskeyProviderOptions()),
        breezApiKey: breezApiKey,
        config: config,
      );

  /// Shared wiring that adapts a [PrfProvider] to the underlying client's
  /// callbacks. Public provider injection goes through
  /// [PasskeyClientBuilder.withPrfProvider].
  PasskeyClient._fromProvider(PrfProvider provider, {String? breezApiKey, PasskeyConfig? config})
    : _inner = rust.PasskeyClient(
        deriveSeeds: provider.deriveSeeds,
        isSupported: provider.isSupported,
        createPasskey: provider.createPasskey,
        breezApiKey: breezApiKey,
        config: config,
      );

  /// Construct from raw PRF callbacks. Use this when the built-in
  /// [PasskeyProvider] doesn't fit (hardware key, FIDO2 transport,
  /// air-gapped backup file, etc.).
  PasskeyClient.fromCallbacks({
    required FutureOr<DeriveSeedsOutput> Function(DeriveSeedsRequest) deriveSeeds,
    required FutureOr<bool> Function() isSupported,
    required FutureOr<PasskeyCredential> Function(List<Uint8List>) createPasskey,
    String? breezApiKey,
    PasskeyConfig? config,
  }) : _inner = rust.PasskeyClient(
         deriveSeeds: deriveSeeds,
         isSupported: isSupported,
         createPasskey: createPasskey,
         breezApiKey: breezApiKey,
         config: config,
       );

  Future<PasskeyAvailability> checkAvailability() => _inner.checkAvailability();

  Future<ConnectWithPasskeyResponse> connectWithPasskey({required ConnectWithPasskeyRequest request}) =>
      _inner.connectWithPasskey(request: request);

  rust.PasskeyLabels labels() => _inner.labels();

  Future<RegisterResponse> register({required RegisterRequest request}) => _inner.register(request: request);

  Future<SignInResponse> signIn({required SignInRequest request}) => _inner.signIn(request: request);
}

/// Builds a [PasskeyClient] backed by a caller-supplied [PrfProvider]. Use
/// this for a custom PRF backend (hardware key, FIDO2, file-backed); set
/// `providerOptions` on the [config] for the built-in provider instead.
class PasskeyClientBuilder {
  PasskeyClientBuilder({this.breezApiKey, this.config});

  /// Breez relay key for authenticated (NIP-42) label storage. Pass
  /// `null` for public relays only.
  final String? breezApiKey;

  /// Passkey client config. `providerOptions` configures the default
  /// provider (ignored when a provider is injected via [withPrfProvider]);
  /// `defaultLabel` is the label-store default.
  final PasskeyConfig? config;

  PrfProvider? _provider;

  /// Inject the [PrfProvider] the client derives seeds through: the
  /// built-in [PasskeyProvider] or any custom PRF backend. Supersedes the
  /// config's `providerOptions`.
  PasskeyClientBuilder withPrfProvider(PrfProvider provider) {
    _provider = provider;
    return this;
  }

  /// Construct the client. Falls back to a default [PasskeyProvider] on
  /// the config's `providerOptions` (default: the Breez RP) when no
  /// provider was injected.
  PasskeyClient build() {
    final provider = _provider ?? PasskeyProvider(config?.providerOptions ?? const PasskeyProviderOptions());
    return PasskeyClient._fromProvider(provider, breezApiKey: breezApiKey, config: config);
  }
}
