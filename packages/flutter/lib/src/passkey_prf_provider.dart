import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/services.dart';

/// Options for constructing a [PasskeyPrfProvider].
class PasskeyPrfProviderOptions {
  /// Relying Party ID. Must match the domain configured for cross-platform
  /// credential sharing.
  ///
  /// Changing this after users have registered passkeys will make their existing
  /// credentials undiscoverable — they would need to create new passkeys.
  ///
  /// Defaults to `'keys.breez.technology'`.
  final String rpId;

  /// RP display name shown during credential registration. Only used when
  /// creating new passkeys; changing it does not affect existing credentials.
  ///
  /// Defaults to `'Breez SDK'`.
  final String rpName;

  /// User name stored with the credential, shown as a secondary label in some
  /// passkey managers. Defaults to [rpName]. Only used during registration;
  /// changing it does not affect existing credentials.
  final String? userName;

  /// User display name shown as the primary label in the passkey picker.
  /// Defaults to [userName]. Only used during registration; changing it does
  /// not affect existing credentials.
  final String? userDisplayName;

  const PasskeyPrfProviderOptions({
    this.rpId = 'keys.breez.technology',
    this.rpName = 'Breez SDK',
    this.userName,
    this.userDisplayName,
  });
}

/// Error thrown by [PasskeyPrfProvider] when a passkey operation fails.
///
/// Provides a structured [code] that maps to native error codes and a
/// human-readable [message]. The [code] can be used for programmatic
/// error handling without parsing message strings.
class PasskeyPrfException implements Exception {
  /// Machine-readable error code matching the native error codes.
  ///
  /// Known codes:
  /// - `userCancelled` — user dismissed the passkey prompt
  /// - `prfNotSupported` — authenticator doesn't support PRF
  /// - `noCredential` — no passkey found for this RP ID
  /// - `authenticationFailed` — passkey assertion failed
  /// - `registrationFailed` — passkey creation failed
  /// - `unknown` — unrecognized error
  final String code;

  /// Human-readable error description.
  final String message;

  const PasskeyPrfException({required this.code, required this.message});

  @override
  String toString() => 'PasskeyPrfException($code): $message';
}

/// Flutter passkey PRF provider using platform-native APIs.
///
/// Implements passkey PRF operations using:
/// - iOS: AuthenticationServices framework (iOS 18+)
/// - Android: Credential Manager API (Android 14+)
///
/// On first use, if no credential exists for the RP ID, a new passkey is
/// automatically created (registered), then the assertion is retried.
///
/// The [derivePrfSeed] and [isPrfAvailable] methods can be passed directly
/// to the [Passkey] constructor as callbacks.
///
/// ```dart
/// import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
///
/// final prfProvider = PasskeyPrfProvider();
/// final passkey = Passkey(
///   derivePrfSeed: prfProvider.derivePrfSeed,
///   isPrfAvailable: prfProvider.isPrfAvailable,
/// );
/// final wallet = await passkey.getWallet(label: 'personal');
/// ```
class PasskeyPrfProvider {
  static const _channel = MethodChannel('breez_sdk_spark_passkey');

  final String _rpId;
  final String _rpName;
  final String _userName;
  final String _userDisplayName;

  PasskeyPrfProvider([PasskeyPrfProviderOptions? options])
    : _rpId = options?.rpId ?? 'keys.breez.technology',
      _rpName = options?.rpName ?? 'Breez SDK',
      _userName = options?.userName ?? (options?.rpName ?? 'Breez SDK'),
      _userDisplayName = options?.userDisplayName ?? (options?.userName ?? (options?.rpName ?? 'Breez SDK'));

  /// Derive a 32-byte seed from passkey PRF with the given salt.
  ///
  /// Authenticates the user via a platform passkey and evaluates the PRF
  /// extension. If no credential exists for this RP ID, a new passkey is
  /// created automatically.
  ///
  /// Throws [PasskeyPrfException] with a structured error code on failure.
  Future<Uint8List> derivePrfSeed(String salt) async {
    try {
      final result = await _channel.invokeMethod<String>('derivePrfSeed', {
        'salt': salt,
        'rpId': _rpId,
        'rpName': _rpName,
        'userName': _userName,
        'userDisplayName': _userDisplayName,
      });
      return base64Decode(result!);
    } on PlatformException catch (e) {
      throw _mapPlatformException(e);
    }
  }

  /// Create a new passkey with PRF support.
  ///
  /// Only registers the credential — no seed derivation. Triggers exactly
  /// 1 platform prompt. Use this to separate credential creation from
  /// derivation in multi-step onboarding flows.
  ///
  /// [excludeCredentialIds] is an optional list of credential IDs to exclude.
  /// Pass previously created credential IDs to prevent the authenticator from
  /// creating a duplicate on the same device.
  ///
  /// Returns the credential ID of the newly created passkey.
  ///
  /// Throws [PasskeyPrfException] if the user cancels or PRF is not supported.
  Future<Uint8List> createPasskey({List<Uint8List>? excludeCredentialIds}) async {
    try {
      final result = await _channel.invokeMethod<String>('createPasskey', {
        'rpId': _rpId,
        'rpName': _rpName,
        'userName': _userName,
        'userDisplayName': _userDisplayName,
        if (excludeCredentialIds != null && excludeCredentialIds.isNotEmpty)
          'excludeCredentialIds': excludeCredentialIds
              .map((id) => base64Encode(id))
              .toList(),
      });
      return base64Decode(result!);
    } on PlatformException catch (e) {
      throw _mapPlatformException(e);
    }
  }

  /// Check if a PRF-capable passkey is available on this device.
  ///
  /// Returns `true` if the platform supports passkeys with PRF extension.
  Future<bool> isPrfAvailable() async {
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
      _ => PasskeyPrfException(code: 'unknown', message: message),
    };
  }
}
