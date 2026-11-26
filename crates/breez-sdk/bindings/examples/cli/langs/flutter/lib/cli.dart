import 'dart:io';

import 'package:breez_sdk_spark_flutter/breez_sdk_spark_dart.dart';

import 'advanced.dart';
import 'bridging.dart';
import 'commands.dart';
import 'contacts.dart';
import 'issuer.dart';
import 'passkey.dart';
import 'persistence.dart';
import 'readline.dart';
import 'serialization.dart';
import 'stable_balance.dart';
import 'token_allowances.dart';
import 'webhooks.dart';

/// Parses `HOST:PORT` into a [ProxyConfig]. Splits from the right so an IPv6
/// literal keeps its colons.
ProxyConfig parseProxy(String address, String? username, String? password) {
  final lastColon = address.lastIndexOf(':');
  if (lastColon < 0) {
    throw FormatException("Invalid proxy '$address', expected HOST:PORT");
  }
  final host = address.substring(0, lastColon);
  final portStr = address.substring(lastColon + 1);
  final port = int.tryParse(portStr);
  if (port == null) {
    throw FormatException("Invalid proxy port '$portStr'");
  }
  return ProxyConfig(host: host, port: port, username: username, password: password);
}

Future<void> runCli({
  required String dataDir,
  required String network,
  SparkConfig? sparkConfig,
  String? chainApiUrl,
  ChainApiType? chainApiType,
  int? accountNumber,
  String? postgresConnectionString,
  String? mysqlConnectionString,
  List<StableBalanceToken> stableBalanceTokens = const [],
  String? stableBalanceDefaultActiveLabel,
  BigInt? stableBalanceThreshold,
  CliPasskeyConfig? passkeyConfig,
  bool serverMode = false,
  String? lnurlDomain,
  String? realTimeSyncServerUrl,
  ProxyConfig? proxy,
}) async {
  final dir = Directory(dataDir);
  if (!dir.existsSync()) {
    dir.createSync(recursive: true);
  }

  final persistence = CliPersistence(dataDir);

  final Network networkEnum;
  switch (network) {
    case 'mainnet':
      networkEnum = Network.mainnet;
    case 'signet':
      networkEnum = Network.signet;
    default:
      networkEnum = Network.regtest;
  }
  Config config;
  if (serverMode) {
    stdout.writeln('Server mode enabled. Run `sync` between operations.');
    config = defaultServerConfig(network: networkEnum);
  } else {
    config = defaultConfig(network: networkEnum);
  }
  final apiKey = Platform.environment['BREEZ_API_KEY'];
  if (apiKey != null) {
    config = config.copyWith(apiKey: apiKey);
  }

  if (sparkConfig != null) {
    config = config.copyWith(sparkConfig: sparkConfig);
  }

  if (stableBalanceTokens.isNotEmpty) {
    config = config.copyWith(
      stableBalanceConfig: StableBalanceConfig(
        tokens: stableBalanceTokens,
        defaultActiveLabel: stableBalanceDefaultActiveLabel,
        thresholdSats: stableBalanceThreshold,
        maxSlippageBps: null,
      ),
    );
  }

  if (lnurlDomain != null) {
    config = config.copyWith(lnurlDomain: lnurlDomain);
  }

  if (realTimeSyncServerUrl != null) {
    config = config.copyWith(realTimeSyncServerUrl: realTimeSyncServerUrl);
  }

  if (proxy != null) {
    config = config.copyWith(proxy: proxy);
  }

  if (networkEnum == Network.mainnet) {
    config = config.copyWith(crossChainConfig: CrossChainConfig());
  }

  Seed seed;
  if (passkeyConfig != null) {
    seed = await resolvePasskeySeed(passkeyConfig, dataDir, apiKey, proxy);
  } else {
    final mnemonic = persistence.getOrCreateMnemonic();
    seed = MnemonicSeed(mnemonic: mnemonic, passphrase: null);
  }

  final builder = SdkBuilder(config: config, seed: seed);

  if (chainApiUrl != null) {
    await builder.withRestChainService(
      url: chainApiUrl,
      apiType: chainApiType ?? ChainApiType.esplora,
      credentials: null,
    );
  }

  if (postgresConnectionString != null) {
    stderr.writeln(
      'Warning: --postgres-connection-string is not yet supported in the Dart CLI. '
      'Using default SQLite storage instead.',
    );
  } else if (mysqlConnectionString != null) {
    stderr.writeln(
      'Warning: --mysql-connection-string is not yet supported in the Dart CLI. '
      'Using default SQLite storage instead.',
    );
  }
  await builder.withDefaultStorage(storageDir: dataDir);

  if (accountNumber != null) {
    await builder.withAccountNumber(accountNumber: accountNumber);
  }

  final sdk = await builder.build();

  await sdk.addEventListener(listener: _CliEventListener());

  final tokenIssuer = sdk.getTokenIssuer();

  await _runRepl(sdk, tokenIssuer, networkEnum, persistence);
}

class _CliEventListener implements EventListener {
  @override
  Future<void> onEvent(SdkEvent event) async {
    try {
      stderr.writeln('Event: ${serializeCompact(event)}');
    } catch (_) {
      stderr.writeln('Event: $event');
    }
  }
}

