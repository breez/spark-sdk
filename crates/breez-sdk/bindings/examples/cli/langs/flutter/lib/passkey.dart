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
class FilePrfProvider implements PrfProvider {
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

  Uint8List _hmac(String salt) {
    final hmacSha256 = Hmac(sha256, _secret);
    final digest = hmacSha256.convert(salt.codeUnits);
    return Uint8List.fromList(digest.bytes);
  }

  @override
  Future<DeriveSeedsOutput> deriveSeeds(DeriveSeedsRequest request) async {
    final seeds = request.salts.map(_hmac).toList();
    return DeriveSeedsOutput(seeds: seeds, credentialId: null);
  }

  @override
  Future<bool> isSupported() async => true;

  @override
  Future<PasskeyCredential> createPasskey(List<Uint8List> excludeCredentials) async {
    throw Exception(
      'File-backed PRF provider does not implement create-credential; '
      'use sign-in by label instead.',
    );
  }
}

/// Configuration for passkey seed derivation.
class CliPasskeyConfig {
  final String provider;
  final String? label;
  final bool listLabels;
  final bool storeLabel;
  final String? rpid;

  CliPasskeyConfig({
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
Future<Seed> resolvePasskeySeed(CliPasskeyConfig config, String dataDir, String? breezApiKey) async {
  if (config.provider != 'file') {
    throw Exception(
      'Passkey provider "${config.provider}" is not yet supported in the Flutter CLI. '
      'Only the "file" provider is currently available.',
    );
  }

  final filePrf = FilePrfProvider.create(dataDir);
  final passkey = PasskeyClientBuilder(breezApiKey: breezApiKey).withPrfProvider(filePrf).build();

  // --list-labels: discovery sign-in (no cached label) returns the
  // published label set; prompt the user to pick one.
  String? label;
  if (config.listLabels) {
    print('Querying Nostr for available labels...');
    final discovery = await passkey.signIn(request: SignInRequest(label: null, allowCredentials: const []));
    final labels = discovery.labels;

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

  // --store-label: publish before signing in so a fresh client can
  // discover the label later.
  if (config.storeLabel && label != null) {
    print("Publishing label '$label' to Nostr...");
    await passkey.labels().store(label: label);
    print("Label '$label' published successfully.");
  }

  final response = await passkey.signIn(request: SignInRequest(label: label, allowCredentials: const []));
  return response.wallet.seed;
}
