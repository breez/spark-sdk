import 'dart:convert';

import 'package:flutter/services.dart';

import 'rust/models.dart' show Config, DeriveSeedsRequest, PasskeyConfig, RegisteredCredential;
import 'rust/passkey.dart' show PasskeyClient;

/// Result of a domain-association verification check against the
/// platform's well-known configuration source. Mirrors the Rust
/// `DomainAssociation` enum shape one-to-one so callers can switch
/// on `kind` regardless of which native plugin produced the result.
sealed class DomainAssociation {
  const DomainAssociation();
}

class DomainAssociationAssociated extends DomainAssociation {
  const DomainAssociationAssociated();
}

class DomainAssociationNotAssociated extends DomainAssociation {
  /// Names the verification origin (e.g. "Apple AASA CDN",
  /// "Google Digital Asset Links API"). Diagnostic only.
  final String source;

  /// Human-readable explanation of what was missing. Surface this in
  /// developer-facing diagnostics; end-user copy should be platform-
  /// neutral.
  final String reason;

  const DomainAssociationNotAssociated({required this.source, required this.reason});
}

class DomainAssociationSkipped extends DomainAssociation {
  /// Why the check was not performed (provider has no verification
  /// source, or the probe itself could not complete). Not a negative
  /// signal; the caller may proceed with the WebAuthn ceremony.
  final String reason;

  const DomainAssociationSkipped({required this.reason});
}

/// App-side persistent store of credential IDs registered for an RP.
/// The SDK does not ship a built-in implementation: bring your own
/// (Keychain on iOS, Block Store + SharedPreferences on Android, or
/// any custom backend). See the reference implementations in the
/// passkey guide.
///
/// All methods are called from the SDK as best-effort optimizations:
/// failures and timeouts (3s) are swallowed and surfaced via the
/// caller's `onRegistryError`; they never block the WebAuthn ceremony.
abstract class CredentialRegistry {
  Future<List<Uint8List>> read(String rpId);
  Future<void> add(String rpId, Uint8List credentialId);
  Future<void> remove(String rpId, Uint8List credentialId);
  Future<void> clear(String rpId);
}

/// Discriminator for the `onRegistryError` callback.
enum RegistryOperation { read, add, remove, clear }

const Duration _registryTimeout = Duration(seconds: 3);

const String _credentialRegistryHelpSuffix =
    ' (No CredentialRegistry was supplied to PasskeyProvider; '
    'if you expect the SDK to auto-discover known credentials, see '
    'https://sdk-doc-spark.breez.technology/guide/passkey.html#credentialregistry)';

Future<List<Uint8List>> _registryReadBestEffort(
  CredentialRegistry registry,
  String rpId,
  void Function(RegistryOperation, Object)? onRegistryError,
) async {
  try {
    return await registry.read(rpId).timeout(_registryTimeout);
  } catch (e) {
    onRegistryError?.call(RegistryOperation.read, e);
    return const [];
  }
}

void _registryAddFireAndForget(
  CredentialRegistry registry,
  String rpId,
  Uint8List credentialId,
  void Function(RegistryOperation, Object)? onRegistryError,
) {
  registry.add(rpId, credentialId).timeout(_registryTimeout).catchError((Object e) {
    onRegistryError?.call(RegistryOperation.add, e);
  });
}

/// Options for constructing a [PasskeyProvider]. `rpId` is required : 
/// pass [PasskeyProvider.breezRpId] to opt into Breez's shared RP.
class PasskeyProviderOptions {
  /// Relying Party ID. Must match the domain configured for cross-platform
  /// credential sharing. Changing this after users have registered passkeys
  /// will make their existing credentials undiscoverable.
  ///
  /// Pass [PasskeyProvider.breezRpId] to opt into the Breez-managed
  /// `keys.breez.technology` RP (only valid for Breez-registered apps).
  final String rpId;

  /// RP display name shown during credential registration. Only used when
  /// creating new passkeys.
  ///
  /// Defaults to `'Breez SDK'`.
  final String rpName;

  /// User name stored with the credential, shown as a secondary label in
  /// some passkey managers. Defaults to [rpName]. Only used during
  /// registration.
  final String? userName;

  /// User display name shown as the primary label in the passkey picker.
  /// Defaults to [userName]. Only used during registration.
  final String? userDisplayName;

  /// Optional opt-in registry. When set, the Dart-side wrapper
  /// merges stored IDs into `allowCredentialIds` / `excludeCredentialIds`
  /// before the MethodChannel call and writes the asserted /
  /// created credential ID back after success. The native plugin
  /// never sees the registry. Calls are best-effort with a 3s
  /// timeout; failures fire [onRegistryError] and the ceremony
  /// proceeds.
  final CredentialRegistry? credentialRegistry;