Future<void> _runRepl(
  BreezSdk sdk,
  TokenIssuer tokenIssuer,
  Network network,
  CliPersistence persistence,
) async {
  final String networkLabel;
  switch (network) {
    case Network.mainnet:
      networkLabel = 'mainnet';
    case Network.signet:
      networkLabel = 'signet';
    default:
      networkLabel = 'regtest';
  }
  final promptStr = 'breez-spark-cli [$networkLabel]> ';

  stdout.writeln('Breez SDK CLI Interactive Mode');
  stdout.writeln("Type 'help' for available commands or 'exit' to quit");

  final registry = buildCommandRegistry();
  final allCommands = [
    ...commandNames,
    ...advancedCommandNames,
    ...bridgingCommandNames,
    ...issuerCommandNames,
    ...contactsCommandNames,
    ...webhookCommandNames,
    ...stableBalanceCommandNames,
    ...tokenAllowanceCommandNames,
    'exit',
    'quit',
    'help',
  ];
  final rl = Readline(completions: allCommands, historyFile: persistence.historyFile);

  while (true) {
    try {
      final line = (await rl.readLine(promptStr))?.trim();
      if (line == null) {
        // EOF (CTRL-D)
        stdout.writeln('CTRL-D');
        break;
      }
      if (line.isEmpty) continue;

      if (line == 'exit' || line == 'quit') break;

      if (line == 'help') {
        _printHelp(registry);
        continue;
      }

      final args = _splitArgs(line);
      if (args.isEmpty) continue;

      final cmdName = args[0];
      final cmdArgs = args.sublist(1);

      if (cmdName == 'advanced') {
        await dispatchAdvancedCommand(cmdArgs, sdk);
      } else if (cmdName == 'bridging') {
        await dispatchBridgingCommand(cmdArgs, sdk);
      } else if (cmdName == 'issuer') {
        await dispatchIssuerCommand(cmdArgs, tokenIssuer);
      } else if (cmdName == 'contacts') {
        await dispatchContactsCommand(cmdArgs, sdk);
      } else if (cmdName == 'webhooks') {
        await dispatchWebhookCommand(cmdArgs, sdk);
      } else if (cmdName == 'stable-balance') {
        await dispatchStableBalanceCommand(cmdArgs, sdk);
      } else if (cmdName == 'token-allowances') {
        await dispatchTokenAllowanceCommand(cmdArgs, sdk);
      } else if (registry.containsKey(cmdName)) {
        final entry = registry[cmdName]!;
        await entry.handler(sdk, tokenIssuer, cmdArgs);
      } else {
        stdout.writeln("Unknown command: $cmdName. Type 'help' for available commands.");
      }
    } on StdinException {
      stdout.writeln('');
      break;
    } catch (e) {
      stdout.writeln('Error: $e');
    }
  }

  rl.close();

  try {
    await sdk.disconnect();
  } catch (e) {
    stderr.writeln('Failed to gracefully stop SDK: $e');
  }

  stdout.writeln('Goodbye!');
}

void _printHelp(Map<String, CommandEntry> registry) {
  stdout.writeln('\nAvailable commands:\n');
  for (final name in registry.keys.toList()..sort()) {
    final desc = registry[name]!.description;
    stdout.writeln('  ${name.padRight(40)} $desc');
  }
  stdout.writeln(
    '  ${'advanced <subcommand>'.padRight(40)} Expert-only commands (use \'advanced help\' for details)',
  );
  stdout.writeln(
    '  ${'bridging <subcommand>'.padRight(40)} Bridging commands (use \'bridging help\' for details)',
  );
  stdout.writeln(
    '  ${'issuer <subcommand>'.padRight(40)} Token issuer commands (use \'issuer help\' for details)',
  );
  stdout.writeln(
    '  ${'contacts <subcommand>'.padRight(40)} Contacts commands (use \'contacts help\' for details)',
  );
  stdout.writeln(
    '  ${'webhooks <subcommand>'.padRight(40)} Webhook commands (use \'webhooks help\' for details)',
  );
  stdout.writeln(
    '  ${'stable-balance <subcommand>'.padRight(40)} Stable balance commands (use \'stable-balance help\' for details)',
  );
  stdout.writeln(
    '  ${'token-allowances <subcommand>'.padRight(40)} Token allowance commands (use \'token-allowances help\' for details)',
  );
  stdout.writeln('  ${'exit / quit'.padRight(40)} Exit the CLI');
  stdout.writeln('  ${'help'.padRight(40)} Show this help message');
  stdout.writeln('');
}

/// Split a command line into tokens, respecting quoted strings.
List<String> _splitArgs(String line) {
  final args = <String>[];
  final buf = StringBuffer();
  var inQuote = false;
  String? quoteChar;

  for (var i = 0; i < line.length; i++) {
    final c = line[i];
    if (inQuote) {
      if (c == quoteChar) {
        inQuote = false;
        quoteChar = null;
      } else {
        buf.write(c);
      }
    } else if (c == '"' || c == "'") {
      inQuote = true;
      quoteChar = c;
    } else if (c == ' ' || c == '\t') {
      if (buf.isNotEmpty) {
        args.add(buf.toString());
        buf.clear();
      }
    } else {
      buf.write(c);
    }
  }
  if (buf.isNotEmpty) {
    args.add(buf.toString());
  }
  return args;
}

/// Prompt user for input and return the trimmed response. Piped (non-TTY)
/// stdin gets no prompt text, so the output that follows starts its own line.
Future<String> prompt(String message, {String? defaultValue}) async {
  if (stdin.hasTerminal) {
    stdout.write(defaultValue != null ? '$message[$defaultValue] ' : message);
  }
  final line = (await input.readLine())?.trim() ?? '';
  if (line.isEmpty && defaultValue != null) return defaultValue;
  return line;
}
