import 'dart:convert';
import 'dart:io';

import 'package:args/args.dart';
import 'package:breez_cli/cli.dart';
import 'package:breez_cli/passkey.dart';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> main(List<String> arguments) async {
  final parser =
      ArgParser()
        ..addOption('data-dir', abbr: 'd', defaultsTo: './.data', help: 'Path to the data directory')
        ..addOption(
          'network',
          defaultsTo: 'regtest',
          allowed: ['regtest', 'signet', 'mainnet'],
          help: 'Network to use',
        )
        ..addOption(
          'spark-config',
          help: 'JSON file with Spark operators and SSP configuration',
          valueHelp: 'FILE',
        )
        ..addOption('chain-api-url', help: 'Chain API base URL (required for signet)', valueHelp: 'URL')
        ..addOption(
          'chain-api-type',
          help: 'Chain API type: esplora (default) or mempool-space',
          allowed: ['esplora', 'mempool-space'],
        )
        ..addOption('account-number', help: 'Account number for the Spark signer')
        ..addOption(
          'postgres-connection-string',
          help: 'PostgreSQL connection string (uses SQLite by default)',
        )
        ..addOption('mysql-connection-string', help: 'MySQL connection string (uses SQLite by default)')
        ..addMultiOption(
          'stable-balance-token',
          help: 'Stable balance token in TICKER:token_identifier format (repeatable)',
        )
        ..addOption('stable-balance-default-active-label', help: 'Default active label for stable balance')
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
        ..addFlag(
          'server-mode',
          negatable: false,
          help: 'Run in server mode (background_tasks_enabled=false)',
        )
        ..addOption('lnurl-domain', help: 'LNURL server domain for lightning address registration')
        ..addOption(
          'proxy',
          help: 'Route every connection through a SOCKS5 proxy, as HOST:PORT',
          valueHelp: 'HOST:PORT',
        )
        ..addOption('proxy-user', help: 'Username for SOCKS5 authentication (requires --proxy-password)')
        ..addOption('proxy-password', help: 'Password for SOCKS5 authentication (requires --proxy-user)')
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
  final mysqlConnectionString = results.option('mysql-connection-string');

  if (postgresConnectionString != null && mysqlConnectionString != null) {
    stderr.writeln(
      'Error: --postgres-connection-string and --mysql-connection-string are mutually exclusive',
    );
    exit(1);
  }

  final stableBalanceTokenStrings = results.multiOption('stable-balance-token');
  final stableBalanceTokens = <StableBalanceToken>[];
  for (final s in stableBalanceTokenStrings) {
    final colonIdx = s.indexOf(':');
    if (colonIdx < 0) {
      stderr.writeln("Invalid token format '$s', expected LABEL:token_identifier");
      exit(1);
    }
    final label = s.substring(0, colonIdx);
    final tokenIdentifier = s.substring(colonIdx + 1);
    stableBalanceTokens.add(StableBalanceToken(label: label, tokenIdentifier: tokenIdentifier));
  }
  final stableBalanceDefaultActiveLabel = results.option('stable-balance-default-active-label');
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

  // Validate proxy-related flag constraints
  final proxyAddress = results.option('proxy');
  final proxyUser = results.option('proxy-user');
  final proxyPassword = results.option('proxy-password');
  if (proxyUser != null && proxyAddress == null) {
    stderr.writeln('Error: --proxy-user requires --proxy');
    exit(1);
  }
  if (proxyPassword != null && proxyAddress == null) {
    stderr.writeln('Error: --proxy-password requires --proxy');
    exit(1);
  }
  if ((proxyUser != null) != (proxyPassword != null)) {
    stderr.writeln('Error: --proxy-user and --proxy-password must be specified together');
    exit(1);
  }

  ProxyConfig? proxy;
  if (proxyAddress != null) {
    proxy = parseProxy(proxyAddress, proxyUser, proxyPassword);
  }

  // Validate chain-api-type requires chain-api-url
  final chainApiUrl = results.option('chain-api-url');
  final chainApiTypeStr = results.option('chain-api-type');
  if (chainApiTypeStr != null && chainApiUrl == null) {
    stderr.writeln('Error: --chain-api-type requires --chain-api-url');
    exit(1);
  }
  final ChainApiType? chainApiType;
  if (chainApiTypeStr == 'mempool-space') {
    chainApiType = ChainApiType.mempoolSpace;
  } else if (chainApiTypeStr == 'esplora') {
    chainApiType = ChainApiType.esplora;
  } else {
    chainApiType = null;
  }

  // Load SparkConfig from JSON file if specified
  final sparkConfigPath = results.option('spark-config');
  SparkConfig? sparkConfig;
  if (sparkConfigPath != null) {
    sparkConfig = _loadSparkConfig(sparkConfigPath);
  }

  CliPasskeyConfig? passkeyConfig;
  if (passkeyProvider != null) {
    passkeyConfig = CliPasskeyConfig(
      provider: passkeyProvider,
      label: label,
      listLabels: listLabels,
      storeLabel: storeLabel,
      rpid: results.option('rpid'),
    );
  }

  await runCli(
    dataDir: dataDir,
    network: network,
    sparkConfig: sparkConfig,
    chainApiUrl: chainApiUrl,
    chainApiType: chainApiType,
    accountNumber: accountNumber,
    postgresConnectionString: postgresConnectionString,
    mysqlConnectionString: mysqlConnectionString,
    stableBalanceTokens: stableBalanceTokens,
    stableBalanceDefaultActiveLabel: stableBalanceDefaultActiveLabel,
    stableBalanceThreshold: stableBalanceThreshold,
    passkeyConfig: passkeyConfig,
    serverMode: results.flag('server-mode'),
    lnurlDomain: results.option('lnurl-domain'),
    proxy: proxy,
  );

  // Force exit — the native FFI library may keep background threads alive
  // after sdk.disconnect(), preventing the Dart VM from exiting cleanly.
  exit(0);
}

SparkConfig _loadSparkConfig(String path) {
  final file = File(path);
  if (!file.existsSync()) {
    stderr.writeln('Error: Failed to open Spark config $path');
    exit(1);
  }
  try {
    final json = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
    final operators =
        (json['signing_operators'] as List).map((op) {
          final o = op as Map<String, dynamic>;
          return SparkSigningOperator(
            id: o['id'] as int,
            identifier: o['identifier'] as String,
            address: o['address'] as String,
            identityPublicKey: o['identity_public_key'] as String,
            caCertPem: o['ca_cert_pem'] as String?,
          );
        }).toList();
    final ssp = json['ssp_config'] as Map<String, dynamic>;
    return SparkConfig(
      coordinatorIdentifier: json['coordinator_identifier'] as String,
      threshold: json['threshold'] as int,
      signingOperators: operators,
      sspConfig: SparkSspConfig(
        baseUrl: ssp['base_url'] as String,
        identityPublicKey: ssp['identity_public_key'] as String,
        schemaEndpoint: ssp['schema_endpoint'] as String?,
      ),
      expectedWithdrawBondSats: BigInt.from(json['expected_withdraw_bond_sats'] as int),
      expectedWithdrawRelativeBlockLocktime: BigInt.from(
        json['expected_withdraw_relative_block_locktime'] as int,
      ),
      maxTokenTransactionInputs: json['max_token_transaction_inputs'] as int?,
    );
  } catch (e) {
    stderr.writeln('Error: Failed to parse Spark config $path: $e');
    exit(1);
  }
}
