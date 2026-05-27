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

/// Passkey-based wallet client. The default constructor wires the built-in
/// [PasskeyProvider] on the Breez shared RP, so a Breez-registered app needs
/// only its relay key.
class PasskeyClient {
  final rust.PasskeyClient _inner;

  /// Zero-config client on the Breez shared RP (`keys.breez.technology`); set
  /// `rpId` / `rpName` on the [config] to use your own RP.
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

  /// Shared wiring that adapts a [PrfProvider] to the underlying client's
  /// callbacks. Public provider injection goes through
  /// [PasskeyClientBuilder.withPrfProvider].
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

/// Builds a [PasskeyClient] backed by a caller-supplied [PrfProvider]. Use
/// this when you need a configured provider (custom `rpId` / `rpName`, a
/// credential registry, rotating `userName`).
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
  /// built-in [PasskeyProvider] or any custom PRF backend. Supersedes the
  /// config's `rpId` / `rpName`.
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
