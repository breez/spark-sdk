import 'dart:convert';

import 'package:flutter/services.dart';

import 'rust/models.dart' show DeriveSeedsOutput, DeriveSeedsRequest, RegisteredCredential;

/// Result of verifying that the app is associated with its RP domain.
/// Switch on the subtype to handle each outcome.
sealed class DomainAssociation {
  const DomainAssociation();
}

class DomainAssociationAssociated extends DomainAssociation {
  const DomainAssociationAssociated();
}

class DomainAssociationNotAssociated extends DomainAssociation {
  /// Names the verification origin. Diagnostic only.
  final String source;

  /// Human-readable explanation of what was missing. For developer-facing
  /// diagnostics; keep end-user copy platform-neutral.
  final String reason;

  const DomainAssociationNotAssociated({required this.source, required this.reason});
}

class DomainAssociationSkipped extends DomainAssociation {
  /// Why the check was not performed. Not a negative signal: the caller
  /// may still proceed with the ceremony.
  final String reason;

  const DomainAssociationSkipped({required this.reason});
}

/// App-side persistent store of credential IDs registered for an RP. The
/// SDK ships no implementation: bring your own (Keychain, Block Store, etc;
/// see the passkey guide). Calls are best-effort optimizations: failures and
/// 3s timeouts are swallowed, reported via `onRegistryError`, and never block
/// the ceremony.
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

/// Options for constructing a [PasskeyProvider]. `rpId` is required: pass
/// [PasskeyProvider.breezRpId] to opt into Breez's shared RP.
class PasskeyProviderOptions {
  /// Relying Party ID: the domain configured for credential sharing.
  /// Changing it after users register passkeys makes their existing
  /// credentials undiscoverable. Pass [PasskeyProvider.breezRpId] for the
  /// Breez-managed RP (only valid for Breez-registered apps).
  final String rpId;

  /// Display name shown in the OS passkey picker and credential-manager
  /// UIs. Only used at registration; changing it does not affect existing
  /// credentials.
  final String rpName;

  /// Secondary label shown in some passkey managers. Defaults to [rpName].
  /// Only used at registration.
  final String? userName;

  /// Primary label shown in the passkey picker. Defaults to [userName].
  /// Only used at registration.
  final String? userDisplayName;

  /// Optional store of known credential IDs. When set, the SDK merges
  /// stored IDs into the assertion / registration and persists new ones
  /// after success. Best-effort with a 3s timeout: failures fire
  /// [onRegistryError] and the ceremony proceeds.
  final CredentialRegistry? credentialRegistry;

  /// Fired when a [CredentialRegistry] call throws or times out. Never
  /// blocks the ceremony.
  final void Function(RegistryOperation, Object)? onRegistryError;

  const PasskeyProviderOptions({
    required this.rpId,
    required this.rpName,
    this.userName,
    this.userDisplayName,
    this.credentialRegistry,
    this.onRegistryError,
  });
}

/// Error thrown by [PasskeyProvider] when a passkey operation fails.
/// Provides a structured [code] for programmatic handling.
class PasskeyPrfException implements Exception {
  /// Machine-readable error code: one of `userCancelled`, `userTimedOut`,
  /// `prfNotSupported`, `noCredential`, `configuration`,
  /// `credentialAlreadyExists`, `unknown`. `userTimedOut` is the OS biometric
  /// inactivity timeout (distinct from the user dismissing the prompt), so
  /// hosts may safely auto-retry it.
  final String code;
  final String message;

  const PasskeyPrfException({required this.code, required this.message});

  @override
  String toString() => 'PasskeyPrfException($code): $message';
}

/// PRF backend a [PasskeyClient] derives wallet seeds through. The built-in
/// [PasskeyProvider] implements this with platform passkeys; custom backends
/// (hardware key, FIDO2 transport, on-disk key material) implement it
/// directly. Inject an implementation via
/// `PasskeyClientBuilder.withPrfProvider`.
abstract class PrfProvider {
  /// Derive one 32-byte seed per salt in as few OS ceremonies as the
  /// platform supports, plus the credential ID observed in the same
  /// assertion (absent when the backend surfaces none).
  Future<DeriveSeedsOutput> deriveSeeds(DeriveSeedsRequest request);

  /// Whether this device can produce PRF outputs. Hosts gate UX on it.
  Future<bool> isSupported();

  /// Register a new PRF-capable credential. `excludeCredentials` blocks
  /// re-registering the same device, surfaced as a
  /// `credentialAlreadyExists` failure.
  Future<RegisteredCredential> createPasskey(List<Uint8List> excludeCredentials);

  /// Credential IDs the provider has persisted for the current RP. Empty
  /// when the provider keeps no registry.
  Future<List<Uint8List>> getKnownCredentialIds();

  /// Drop a single credential ID from the provider's persisted set.
  Future<void> removeKnownCredentialId(Uint8List credentialId);

  /// Clear the provider's persisted credential-ID set for the current RP.
  Future<void> clearKnownCredentialIds();
}

