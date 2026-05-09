import 'dart:convert';

import 'package:flutter/services.dart';

import 'rust/models.dart' show CreatePasskeyRequest, DeriveSeedsRequest, RegisteredCredential;

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

  const PasskeyProviderOptions({
    this.rpId = 'keys.breez.technology',
    this.rpName = 'Breez SDK',
    this.userName,
    this.userDisplayName,
    this.autoRegister = false,
    this.allowCredentialIds,
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

  PasskeyProvider([PasskeyProviderOptions? options])
    : _rpId = options?.rpId ?? 'keys.breez.technology',
      _rpName = options?.rpName ?? 'Breez SDK',
      _userName = options?.userName ?? (options?.rpName ?? 'Breez SDK'),
      _userDisplayName =
          options?.userDisplayName ?? (options?.userName ?? (options?.rpName ?? 'Breez SDK')),
      _autoRegister = options?.autoRegister ?? false,
      _allowCredentialIds = options?.allowCredentialIds;

  /// Derive multiple 32-byte seeds from passkey PRF with the given salts
  /// in as few OS ceremonies as the platform supports (dual-salt
  /// assertion where available). For the `salts.length == 1` case the
  /// native plugin short-circuits to a single-salt assertion (one
  /// prompt). Used by the SDK's `setup_wallet` orchestration to collapse
  /// master + label derivation into one prompt.
  Future<List<Uint8List>> deriveSeeds(DeriveSeedsRequest request) async {
    try {
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
      final List<Uint8List>? effectiveAllow = perCallAllow.isNotEmpty
          ? perCallAllow.map(Uint8List.fromList).toList()
          : _allowCredentialIds;
      if (effectiveAllow != null && effectiveAllow.isNotEmpty) {
        args['allowCredentialIds'] = effectiveAllow.map(base64Encode).toList();
      }
      final preferImmediate = request.preferImmediatelyAvailableCredentials;
      if (preferImmediate != null) {
        args['preferImmediatelyAvailableCredentials'] = preferImmediate;
      }
      final result = await _channel.invokeMethod<List<Object?>>('deriveSeeds', args);
      if (result == null) {
        throw const PasskeyPrfException(
          code: 'unknown',
          message: 'deriveSeeds returned null',
        );
      }
      return result.cast<String>().map((b64) => base64Decode(b64)).toList();
    } on PlatformException catch (e) {
      throw _mapPlatformException(e);
    }
  }

  /// Register a new passkey with PRF support. Per-call overrides on
  /// `request` (excludeCredentialIds, userId, userName, userDisplayName)
  /// fall back to the constructor values when omitted. Throws
  /// [PasskeyPrfException] on failure.
  Future<RegisteredCredential> createPasskey(CreatePasskeyRequest request) async {
    try {
      final args = <String, Object?>{
        'rpId': _rpId,
        'rpName': _rpName,
        'userName': request.userName ?? _userName,
        'userDisplayName': request.userDisplayName ?? _userDisplayName,
      };
      if (request.excludeCredentialIds.isNotEmpty) {
        args['excludeCredentialIds'] =
            request.excludeCredentialIds.map((id) => base64Encode(id)).toList();
      }
      if (request.userId != null) {
        args['userId'] = base64Encode(request.userId!);
      }
      final result = await _channel.invokeMethod<Map<Object?, Object?>>('createPasskey', args);
      final map = result!;
      final credentialId = base64Decode(map['credentialId'] as String);
      final aaguidB64 = map['aaguid'] as String?;
      final aaguid = aaguidB64 == null ? null : base64Decode(aaguidB64);
      final backupEligible = map['backupEligible'] as bool?;
      return RegisteredCredential(
        credentialId: credentialId,
        aaguid: aaguid,
        backupEligible: backupEligible,
      );
    } on PlatformException catch (e) {
      throw _mapPlatformException(e);
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
