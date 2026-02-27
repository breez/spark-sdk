import 'dart:async';
import 'dart:io';

import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

import 'commands.dart';
import 'helpers.dart';
import 'issuer.dart';
import 'persistence.dart';
import 'readline.dart';
import 'serialization.dart';

Future<void> runCli({
  required String dataDir,
  required String network,
  int? accountNumber,
}) async {
  await BreezSdkSparkLib.init(
    externalLibrary: ExternalLibrary.open(_nativeLibPath()),
  );

  final dir = Directory(dataDir);
  if (!dir.existsSync()) {
    dir.createSync(recursive: true);
  }

  final persistence = CliPersistence(dataDir);
  final mnemonic = persistence.getOrCreateMnemonic();

  final networkEnum = network == 'mainnet' ? Network.mainnet : Network.regtest;
  var config = defaultConfig(network: networkEnum);
  final apiKey = Platform.environment['BREEZ_API_KEY'];
  if (apiKey != null) {
    config = config.copyWith(apiKey: apiKey);
  }

  final seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: null);
  var builder = SdkBuilder(config: config, seed: seed);
  builder = builder.withDefaultStorage(storageDir: dataDir);

  if (accountNumber != null) {
    builder = builder.withKeySet(
      config: KeySetConfig(
        keySetType: KeySetType.default_,
        useAddressIndex: false,
        accountNumber: accountNumber,
      ),
    );
  }

  final sdk = await builder.build();

  // Listen for events in the background
  StreamSubscription<SdkEvent>? eventSub;
  try {
    final eventStream = sdk.addEventListener().asBroadcastStream();
    eventSub = eventStream.listen((event) {
      try {
        stderr.writeln('Event: ${serialize(event)}');
      } catch (_) {
        stderr.writeln('Event: $event');
      }
    });
  } catch (e) {
    stderr.writeln('Warning: could not subscribe to events: $e');
  }

  final tokenIssuer = sdk.getTokenIssuer();

  await _runRepl(sdk, tokenIssuer, networkEnum, persistence);

  eventSub?.cancel();
}

Future<void> _runRepl(
  BreezSdk sdk,
  TokenIssuer tokenIssuer,
  Network network,
  CliPersistence persistence,
) async {
  final networkLabel = network == Network.mainnet ? 'mainnet' : 'regtest';
  final promptStr = 'breez-spark-cli [$networkLabel]> ';

  stdout.writeln('Breez SDK CLI Interactive Mode');
  stdout.writeln("Type 'help' for available commands or 'exit' to quit");

  final registry = buildCommandRegistry();
  final allCommands = [
    ...commandNames,
    ...issuerCommandNames,
    'exit',
    'quit',
    'help',
  ];
  final rl = Readline(
    completions: allCommands,
    historyFile: persistence.historyFile,
  );

  while (true) {
    try {
      final line = rl.readLine(promptStr)?.trim();
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

      if (cmdName == 'issuer') {
        await dispatchIssuerCommand(cmdArgs, tokenIssuer);
      } else if (registry.containsKey(cmdName)) {
        final entry = registry[cmdName]!;
        await entry.handler(sdk, tokenIssuer, cmdArgs);
      } else {
        stdout.writeln(
          "Unknown command: $cmdName. Type 'help' for available commands.",
        );
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
    '  ${'issuer <subcommand>'.padRight(40)} Token issuer commands (use \'issuer help\' for details)',
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

/// Prompt user for input and return the trimmed response.
String prompt(String message, {String? defaultValue}) {
  if (defaultValue != null) {
    stdout.write('$message[$defaultValue] ');
  } else {
    stdout.write(message);
  }
  final line = stdin.readLineSync()?.trim() ?? '';
  if (line.isEmpty && defaultValue != null) return defaultValue;
  return line;
}

/// Resolve the path to the native library built by `make setup`.
String _nativeLibPath() {
  final ext = Platform.isMacOS ? 'dylib' : 'so';
  return '../../../../../../../packages/flutter/rust/target/release/'
      'libbreez_sdk_spark_flutter.$ext';
}