  /// Fired when a [CredentialRegistry] call throws or times out.
  /// Best-effort: invocation never blocks ceremony progress.
  final void Function(RegistryOperation, Object)? onRegistryError;

  const PasskeyProviderOptions({
    required this.rpId,
    this.rpName = 'Breez SDK',
    this.userName,
    this.userDisplayName,
    this.credentialRegistry,
    this.onRegistryError,
  });
}

/// Error thrown by [PasskeyProvider] when a passkey operation fails.
/// Provides a structured [code] for programmatic handling.
class PasskeyPrfException implements Exception {
  /// Machine-readable error code:
  /// - `userCancelled`, `userTimedOut`, `prfNotSupported`, `noCredential`,
  ///   `configuration`, `credentialAlreadyExists`, `unknown`.
  ///
  /// `userTimedOut` distinguishes the OS biometric inactivity timeout
  /// (~55s+ with no user interaction) from `userCancelled` (the user
  /// actively dismissed the prompt). Hosts may auto-retry on
  /// `userTimedOut` without treating it as user intent to abandon.
  final String code;
  final String message;

  const PasskeyPrfException({required this.code, required this.message});

  @override
  String toString() => 'PasskeyPrfException($code): $message';
}

/// Flutter passkey PRF provider using platform-native APIs (iOS
/// AuthenticationServices, Android Credential Manager). Wires directly
/// into the [PasskeyClient] constructor:
///
/// ```dart
/// final provider = PasskeyProvider();
/// final client = PasskeyClient(
///   deriveSeeds: provider.deriveSeeds,
///   isSupported: provider.isSupported,
///   createPasskey: provider.createPasskey,
///   getKnownCredentialIds: provider.getKnownCredentialIds,
///   removeKnownCredentialId: provider.removeKnownCredentialId,
///   clearKnownCredentialIds: provider.clearKnownCredentialIds,
/// );
/// ```
class PasskeyProvider {
  /// Constant identifying Breez's shared `keys.breez.technology` RP.
  /// Pass as `rpId` when opting into the Breez-managed Relying Party
  /// (only valid for apps registered with Breez). Apps with their own
  /// RP domain pass their own string.
  static const String breezRpId = 'keys.breez.technology';

  static const _channel = MethodChannel('breez_sdk_spark_passkey');

  final String _rpId;
  final String _rpName;
  final String _userName;
  final String _userDisplayName;
  final CredentialRegistry? _credentialRegistry;
  final void Function(RegistryOperation, Object)? _onRegistryError;
  Uint8List? _lastObservedCredentialId;

  PasskeyProvider(PasskeyProviderOptions options)
    : _rpId = options.rpId,
      _rpName = options.rpName,
      _userName = options.userName ?? options.rpName,
      _userDisplayName = options.userDisplayName ?? (options.userName ?? options.rpName),
      _credentialRegistry = options.credentialRegistry,
      _onRegistryError = options.onRegistryError;

  /// Take ownership of the credential ID captured by the most recent
  /// successful assertion. Returns `null` if no assertion has
  /// completed since the last call.
  Uint8List? takeLastObservedCredentialId() {
    final v = _lastObservedCredentialId;
    _lastObservedCredentialId = null;
    return v;
  }