/// Built-in Flutter passkey PRF provider using platform-native APIs
/// (iOS AuthenticationServices, Android Credential Manager). The
/// default [PrfProvider]; inject a configured instance through
/// [PasskeyClientBuilder.withPrfProvider].
class PasskeyProvider implements PrfProvider {
  /// Breez's shared `keys.breez.technology` RP. Pass as `rpId` to opt in
  /// (only valid for apps registered with Breez); apps with their own RP
  /// domain pass their own string.
  static const String breezRpId = 'keys.breez.technology';

  /// Default `rpName` for the zero-config client when none is supplied.
  static const String defaultRpName = 'Breez';

  static const _channel = MethodChannel('breez_sdk_spark_passkey');

  final String _rpId;
  final String _rpName;
  final String _userName;
  final String _userDisplayName;
  final CredentialRegistry? _credentialRegistry;
  final void Function(RegistryOperation, Object)? _onRegistryError;

  PasskeyProvider(PasskeyProviderOptions options)
    : _rpId = options.rpId,
      _rpName = options.rpName,
      _userName = options.userName ?? options.rpName,
      _userDisplayName = options.userDisplayName ?? (options.userName ?? options.rpName),
      _credentialRegistry = options.credentialRegistry,
      _onRegistryError = options.onRegistryError;

  /// Derive one 32-byte seed per salt from passkey PRF, in as few OS prompts
  /// as the platform supports. Returns the seeds plus the credential ID
  /// observed in the same assertion (absent when none ran), so a subsequent
  /// derive can be pinned to the same credential.
  @override
  Future<DeriveSeedsOutput> deriveSeeds(DeriveSeedsRequest request) async {
    final args = <String, Object?>{
      'salts': request.salts,
      'rpId': _rpId,
      'rpName': _rpName,
      'userName': _userName,
      'userDisplayName': _userDisplayName,
      'autoRegister': false,
    };
    List<Uint8List> effectiveAllow = request.allowCredentials.map(Uint8List.fromList).toList();
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
      args['allowCredentials'] = effectiveAllow.map(base64Encode).toList();
    }
    final preferImmediate = request.preferImmediatelyAvailableCredentials;
    if (preferImmediate != null) {
      args['preferImmediatelyAvailableCredentials'] = preferImmediate;
    }
    try {
      final result = await _channel.invokeMethod<Map<Object?, Object?>>('deriveSeeds', args);
      if (result == null) {
        throw const PasskeyPrfException(code: 'unknown', message: 'deriveSeeds returned null');
      }
      final rawSeeds = (result['seeds'] as List<Object?>?) ?? const <Object?>[];
      final seeds = rawSeeds.cast<String>().map(base64Decode).toList();
      final credentialIdB64 = result['credentialId'] as String?;
      return DeriveSeedsOutput(
        seeds: seeds,
        credentialId: credentialIdB64 == null ? null : base64Decode(credentialIdB64),
      );
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

  /// Register a new passkey with PRF support. `excludeCredentials` blocks
  /// re-registering a device already holding a credential. The user handle
  /// is minted fresh per call (never host-supplied) and returned via
  /// [RegisteredCredential.userId]. Throws [PasskeyPrfException] on failure.
  @override
  Future<RegisteredCredential> createPasskey(List<Uint8List> excludeCredentials) async {
    final args = <String, Object?>{
      'rpId': _rpId,
      'rpName': _rpName,
      'userName': _userName,
      'userDisplayName': _userDisplayName,
    };
    var excludeIds = List<Uint8List>.from(excludeCredentials);
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
      args['excludeCredentials'] = excludeIds.map(base64Encode).toList();
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

  /// Credential IDs the configured [CredentialRegistry] has stored for the
  /// current `rpId`. Empty when no registry is configured.
  @override
  Future<List<Uint8List>> getKnownCredentialIds() async {
    final registry = _credentialRegistry;
    if (registry == null) return const [];
    return _registryReadBestEffort(registry, _rpId, _onRegistryError);
  }

  /// Drop a single credential ID from the configured registry. No-op when
  /// no registry is configured.
  @override
  Future<void> removeKnownCredentialId(Uint8List credentialId) async {
    final registry = _credentialRegistry;
    if (registry == null) return;
    try {
      await registry.remove(_rpId, credentialId).timeout(_registryTimeout);
    } catch (e) {
      _onRegistryError?.call(RegistryOperation.remove, e);
    }
  }

  /// Clear the configured registry's credential IDs for the current
  /// `rpId`. No-op when no registry is configured.
  @override
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
  @override
  Future<bool> isSupported() async {
    final result = await _channel.invokeMethod<bool>('isSupported');
    return result ?? false;
  }

  /// Verify the app is associated with the configured `rpId` for WebAuthn.
  /// Returns [DomainAssociationNotAssociated] only on iOS (Android degrades
  /// to [DomainAssociationSkipped]). The SDK never gates on the result;
  /// hosts pick their own policy.
  Future<DomainAssociation> checkDomainAssociation() async {
    try {
      final result = await _channel.invokeMethod<Map<Object?, Object?>>('checkDomainAssociation', {
        'rpId': _rpId,
      });
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
      'ERR_CREDENTIAL_ALREADY_EXISTS' => PasskeyPrfException(
        code: 'credentialAlreadyExists',
        message: message,
      ),
      _ => PasskeyPrfException(code: 'unknown', message: message),
    };
  }
}
