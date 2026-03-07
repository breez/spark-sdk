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
  final String? walletName;
  final bool listWalletNames;
  final bool storeWalletName;

  PasskeyConfig({
    required this.provider,
    this.walletName,
    this.listWalletNames = false,
    this.storeWalletName = false,
  });
}

/// Resolve the seed using a passkey PRF provider.
///
/// Mirrors the Rust CLI's `resolve_passkey_seed` function.
Future<Seed> resolvePasskeySeed(PasskeyConfig config, String dataDir, String? breezApiKey) async {
  if (config.provider != 'file') {
    // YubiKey and FIDO2 providers require hardware access not yet available in Dart.
    throw Exception(
      'Passkey provider "${config.provider}" is not yet supported in the Dart CLI. '
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

  // --store-wallet-name: publish the wallet name to Nostr
  if (config.storeWalletName && config.walletName != null) {
    print("Publishing wallet name '${config.walletName}' to Nostr...");
    await passkey.storeWalletName(walletName: config.walletName!);
    print("Wallet name '${config.walletName}' published successfully.");
  }

  // --list-wallet-names: query Nostr and prompt user to select
  String? walletName;
  if (config.listWalletNames) {
    print('Querying Nostr for available wallet names...');
    final walletNames = await passkey.listWalletNames();

    if (walletNames.isEmpty) {
      throw Exception('No wallet names found on Nostr for this identity');
    }

    print('Available wallet names:');
    for (var i = 0; i < walletNames.length; i++) {
      print('  ${i + 1}: ${walletNames[i]}');
    }

    stdout.write('Select wallet name (1-${walletNames.length}): ');
    final input = stdin.readLineSync()?.trim() ?? '';
    final idx = int.tryParse(input);
    if (idx == null || idx < 1 || idx > walletNames.length) {
      throw Exception('Invalid selection');
    }

    walletName = walletNames[idx - 1];
  } else {
    walletName = config.walletName;
  }

  final wallet = await passkey.getWallet(walletName: walletName);
  return wallet.seed;
}
