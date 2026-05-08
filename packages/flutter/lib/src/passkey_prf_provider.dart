import 'dart:convert';

import 'package:flutter/services.dart';

import 'rust/models.dart' show CreatePasskeyRequest, RegisteredCredential;

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

  /// When `true`, [PasskeyProvider.deriveSeed] automatically creates a
  /// new passkey if none exists, then retries the assertion. When `false`
  /// (default), throws [PasskeyPrfException] with code `noCredential` and
  /// the caller drives registration via [PasskeyProvider.createPasskey].
  final bool autoRegister;

  const PasskeyProviderOptions({
    this.rpId = 'keys.breez.technology',
    this.rpName = 'Breez SDK',
    this.userName,
    this.userDisplayName,
    this.autoRegister = false,
  });
}

/// Error thrown by [PasskeyProvider] when a passkey operation fails.
/// Provides a structured [code] for programmatic handling.
class PasskeyPrfException implements Exception {
  /// Machine-readable error code:
  /// - `userCancelled`, `prfNotSupported`, `noCredential`, `configuration`,
  ///   `credentialAlreadyExists`, `unknown`.
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
///   deriveSeed: provider.deriveSeed,
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

  PasskeyProvider([PasskeyProviderOptions? options])
    : _rpId = options?.rpId ?? 'keys.breez.technology',
      _rpName = options?.rpName ?? 'Breez SDK',
      _userName = options?.userName ?? (options?.rpName ?? 'Breez SDK'),
      _userDisplayName =
          options?.userDisplayName ?? (options?.userName ?? (options?.rpName ?? 'Breez SDK')),
      _autoRegister = options?.autoRegister ?? false;

  /// Derive a 32-byte seed from passkey PRF with the given salt.
  /// Throws [PasskeyPrfException] on failure.
  Future<Uint8List> deriveSeed(String salt) async {
    try {
      final result = await _channel.invokeMethod<String>('derivePrfSeed', {
        'salt': salt,
        'rpId': _rpId,
        'rpName': _rpName,
        'userName': _userName,
        'userDisplayName': _userDisplayName,
        'autoRegister': _autoRegister,
      });
      return base64Decode(result!);
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
    final result = await _channel.invokeMethod<bool>('isPrfAvailable');
    return result ?? false;
  }

  /// Map native [PlatformException] error codes to [PasskeyPrfException].
  static PasskeyPrfException _mapPlatformException(PlatformException e) {
    final message = e.message ?? 'Unknown passkey error';
    return switch (e.code) {
      'ERR_USER_CANCELLED' => PasskeyPrfException(code: 'userCancelled', message: message),
      'ERR_PRF_NOT_SUPPORTED' => PasskeyPrfException(code: 'prfNotSupported', message: message),
      'ERR_NO_CREDENTIAL' => PasskeyPrfException(code: 'noCredential', message: message),
      'ERR_CONFIGURATION' => PasskeyPrfException(code: 'configuration', message: message),
      'ERR_CREDENTIAL_ALREADY_EXISTS' =>
        PasskeyPrfException(code: 'credentialAlreadyExists', message: message),
      _ => PasskeyPrfException(code: 'unknown', message: message),
    };
  }
}
