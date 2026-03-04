import 'dart:io';

import 'package:args/args.dart';
import 'package:breez_cli/cli.dart';

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

  await runCli(
    dataDir: dataDir,
    network: network,
    accountNumber: accountNumber,
    postgresConnectionString: postgresConnectionString,
    stableBalanceTokenIdentifier: stableBalanceTokenIdentifier,
    stableBalanceThreshold: stableBalanceThreshold,
  );

  // Force exit — the native FFI library may keep background threads alive
  // after sdk.disconnect(), preventing the Dart VM from exiting cleanly.
  exit(0);
}