  /// Derive multiple 32-byte seeds from passkey PRF with the given salts
  /// in as few OS ceremonies as the platform supports (dual-salt
  /// assertion where available). For the `salts.length == 1` case the
  /// native plugin short-circuits to a single-salt assertion (one
  /// prompt). Used by the SDK's `setup_wallet` orchestration to collapse
  /// master + label derivation into one prompt.
  Future<List<Uint8List>> deriveSeeds(DeriveSeedsRequest request) async {
    final args = <String, Object?>{
      'salts': request.salts,
      'rpId': _rpId,
      'rpName': _rpName,
      'userName': _userName,
      'userDisplayName': _userDisplayName,
      'autoRegister': false,
    };
    List<Uint8List> effectiveAllow = request.allowCredentialIds
        .map(Uint8List.fromList)
        .toList();
    // Auto-merge registry IDs into the allow-list. Dart-side dance:
    // the native plugin never sees the registry.
    final registry = _credentialRegistry;
    if (registry != null) {
      final registryIds = await _registryReadBestEffort(registry, _rpId, _onRegistryError);
      if (registryIds.isNotEmpty) {
        final seen = effectiveAllow.map(base64Encode).toSet();
        final merged = <Uint8List>[...effectiveAllow];
        for (final id in registryIds) {
          final key = base64Encode(id);
          if (seen.add(key)) {
            merged.add(id);
          }
        }
        effectiveAllow = merged;
      }
    }
    if (effectiveAllow.isNotEmpty) {
      args['allowCredentialIds'] = effectiveAllow.map(base64Encode).toList();
    }
    final preferImmediate = request.preferImmediatelyAvailableCredentials;
    if (preferImmediate != null) {
      args['preferImmediatelyAvailableCredentials'] = preferImmediate;
    }
    try {
      final result = await _channel.invokeMethod<List<Object?>>('deriveSeeds', args);
      if (result == null) {
        throw const PasskeyPrfException(
          code: 'unknown',
          message: 'deriveSeeds returned null',
        );
      }
      return result.cast<String>().map((b64) => base64Decode(b64)).toList();
    } on PlatformException catch (e) {
      var mapped = _mapPlatformException(e);
      // Augment CredentialNotFound when host had no allow-list and no
      // registry so integrators can discover the opt-in path.
      if (mapped.code == 'noCredential' && effectiveAllow.isEmpty && registry == null) {
        mapped = PasskeyPrfException(
          code: mapped.code,
          message: mapped.message + _credentialRegistryHelpSuffix,
        );
      }
      throw mapped;
    }
  }

  /// Register a new passkey with PRF support. `excludeCredentialIds`
  /// is the only per-call knob: branding fields (`userName`,
  /// `userDisplayName`) live on the constructor.
  ///
  /// `user.id` is never host-supplied: the native plugin mints a fresh
  /// random 16-byte handle per call and surfaces it via
  /// [RegisteredCredential.userId]. Throws [PasskeyPrfException] on
  /// failure.
  Future<RegisteredCredential> createPasskey(List<Uint8List> excludeCredentialIds) async {
    final args = <String, Object?>{
      'rpId': _rpId,
      'rpName': _rpName,
      'userName': _userName,
      'userDisplayName': _userDisplayName,
    };
    var excludeIds = List<Uint8List>.from(excludeCredentialIds);
    final registry = _credentialRegistry;
    if (registry != null) {
      final registryIds = await _registryReadBestEffort(registry, _rpId, _onRegistryError);
      if (registryIds.isNotEmpty) {
        final seen = excludeIds.map(base64Encode).toSet();
        for (final id in registryIds) {
          final key = base64Encode(id);
          if (seen.add(key)) {
            excludeIds.add(id);
          }
        }
      }
    }
    if (excludeIds.isNotEmpty) {
      args['excludeCredentialIds'] = excludeIds.map(base64Encode).toList();
    }
    try {
      final result = await _channel.invokeMethod<Map<Object?, Object?>>('createPasskey', args);
      final map = result!;
      final credentialId = base64Decode(map['credentialId'] as String);
      final userId = base64Decode(map['userId'] as String);
      final aaguidB64 = map['aaguid'] as String?;
      final aaguid = aaguidB64 == null ? null : base64Decode(aaguidB64);
      final backupEligible = map['backupEligible'] as bool?;
      // Persist new credential ID to the registry post-success.
      if (registry != null) {
        _registryAddFireAndForget(registry, _rpId, credentialId, _onRegistryError);
      }
      return RegisteredCredential(
        credentialId: credentialId,
        userId: userId,
        aaguid: aaguid,
        backupEligible: backupEligible,
      );
    } on PlatformException catch (e) {
      throw _mapPlatformException(e);
    }
  }

  /// Read the credential IDs the configured [CredentialRegistry] has
  /// stored for the current `rpId`. Empty list when no registry is
  /// configured. Backs `PasskeyClient.credentials().get()`.
  Future<List<Uint8List>> getKnownCredentialIds() async {
    final registry = _credentialRegistry;
    if (registry == null) return const [];
    return _registryReadBestEffort(registry, _rpId, _onRegistryError);
  }

  /// Drop a single credential ID from the configured registry. No-op
  /// when no registry is configured. Backs
  /// `PasskeyClient.credentials().remove(id)`.
  Future<void> removeKnownCredentialId(Uint8List credentialId) async {
    final registry = _credentialRegistry;
    if (registry == null) return;
    try {
      await registry.remove(_rpId, credentialId).timeout(_registryTimeout);
    } catch (e) {
      _onRegistryError?.call(RegistryOperation.remove, e);
    }
  }

