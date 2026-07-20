import 'dart:typed_data';

import 'package:args/args.dart';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

import 'cli.dart';
import 'serialization.dart';

/// Advanced subcommand names (used for help and tab completion).
const advancedCommandNames = ['advanced unilateral-exit'];

typedef AdvancedHandler = Future<void> Function(BreezSdk sdk, List<String> args);

class _AdvancedEntry {
  final String description;
  final AdvancedHandler handler;
  const _AdvancedEntry(this.description, this.handler);
}

Map<String, _AdvancedEntry>? _registry;

Map<String, _AdvancedEntry> _getRegistry() {
  return _registry ??= {
    'unilateral-exit': _AdvancedEntry(
      'Build and sign a unilateral exit (expert-only)',
      _handleUnilateralExit,
    ),
  };
}

/// Dispatch an advanced subcommand given the args after 'advanced'.
Future<void> dispatchAdvancedCommand(List<String> args, BreezSdk sdk) async {
  final registry = _getRegistry();

  if (args.isEmpty || args[0] == 'help' || args[0] == '--help') {
    print('\nAdvanced subcommands (expert-only, misuse can strand or lose funds):\n');
    for (final entry in registry.entries.toList()..sort((a, b) => a.key.compareTo(b.key))) {
      print('  advanced ${entry.key.padRight(30)} ${entry.value.description}');
    }
    print('');
    return;
  }

  final subName = args[0];
  final subArgs = args.sublist(1);

  if (!registry.containsKey(subName)) {
    print("Unknown advanced subcommand: $subName. Use 'advanced help' for available commands.");
    return;
  }

  await registry[subName]!.handler(sdk, subArgs);
}

// --- unilateral-exit ---

CpfpFundingKind? _parseFundingKind(String s) {
  switch (s.toLowerCase()) {
    case 'p2wpkh':
      return const CpfpFundingKind.p2Wpkh();
    case 'p2tr':
      return const CpfpFundingKind.p2Tr();
    default:
      return null;
  }
}

/// Parse [args] with [parser], returning `null` if the user asked for help
/// or if parsing fails (prints usage + error in that case).
ArgResults? _parseArgs(ArgParser parser, List<String> args, String usage) {
  if (args.contains('help') || args.contains('--help') || args.contains('-h')) {
    print('Usage: $usage');
    print(parser.usage);
    return null;
  }
  try {
    return parser.parse(args);
  } on ArgParserException catch (e) {
    print('Usage: $usage');
    print(parser.usage);
    print('\nError: ${e.message}');
    return null;
  }
}

Future<void> _handleUnilateralExit(BreezSdk sdk, List<String> args) async {
  final parser =
      ArgParser(usageLineLength: 80)
        ..addOption('fee-rate', mandatory: true, help: 'Target fee rate in sat/vByte')
        ..addOption('funding-kind', defaultsTo: 'p2tr', help: 'Funding UTXO kind: p2wpkh or p2tr')
        ..addOption('destination', mandatory: true, help: 'Destination address for swept funds')
        ..addMultiOption('leaf', help: 'Leaf id to exit (repeatable). Omit to auto-select.');
  final results = _parseArgs(
    parser,
    args,
    'advanced unilateral-exit --fee-rate <rate> --destination <addr> [--funding-kind p2tr] [--leaf <id>...]',
  );
  if (results == null) return;

  final feeRate = BigInt.parse(results.option('fee-rate')!);
  final fundingKindStr = results.option('funding-kind')!;
  final fundingKind = _parseFundingKind(fundingKindStr);
  if (fundingKind == null) {
    print('Invalid funding kind: $fundingKindStr (expected p2wpkh or p2tr)');
    return;
  }
  final destination = results.option('destination')!;
  final leafIds = results.multiOption('leaf');

  final ExitLeafSelection selection =
      leafIds.isEmpty ? const ExitLeafSelection.auto() : ExitLeafSelection.specific(leafIds: leafIds);

  final prepared = await sdk.prepareUnilateralExit(
    request: PrepareUnilateralExitRequest(
      feeRateSatPerVbyte: feeRate,
      fundingKind: fundingKind,
      destination: destination,
      selection: selection,
    ),
  );
  printValue(prepared);

  if (prepared.leaves.isEmpty) {
    print('No leaves to exit.');
    return;
  }

  final utxoLine = prompt('Funding UTXO(s) as txid:vout:value:pubkey (space-separated, blank to stop): ');
  if (utxoLine.trim().isEmpty) {
    print('No funding provided; showing the quote only.');
    return;
  }

  final fundingInputs = <CpfpInput>[];
  for (final u in utxoLine.split(RegExp(r'\s+'))) {
    if (u.isEmpty) continue;
    final input = _parseCpfpInput(u, fundingKindStr);
    if (input == null) return;
    fundingInputs.add(input);
  }

  final keyLine = prompt('Hex secret key for the funding UTXO(s): ');
  final secretKeyBytes = _hexDecode(keyLine.trim());

  final response = await sdk.unilateralExit(
    request: UnilateralExitRequest(prepared: prepared, fundingInputs: fundingInputs),
    signerSecretKey: secretKeyBytes,
  );
  _printExitTransactions(response);
}

CpfpInput? _parseCpfpInput(String s, String kindStr) {
  final parts = s.split(':');
  if (parts.length != 4) {
    print("Invalid funding UTXO '$s', expected txid:vout:value:pubkey");
    return null;
  }
  final txid = parts[0];
  final vout = int.tryParse(parts[1]);
  final value = BigInt.tryParse(parts[2]);
  final pubkey = parts[3];
  if (vout == null || value == null) {
    print("Invalid funding UTXO '$s': could not parse vout or value");
    return null;
  }
  switch (kindStr.toLowerCase()) {
    case 'p2wpkh':
      return CpfpInput.p2Wpkh(txid: txid, vout: vout, value: value, pubkey: pubkey);
    case 'p2tr':
      return CpfpInput.p2Tr(txid: txid, vout: vout, value: value, pubkey: pubkey);
    default:
      print('Invalid funding kind: $kindStr');
      return null;
  }
}

void _printExitTransactions(UnilateralExitResponse response) {
  print(
    'Recoverable ${response.recoverableValueSat} sats, '
    'total fee ${response.totalFeeSat} sats, '
    '${response.transactions.length} transaction(s):',
  );
  for (var i = 0; i < response.transactions.length; i++) {
    final tx = response.transactions[i];
    final after = tx.dependsOn.isEmpty ? '' : ', after ${tx.dependsOn.join(",")}';
    final csv = tx.csvTimelockBlocks != null ? ', csv ${tx.csvTimelockBlocks} blocks' : '';
    print('  [$i] ${tx.kind} status=${tx.status} txid=${tx.txid}$after$csv');
    if (tx.status == ConfirmationStatus.confirmed) {
      print('      (already confirmed, nothing to broadcast)');
      continue;
    }
    final package = tx.cpfpTxHex != null ? '${tx.txHex},${tx.cpfpTxHex}' : tx.txHex;
    print('      Package: $package');
  }
}

Uint8List _hexDecode(String hexStr) {
  final bytes = Uint8List(hexStr.length ~/ 2);
  for (var i = 0; i < bytes.length; i++) {
    bytes[i] = int.parse(hexStr.substring(i * 2, i * 2 + 2), radix: 16);
  }
  return bytes;
}
