import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:args/args.dart';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

import 'cli.dart';
import 'serialization.dart';

/// Advanced subcommand names (used for help and tab completion).
const advancedCommandNames = [
  'advanced unilateral-exit',
  'advanced check-unilateral-exit',
  'advanced export-unilateral-exit-state',
  'advanced import-unilateral-exit-state',
];

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
    'check-unilateral-exit': _AdvancedEntry(
      'Check a signed exit against the chain',
      _handleCheckUnilateralExit,
    ),
    'export-unilateral-exit-state': _AdvancedEntry(
      'Export the wallet\'s unilateral exit state to a file',
      _handleExportUnilateralExitState,
    ),
    'import-unilateral-exit-state': _AdvancedEntry(
      'Import a previously exported unilateral exit state',
      _handleImportUnilateralExitState,
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
        ..addMultiOption('leaf', help: 'Leaf id to exit (repeatable). Omit to auto-select.')
        ..addOption('output-file', help: 'File to write the signed exit to');
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
  final outputFile = results.option('output-file');
  if (outputFile != null) {
    _writeExit(outputFile, response);
  }
}

// --- export-unilateral-exit-state ---

Future<void> _handleExportUnilateralExitState(BreezSdk sdk, List<String> args) async {
  final parser = ArgParser(usageLineLength: 80)
    ..addOption('output-file', mandatory: true, help: 'File to write the exit state to');
  final results = _parseArgs(parser, args, 'advanced export-unilateral-exit-state --output-file <path>');
  if (results == null) return;

  final outputFile = results.option('output-file')!;
  final exported = await sdk.exportUnilateralExitState();
  File(outputFile).writeAsStringSync(exported.exitState);
  print('Wrote ${exported.exitState.length} bytes to $outputFile');
}

// --- import-unilateral-exit-state ---

Future<void> _handleImportUnilateralExitState(BreezSdk sdk, List<String> args) async {
  final parser = ArgParser(usageLineLength: 80)
    ..addOption('input-file', mandatory: true, help: 'File the exit state was exported to');
  final results = _parseArgs(parser, args, 'advanced import-unilateral-exit-state --input-file <path>');
  if (results == null) return;

  final inputFile = results.option('input-file')!;
  final exitState = File(inputFile).readAsStringSync();
  final imported = await sdk.importUnilateralExitState(
    request: ImportUnilateralExitStateRequest(exitState: exitState),
  );
  print(
    'Imported ${imported.importedLeaves} leaf(s), '
    'skipped ${imported.skippedForeignLeaves} leaf(s) from a different wallet '
    'and ${imported.skippedConflictingLeaves} that disagree with what this wallet holds, '
    'left out the exit data of ${imported.skippedChains} leaf(s)',
  );
}

// --- check-unilateral-exit ---

Future<void> _handleCheckUnilateralExit(BreezSdk sdk, List<String> args) async {
  final parser =
      ArgParser(usageLineLength: 80)
        ..addOption('input-file', mandatory: true, help: 'File the exit was written to')
        ..addOption('output-file', help: 'File to write the updated exit to. Defaults to --input-file.');
  final results = _parseArgs(
    parser,
    args,
    'advanced check-unilateral-exit --input-file <path> [--output-file <path>]',
  );
  if (results == null) return;

  final inputFile = results.option('input-file')!;
  final outputFile = results.option('output-file') ?? inputFile;

  final exit = _readExit(inputFile);
  final checked = await sdk.checkUnilateralExit(request: CheckUnilateralExitRequest(exit: exit));

  final verdict = checked.verdict;
  if (verdict is UnilateralExitVerdict_Redo) {
    print('Verdict: Redo { reason: "${verdict.reason}" }');
    print('  (this exit cannot be finished, quote and build it again)');
  } else if (verdict is UnilateralExitVerdict_Done) {
    print('Verdict: Done');
  } else {
    print('Verdict: Valid');
  }
  _printExitTransactions(checked.exit);
  _writeExit(outputFile, checked.exit);
}

