import 'dart:convert';

import 'package:flutter/services.dart';

import 'rust/models.dart'
    show CreatePasskeyRequest, DeriveSeedsRequest, RegisteredCredential;

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

/// Options for constructing a [PasskeyProvider].
class PasskeyProviderOptions {
  /// Relying Party ID. Must match the domain configured for cross-platform
  /// credential sharing. Changing this after users have registered passkeys
  /// will make their existing credentials undiscoverable.
  ///
  /// Defaults to `'keys.breez.technology'`.
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

  /// When `true`, [PasskeyProvider.deriveSeeds] automatically creates a
  /// new passkey if none exists, then retries the assertion. When `false`
  /// (default), throws [PasskeyPrfException] with code `noCredential` and
  /// the caller drives registration via [PasskeyProvider.createPasskey].
  final bool autoRegister;

  /// Restrict assertion (sign-in) to one of these credential IDs.
  /// The platform refuses any other credential for this RP. Use this
  /// to bind sign-in to a specific passkey the caller has registered,
  /// instead of letting the platform pick any sibling credential that
  /// happens to share the RP. When null or empty, the platform picks
  /// any credential matching the RP. Critical for deterministic seed
  /// derivation when multiple credentials might exist for the same RP.
  final List<Uint8List>? allowCredentialIds;

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
    this.rpId = 'keys.breez.technology',
    this.rpName = 'Breez SDK',
    this.userName,
    this.userDisplayName,
    this.autoRegister = false,
    this.allowCredentialIds,
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
  static const _channel = MethodChannel('breez_sdk_spark_passkey');

  final String _rpId;
  final String _rpName;
  final String _userName;
  final String _userDisplayName;
  final bool _autoRegister;
  final List<Uint8List>? _allowCredentialIds;
  final CredentialRegistry? _credentialRegistry;
  final void Function(RegistryOperation, Object)? _onRegistryError;
  Uint8List? _lastObservedCredentialId;

  PasskeyProvider([PasskeyProviderOptions? options])
    : _rpId = options?.rpId ?? 'keys.breez.technology',
      _rpName = options?.rpName ?? 'Breez SDK',
      _userName = options?.userName ?? (options?.rpName ?? 'Breez SDK'),
      _userDisplayName =
          options?.userDisplayName ?? (options?.userName ?? (options?.rpName ?? 'Breez SDK')),
      _autoRegister = options?.autoRegister ?? false,
      _allowCredentialIds = options?.allowCredentialIds,
      _credentialRegistry = options?.credentialRegistry,
      _onRegistryError = options?.onRegistryError;

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
      'autoRegister': _autoRegister,
    };
    // Per-call overrides win over per-instance defaults; an empty
    // per-call list defers to the constructor's allowCredentialIds.
    final perCallAllow = request.allowCredentialIds;
    List<Uint8List> effectiveAllow = perCallAllow.isNotEmpty
        ? perCallAllow.map(Uint8List.fromList).toList()
        : (_allowCredentialIds ?? const []);
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

  /// Register a new passkey with PRF support. Per-call overrides on
  /// `request` (excludeCredentialIds, userName, userDisplayName) fall
  /// back to the constructor values when omitted.
  ///
  /// `user.id` is never host-supplied: the native plugin mints a fresh
  /// random 16-byte handle per call and surfaces it via
  /// [RegisteredCredential.userId]. Throws [PasskeyPrfException] on
  /// failure.
  Future<RegisteredCredential> createPasskey(CreatePasskeyRequest request) async {
    final args = <String, Object?>{
      'rpId': _rpId,
      'rpName': _rpName,
      'userName': request.userName ?? _userName,
      'userDisplayName': request.userDisplayName ?? _userDisplayName,
    };
    var excludeIds = List<Uint8List>.from(request.excludeCredentialIds);
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
