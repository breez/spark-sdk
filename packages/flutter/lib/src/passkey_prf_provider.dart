import 'dart:convert';

import 'package:flutter/services.dart';

import 'rust/models.dart'
    show DeriveSeedsOutput, DeriveSeedsRequest, PasskeyCredential, PasskeyProviderOptions;

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
  Future<PasskeyCredential> createPasskey(List<Uint8List> excludeCredentials);
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

  PasskeyProvider(PasskeyProviderOptions options)
    : _rpId = options.rpId ?? breezRpId,
      _rpName = options.rpName ?? defaultRpName,
      _userName = options.userName ?? options.rpName ?? defaultRpName,
      _userDisplayName = options.userDisplayName ?? options.userName ?? options.rpName ?? defaultRpName;

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
    final allowCredentials = request.allowCredentials.map(Uint8List.fromList).toList();
    if (allowCredentials.isNotEmpty) {
      args['allowCredentials'] = allowCredentials.map(base64Encode).toList();
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
      throw _mapPlatformException(e);
    }
  }

  /// Register a new passkey with PRF support. `excludeCredentials` blocks
  /// re-registering a device already holding a credential. The user handle
  /// is minted fresh per call (never host-supplied) and returned via
  /// [PasskeyCredential.userId]. Throws [PasskeyPrfException] on failure.
  @override
  Future<PasskeyCredential> createPasskey(List<Uint8List> excludeCredentials) async {
    final args = <String, Object?>{
      'rpId': _rpId,
      'rpName': _rpName,
      'userName': _userName,
      'userDisplayName': _userDisplayName,
    };
    if (excludeCredentials.isNotEmpty) {
      args['excludeCredentials'] = excludeCredentials.map(base64Encode).toList();
    }
    try {
      final result = await _channel.invokeMethod<Map<Object?, Object?>>('createPasskey', args);
      final map = result!;
      final credentialId = base64Decode(map['credentialId'] as String);
      final userId = base64Decode(map['userId'] as String);
      final aaguidB64 = map['aaguid'] as String?;
      final aaguid = aaguidB64 == null ? null : base64Decode(aaguidB64);
      final backupEligible = map['backupEligible'] as bool?;
      return PasskeyCredential(
        credentialId: credentialId,
        userId: userId,
        aaguid: aaguid,
        backupEligible: backupEligible,
      );
    } on PlatformException catch (e) {
      throw _mapPlatformException(e);
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