// --- exit file I/O ---

void _writeExit(String path, UnilateralExitResponse exit) {
  File(path).writeAsStringSync(const JsonEncoder.withIndent('  ').convert(_exitToJson(exit)));
  print('Wrote the exit to $path');
}

UnilateralExitResponse _readExit(String path) {
  return _exitFromJson(jsonDecode(File(path).readAsStringSync()) as Map<String, dynamic>);
}

Map<String, dynamic> _exitToJson(UnilateralExitResponse r) => {
  'recoverableValueSat': r.recoverableValueSat.toString(),
  'totalFeeSat': r.totalFeeSat.toString(),
  'cpfpFeeSat': r.cpfpFeeSat.toString(),
  'fanoutFeeSat': r.fanoutFeeSat.toString(),
  'sweepFeeSat': r.sweepFeeSat.toString(),
  'leaves': [
    for (final l in r.leaves) {'leafId': l.leafId, 'value': l.value.toString()},
  ],
  'transactions': [for (final t in r.transactions) _txToJson(t)],
  'fundingInputs': [for (final i in r.fundingInputs) _cpfpInputToJson(i)],
};

UnilateralExitResponse _exitFromJson(Map<String, dynamic> j) {
  return UnilateralExitResponse(
    recoverableValueSat: BigInt.parse(j['recoverableValueSat'] as String),
    totalFeeSat: BigInt.parse(j['totalFeeSat'] as String),
    cpfpFeeSat: BigInt.parse(j['cpfpFeeSat'] as String),
    fanoutFeeSat: BigInt.parse(j['fanoutFeeSat'] as String),
    sweepFeeSat: BigInt.parse(j['sweepFeeSat'] as String),
    leaves:
        (j['leaves'] as List).map((e) {
          final l = e as Map<String, dynamic>;
          return UnilateralExitLeaf(leafId: l['leafId'] as String, value: BigInt.parse(l['value'] as String));
        }).toList(),
    transactions: (j['transactions'] as List).map((e) => _txFromJson(e as Map<String, dynamic>)).toList(),
    fundingInputs:
        (j['fundingInputs'] as List).map((e) => _cpfpInputFromJson(e as Map<String, dynamic>)).toList(),
  );
}

Map<String, dynamic> _txToJson(UnilateralExitTransaction tx) => {
  'kind': tx.kind.name,
  'nodeId': tx.nodeId,
  'txid': tx.txid,
  'txHex': tx.txHex,
  'cpfpTxHex': tx.cpfpTxHex,
  'csvTimelockBlocks': tx.csvTimelockBlocks,
  'dependsOn': tx.dependsOn,
  'status': _statusToJson(tx.status),
};

UnilateralExitTransaction _txFromJson(Map<String, dynamic> j) => UnilateralExitTransaction(
  kind: UnilateralExitTxKind.values.byName(j['kind'] as String),
  nodeId: j['nodeId'] as String?,
  txid: j['txid'] as String,
  txHex: j['txHex'] as String,
  cpfpTxHex: j['cpfpTxHex'] as String?,
  csvTimelockBlocks: j['csvTimelockBlocks'] as int?,
  dependsOn: (j['dependsOn'] as List).cast<String>(),
  status: _statusFromJson(j['status'] as Map<String, dynamic>),
);

Map<String, dynamic> _statusToJson(ExitTransactionStatus s) {
  if (s is ExitTransactionStatus_Confirmed) {
    return {'type': 'Confirmed', 'blockHeight': s.blockHeight};
  } else if (s is ExitTransactionStatus_WaitingForTimelock) {
    return {'type': 'WaitingForTimelock', 'spendableAtHeight': s.spendableAtHeight};
  } else if (s is ExitTransactionStatus_WaitingForDependencies) {
    return {'type': 'WaitingForDependencies'};
  } else if (s is ExitTransactionStatus_Ready) {
    return {'type': 'Ready'};
  } else {
    return {'type': 'Unverified'};
  }
}

