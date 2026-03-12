import 'dart:io';

import 'package:args/args.dart';
import 'package:breez_cli/cli.dart';
import 'package:breez_cli/passkey.dart';

Future<void> main(List<String> arguments) async {
  final parser =
      ArgParser()
        ..addOption('data-dir', abbr: 'd', defaultsTo: './.data', help: 'Path to the data directory')
        ..addOption('network', defaultsTo: 'regtest', allowed: ['regtest', 'mainnet'], help: 'Network to use')
        ..addOption('account-number', help: 'Account number for the Spark signer')
        ..addOption(
          'postgres-connection-string',
          help: 'PostgreSQL connection string (uses SQLite by default)',
        )
        ..addOption('stable-balance-token-identifier', help: 'Stable balance token identifier')
        ..addOption('stable-balance-threshold', help: 'Stable balance threshold in sats')
        ..addOption(
          'passkey',
          help: 'Use passkey with PRF provider (file, yubikey, or fido2)',
          valueHelp: 'PROVIDER',
        )
        ..addOption('label', help: 'Label for seed derivation (requires --passkey)')
        ..addFlag(
          'list-labels',
          negatable: false,
          help: 'List and select labels from Nostr (requires --passkey)',
        )
        ..addFlag(
          'store-label',
          negatable: false,
          help: 'Publish label to Nostr (requires --passkey and --label)',
        )
        ..addOption('rpid', help: 'Relying party ID for FIDO2 provider (requires --passkey)')
        ..addFlag('help', abbr: 'h', negatable: false, help: 'Show usage');

  final ArgResults results;
  try {
    results = parser.parse(arguments);
  } on FormatException catch (e) {
    stderr.writeln('Error: ${e.message}');
    stderr.writeln('Usage: dart run breez_cli [options]');
    stderr.writeln(parser.usage);
    exit(1);
  }

  if (results.flag('help')) {
    stdout.writeln('Breez SDK CLI (Dart)');
    stdout.writeln('');
    stdout.writeln('Usage: dart run breez_cli [options]');
    stdout.writeln(parser.usage);
    exit(0);
  }

  final dataDir = results.option('data-dir')!;
  final network = results.option('network')!;
  final accountNumberStr = results.option('account-number');
  final accountNumber = accountNumberStr != null ? int.parse(accountNumberStr) : null;
  final postgresConnectionString = results.option('postgres-connection-string');
  final stableBalanceTokenIdentifier = results.option('stable-balance-token-identifier');
  final stableBalanceThresholdStr = results.option('stable-balance-threshold');
  final stableBalanceThreshold =
      stableBalanceThresholdStr != null ? BigInt.parse(stableBalanceThresholdStr) : null;

  final passkeyProvider = results.option('passkey');
  final label = results.option('label');
  final listLabels = results.flag('list-labels');
  final storeLabel = results.flag('store-label');

  // Validate passkey-related flag constraints (mirroring Rust CLI's clap config)
  if (passkeyProvider == null) {
    if (label != null || listLabels || storeLabel || results.option('rpid') != null) {
      stderr.writeln(
        'Error: --label, --list-labels, --store-label, '
        'and --rpid require --passkey',
      );
      exit(1);
    }
  }
  if (storeLabel && label == null) {
    stderr.writeln('Error: --store-label requires --label');
    exit(1);
  }
  if (listLabels && (label != null || storeLabel)) {
    stderr.writeln('Error: --list-labels conflicts with --label and --store-label');
    exit(1);
  }

  PasskeyConfig? passkeyConfig;
  if (passkeyProvider != null) {
    passkeyConfig = PasskeyConfig(
      provider: passkeyProvider,
      label: label,
      listLabels: listLabels,
      storeLabel: storeLabel,
    );
  }

  await runCli(
    dataDir: dataDir,
    network: network,
    accountNumber: accountNumber,
    postgresConnectionString: postgresConnectionString,
    stableBalanceTokenIdentifier: stableBalanceTokenIdentifier,
    stableBalanceThreshold: stableBalanceThreshold,
    passkeyConfig: passkeyConfig,
  );

  // Force exit — the native FFI library may keep background threads alive
  // after sdk.disconnect(), preventing the Dart VM from exiting cleanly.
  exit(0);
}
