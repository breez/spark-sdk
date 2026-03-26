import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:crypto/crypto.dart';

import 'cli.dart';

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
  final String? rpid;

  PasskeyConfig({
    required this.provider,
    this.label,
    this.listLabels = false,
    this.storeLabel = false,
    this.rpid,
  });
}

/// Resolve the seed using a passkey PRF provider.
///
/// Mirrors the Rust CLI's `resolve_passkey_seed` function.
Future<Seed> resolvePasskeySeed(PasskeyConfig config, String dataDir, String? breezApiKey) async {
  if (config.provider != 'file') {
    throw Exception(
      'Passkey provider "${config.provider}" is not yet supported in the Flutter CLI. '
      'Only the "file" provider is currently available.',
    );
  }

  final filePrf = FilePrfProvider.create(dataDir);

  final relayConfig = NostrRelayConfig(breezApiKey: breezApiKey);
  final passkey = Passkey(
    derivePrfSeed: filePrf.derivePrfSeed,
    isPrfAvailable: filePrf.isPrfAvailable,
    relayConfig: relayConfig,
  );

  // --store-label: publish the label to Nostr
  if (config.storeLabel && config.label != null) {
    print("Publishing label '${config.label}' to Nostr...");
    await passkey.storeLabel(label: config.label!);
    print("Label '${config.label}' published successfully.");
  }

  // --list-labels: query Nostr and prompt user to select
  String? label;
  if (config.listLabels) {
    print('Querying Nostr for available labels...');
    final labels = await passkey.listLabels();

    if (labels.isEmpty) {
      throw Exception('No labels found on Nostr for this identity');
    }

    print('Available labels:');
    for (var i = 0; i < labels.length; i++) {
      print('  ${i + 1}: ${labels[i]}');
    }

    final input = prompt('Select label (1-${labels.length}): ');
    final idx = int.parse(input);
    if (idx < 1 || idx > labels.length) {
      throw Exception('Selection out of range');
    }
    label = labels[idx - 1];
  } else {
    label = config.label;
  }

  final wallet = await passkey.getWallet(label: label);
  return wallet.seed;
}