  /// Clear the configured registry's persisted credential-ID list for
  /// the current `rpId`. No-op when no registry is configured. Backs
  /// `PasskeyClient.credentials().clear()`.
  Future<void> clearKnownCredentialIds() async {
    final registry = _credentialRegistry;
    if (registry == null) return;
    try {
      await registry.clear(_rpId).timeout(_registryTimeout);
    } catch (e) {
      _onRegistryError?.call(RegistryOperation.clear, e);
    }
  }

  /// Whether this device supports passkey PRF.
  Future<bool> isSupported() async {
    final result = await _channel.invokeMethod<bool>('isSupported');
    return result ?? false;
  }

  /// Verify the configured `rpId` is a valid scope for WebAuthn from
  /// the running app's identity. Returns a typed [DomainAssociation]:
  /// `Associated` when verification succeeded, `NotAssociated` with a
  /// concrete reason when it fails (iOS only; Android degrades to
  /// `Skipped`), or `Skipped` when the probe couldn't complete (no
  /// network, no signing certificate, etc.).
  ///
  /// The SDK never gates internally; hosts pick their own policy.
  Future<DomainAssociation> checkDomainAssociation() async {
    try {
      final result = await _channel.invokeMethod<Map<Object?, Object?>>(
        'checkDomainAssociation',
        {'rpId': _rpId},
      );
      final map = result ?? {};
      final kind = map['kind'] as String?;
      switch (kind) {
        case 'Associated':
          return const DomainAssociationAssociated();
        case 'NotAssociated':
          return DomainAssociationNotAssociated(
            source: (map['source'] as String?) ?? 'unknown',
            reason: (map['reason'] as String?) ?? '',
          );
        case 'Skipped':
        default:
          return DomainAssociationSkipped(reason: (map['reason'] as String?) ?? '');
      }
    } on PlatformException catch (e) {
      // Treat platform exceptions as Skipped so the caller has a
      // single contract to handle.
      return DomainAssociationSkipped(reason: e.message ?? e.code);
    }
  }

  /// Map native [PlatformException] error codes to [PasskeyPrfException].
  static PasskeyPrfException _mapPlatformException(PlatformException e) {
    final message = e.message ?? 'Unknown passkey error';
    return switch (e.code) {
      'ERR_USER_CANCELLED' => PasskeyPrfException(code: 'userCancelled', message: message),
      'ERR_USER_TIMED_OUT' => PasskeyPrfException(code: 'userTimedOut', message: message),
      'ERR_PRF_NOT_SUPPORTED' => PasskeyPrfException(code: 'prfNotSupported', message: message),
      'ERR_NO_CREDENTIAL' => PasskeyPrfException(code: 'noCredential', message: message),
      'ERR_CONFIGURATION' => PasskeyPrfException(code: 'configuration', message: message),
      'ERR_CREDENTIAL_ALREADY_EXISTS' =>
        PasskeyPrfException(code: 'credentialAlreadyExists', message: message),
      _ => PasskeyPrfException(code: 'unknown', message: message),
    };
  }
}

/// Convenience factory: builds the platform [PasskeyProvider] with
/// sensible defaults and wires it to a new [PasskeyClient], forwarding
/// the Breez API key from the SDK [Config].
///
/// Equivalent to constructing [PasskeyProvider] and passing all six
/// callbacks plus `breezApiKey: sdkConfig.apiKey` to [PasskeyClient].
///
/// Hosts that need a custom `PrfProvider` (CLI / YubiKey / FIDO2) or
/// non-default [PasskeyProviderOptions] should construct
/// [PasskeyClient] directly with the appropriate callbacks instead.
PasskeyClient createPasskeyClient({
  required String rpId,
  required Config sdkConfig,
  PasskeyConfig? passkeyConfig,
  String? rpName,
  String? userName,
  String? userDisplayName,
  CredentialRegistry? credentialRegistry,
  void Function(RegistryOperation, Object)? onRegistryError,
}) {
  final provider = PasskeyProvider(
    PasskeyProviderOptions(
      rpId: rpId,
      rpName: rpName ?? 'Breez SDK',
      userName: userName,
      userDisplayName: userDisplayName,
      credentialRegistry: credentialRegistry,
      onRegistryError: onRegistryError,
    ),
  );
  return PasskeyClient(
    deriveSeeds: provider.deriveSeeds,
    isSupported: provider.isSupported,
    createPasskey: provider.createPasskey,
    getKnownCredentialIds: provider.getKnownCredentialIds,
    removeKnownCredentialId: provider.removeKnownCredentialId,
    clearKnownCredentialIds: provider.clearKnownCredentialIds,
    breezApiKey: sdkConfig.apiKey,
    config: passkeyConfig,
  );
}