ExitTransactionStatus _statusFromJson(Map<String, dynamic> j) {
  switch (j['type'] as String) {
    case 'Confirmed':
      return ExitTransactionStatus.confirmed(blockHeight: j['blockHeight'] as int?);
    case 'WaitingForTimelock':
      return ExitTransactionStatus.waitingForTimelock(spendableAtHeight: j['spendableAtHeight'] as int?);
    case 'WaitingForDependencies':
      return const ExitTransactionStatus.waitingForDependencies();
    case 'Ready':
      return const ExitTransactionStatus.ready();
    default:
      return const ExitTransactionStatus.unverified();
  }
}

Map<String, dynamic> _cpfpInputToJson(CpfpInput input) {
  if (input is CpfpInput_P2tr) {
    return {
      'type': 'P2tr',
      'txid': input.txid,
      'vout': input.vout,
      'value': input.value.toString(),
      'pubkey': input.pubkey,
    };
  } else if (input is CpfpInput_P2wpkh) {
    return {
      'type': 'P2wpkh',
      'txid': input.txid,
      'vout': input.vout,
      'value': input.value.toString(),
      'pubkey': input.pubkey,
    };
  } else if (input is CpfpInput_Custom) {
    return {
      'type': 'Custom',
      'txid': input.txid,
      'vout': input.vout,
      'value': input.value.toString(),
      'scriptPubkeyHex': input.scriptPubkeyHex,
      'signedInputWeight': input.signedInputWeight.toString(),
    };
  }
  throw StateError('Unknown CpfpInput variant: ${input.runtimeType}');
}

CpfpInput _cpfpInputFromJson(Map<String, dynamic> j) {
  final value = BigInt.parse(j['value'] as String);
  switch (j['type'] as String) {
    case 'P2tr':
      return CpfpInput.p2Tr(
        txid: j['txid'] as String,
        vout: j['vout'] as int,
        value: value,
        pubkey: j['pubkey'] as String,
      );
    case 'P2wpkh':
      return CpfpInput.p2Wpkh(
        txid: j['txid'] as String,
        vout: j['vout'] as int,
        value: value,
        pubkey: j['pubkey'] as String,
      );
    case 'Custom':
      return CpfpInput.custom(
        txid: j['txid'] as String,
        vout: j['vout'] as int,
        value: value,
        scriptPubkeyHex: j['scriptPubkeyHex'] as String,
        signedInputWeight: BigInt.parse(j['signedInputWeight'] as String),
      );
    default:
      throw StateError("Unknown CpfpInput type: ${j['type']}");
  }
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
    'total fee ${response.totalFeeSat} sats '
    '(cpfp ${response.cpfpFeeSat}, fanout ${response.fanoutFeeSat}, '
    'sweep ${response.sweepFeeSat}), '
    '${response.transactions.length} transaction(s):',
  );
  for (var i = 0; i < response.transactions.length; i++) {
    final tx = response.transactions[i];
    final after = tx.dependsOn.isEmpty ? '' : ', after ${tx.dependsOn.join(",")}';
    final csv = tx.csvTimelockBlocks != null ? ', csv ${tx.csvTimelockBlocks} blocks' : '';
    print('  [$i] ${tx.kind} status=${tx.status} txid=${tx.txid}$after$csv');
    final status = tx.status;
    if (status is ExitTransactionStatus_Confirmed) {
      final height = status.blockHeight;
      if (height != null) {
        print('      (confirmed in block $height, nothing to broadcast)');
      } else {
        print('      (already confirmed, nothing to broadcast)');
      }
      continue;
    } else if (status is ExitTransactionStatus_WaitingForDependencies) {
      print('      (waiting on the transactions it depends on)');
    } else if (status is ExitTransactionStatus_WaitingForTimelock) {
      final height = status.spendableAtHeight;
      if (height != null) {
        print('      (waiting for its timelock, until block $height)');
      } else {
        print('      (waiting for its timelock)');
      }
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
