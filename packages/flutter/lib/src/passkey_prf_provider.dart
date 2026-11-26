import 'dart:convert';

import 'package:flutter/services.dart';

import 'generated/breez_sdk_spark.dart';

/// Built-in passkey PRF provider using the platform's passkey APIs (iOS
/// AuthenticationServices, Android Credential Manager). Pass it to
/// [PasskeyClient] as its `prfProvider`.
class PasskeyProvider implements PrfProvider {
  /// Breez's shared `keys.breez.technology` RP. Pass as `rpId` to opt in
  /// (only valid for apps registered with Breez); apps with their own RP
  /// domain pass their own string.
  static const String breezRpId = 'keys.breez.technology';

  /// Default `rpName` when none is supplied.
  static const String defaultRpName = 'Breez';

  static const _channel = MethodChannel('breez_sdk_spark_passkey');

  final String _rpId;
  final String _rpName;
  final String _userName;
  final String _userDisplayName;

  /// Unset options use the Breez RP.
  PasskeyProvider([PasskeyProviderOptions? options])
    : _rpId = options?.rpId ?? breezRpId,
      _rpName = options?.rpName ?? defaultRpName,
      _userName = options?.userName ?? options?.rpName ?? defaultRpName,
      _userDisplayName = options?.userDisplayName ?? options?.userName ?? options?.rpName ?? defaultRpName;

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
    if (request.allowCredentials.isNotEmpty) {
      args['allowCredentials'] = request.allowCredentials.map(base64Encode).toList();
    }
    final preferImmediate = request.preferImmediatelyAvailableCredentials;
    if (preferImmediate != null) {
      args['preferImmediatelyAvailableCredentials'] = preferImmediate;
    }
    try {
      final result = await _channel.invokeMethod<Map<Object?, Object?>>('deriveSeeds', args);
      if (result == null) {
        throw GenericPrfProviderException('deriveSeeds returned null');
      }
      final rawSeeds = (result['seeds'] as List<Object?>?) ?? const <Object?>[];
      final credentialIdB64 = result['credentialId'] as String?;
      return DeriveSeedsOutput(
        seeds: rawSeeds.cast<String>().map(base64Decode).toList(),
        credentialId: credentialIdB64 == null ? null : base64Decode(credentialIdB64),
      );
    } on PlatformException catch (e) {
      throw _mapPlatformException(e);
    }
  }

  /// Register a new PRF-capable credential, evaluating PRF for [salts] in the
  /// same ceremony when the authenticator supports it.
  @override
  Future<CreatePasskeyOutput> createPasskey(List<Uint8List> excludeCredentials, List<String> salts) async {
    final args = <String, Object?>{
      'rpId': _rpId,
      'rpName': _rpName,
      'userName': _userName,
      'userDisplayName': _userDisplayName,
      'salts': salts,
    };
    if (excludeCredentials.isNotEmpty) {
      args['excludeCredentials'] = excludeCredentials.map(base64Encode).toList();
    }
    try {
      final result = await _channel.invokeMethod<Map<Object?, Object?>>('createPasskey', args);
      if (result == null) {
        throw GenericPrfProviderException('createPasskey returned null');
      }
      final userIdB64 = result['userId'] as String?;
      final aaguidB64 = result['aaguid'] as String?;
      final seedsB64 = (result['seeds'] as List<Object?>?)?.cast<String>();
      return CreatePasskeyOutput(
        credential: PasskeyCredential(
          credentialId: base64Decode(result['credentialId'] as String),
          userId: userIdB64 == null ? null : base64Decode(userIdB64),
          aaguid: aaguidB64 == null ? null : base64Decode(aaguidB64),
          backupEligible: result['backupEligible'] as bool?,
        ),
        seeds: seedsB64?.map(base64Decode).toList(),
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
  /// Returns [NotAssociatedDomainAssociation] only on iOS (Android degrades
  /// to [SkippedDomainAssociation]). The SDK never gates on the result;
  /// hosts pick their own policy.
  @override
  Future<DomainAssociation> checkDomainAssociation() async {
    try {
      final result = await _channel.invokeMethod<Map<Object?, Object?>>('checkDomainAssociation', {
        'rpId': _rpId,
      });
      final map = result ?? {};
      return switch (map['kind'] as String?) {
        'Associated' => AssociatedDomainAssociation(),
        'NotAssociated' => NotAssociatedDomainAssociation(
          source: (map['source'] as String?) ?? 'unknown',
          reason: (map['reason'] as String?) ?? '',
        ),
        _ => SkippedDomainAssociation((map['reason'] as String?) ?? ''),
      };
    } on PlatformException catch (e) {
      // Platform exceptions count as Skipped, so the caller has a single
      // contract to handle.
      return SkippedDomainAssociation(e.message ?? e.code);
    }
  }

  static PrfProviderException _mapPlatformException(PlatformException e) {
    final message = e.message ?? 'Unknown passkey error';
    return switch (e.code) {
      'ERR_USER_CANCELLED' => UserCancelledPrfProviderException(),
      'ERR_USER_TIMED_OUT' => UserTimedOutPrfProviderException(),
      'ERR_PRF_NOT_SUPPORTED' => PrfNotSupportedPrfProviderException(),
      'ERR_NO_CREDENTIAL' => CredentialNotFoundPrfProviderException(message),
      'ERR_CONFIGURATION' => ConfigurationPrfProviderException(message),
      'ERR_CREDENTIAL_ALREADY_EXISTS' => CredentialAlreadyExistsPrfProviderException(message),
      _ => GenericPrfProviderException(message),
    };
  }
}
