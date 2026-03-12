import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:crypto/crypto.dart';

const _secretFileName = 'seedless-restore-secret';

/// File-based implementation of passkey PRF provider.
///
/// Uses HMAC-SHA256 with a secret stored in a file. The secret is generated
/// randomly on first use and persisted to disk.
class FilePrfProvider {
  final Uint8List _secret;

  FilePrfProvider._(this._secret);

  static FilePrfProvider create(String dataDir) {
    final secretFile = File('$dataDir/$_secretFileName');

    Uint8List secret;
    if (secretFile.existsSync()) {
      final bytes = secretFile.readAsBytesSync();
      if (bytes.length != 32) {
        throw Exception('Invalid secret file: expected 32 bytes, got ${bytes.length}');
      }
      secret = Uint8List.fromList(bytes);
    } else {
      final dir = Directory(dataDir);
      if (!dir.existsSync()) {
        dir.createSync(recursive: true);
      }
      final random = Random.secure();
      secret = Uint8List(32);
      for (var i = 0; i < 32; i++) {
        secret[i] = random.nextInt(256);
      }
      secretFile.writeAsBytesSync(secret);
    }

    return FilePrfProvider._(secret);
  }

  Future<Uint8List> derivePrfSeed(String salt) async {
    final hmacSha256 = Hmac(sha256, _secret);
    final digest = hmacSha256.convert(salt.codeUnits);
    return Uint8List.fromList(digest.bytes);
  }

  Future<bool> isPrfAvailable() async => true;
}

/// Configuration for passkey seed derivation.
class PasskeyConfig {
  final String provider;
  final String? label;
  final bool listLabels;
  final bool storeLabel;

  PasskeyConfig({
    required this.provider,
    this.label,
    this.listLabels = false,
    this.storeLabel = false,
  });
}

/// Resolve the seed using a passkey PRF provider.
///
/// Mirrors the Rust CLI's `resolve_passkey_seed` function.
///
/// Note: Passkey/Nostr label operations are not yet available in the
/// Flutter SDK. This implementation derives a seed from the file-based PRF
/// provider using the Entropy seed variant.
Future<Seed> resolvePasskeySeed(PasskeyConfig config, String dataDir, String? breezApiKey) async {
  if (config.provider != 'file') {
    throw Exception(
      'Passkey provider "${config.provider}" is not yet supported in the Flutter CLI. '
      'Only the "file" provider is currently available.',
    );
  }

  final filePrf = FilePrfProvider.create(dataDir);

  // Passkey and NostrRelayConfig are not yet available in the Flutter SDK.
  // For now, derive a seed directly from the PRF provider.
  if (config.storeLabel && config.label != null) {
    print('Note: Label publishing to Nostr is not yet supported in Flutter');
  }

  if (config.listLabels) {
    print('Note: Label listing from Nostr is not yet supported in Flutter');
  }

  final label = config.label ?? 'Default';
  final seedBytes = await filePrf.derivePrfSeed(label);
  return Seed.entropy(seedBytes);
}
