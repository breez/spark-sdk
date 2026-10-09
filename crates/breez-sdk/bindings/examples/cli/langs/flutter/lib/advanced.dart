import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:args/args.dart';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

import 'cli.dart';
import 'serialization.dart';

/// Advanced subcommand names (used for help and tab completion).
const advancedCommandNames = [
  'advanced recover-funds',
  'advanced check-recover-funds',
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
    'recover-funds': _AdvancedEntry(
      'Recover the funds that left the balance, or with --all every leaf',
      _handleRecoverFunds,
    ),
    'check-recover-funds': _AdvancedEntry(
      'Read a recovery written by recover-funds back against the chain',
      _handleCheckRecoverFunds,
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

// --- argument parsing ---

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

// --- recover-funds ---

Future<void> _handleRecoverFunds(BreezSdk sdk, List<String> args) async {
  const usage =
      'advanced recover-funds --fee-rate <rate> --destination <addr> [--funding-kind p2tr] '
      '[--all | --leaf <id>...] [--output-file <path>]';
  final parser =
      ArgParser(usageLineLength: 80)
        ..addOption('fee-rate', mandatory: true, help: 'Target fee rate in sat/vByte')
        ..addOption('funding-kind', defaultsTo: 'p2tr', help: 'Funding UTXO kind: p2wpkh or p2tr')
        ..addOption('destination', mandatory: true, help: 'Destination address for the recovered funds')
        ..addFlag(
          'all',
          negatable: false,
          help:
              'Recover every leaf worth it, including the ones still in the balance. '
              'Only for when the operators are unreachable or refuse to serve the wallet.',
        )
        ..addMultiOption(
          'leaf',
          help: 'Leaf id to recover (repeatable). Omit to recover the leaves that left the balance.',
        )
        ..addOption(
          'output-file',
          help: 'File to write the signed recovery to, for check-recover-funds to read back',
        );
  final results = _parseArgs(parser, args, usage);
  if (results == null) return;

  final all = results.flag('all');
  final leafIds = results.multiOption('leaf');
  if (all && leafIds.isNotEmpty) {
    print('Usage: $usage');
    print(parser.usage);
    print('\nError: --all cannot be used with --leaf');
    return;
  }
  final fundingKindStr = results.option('funding-kind')!;
  final fundingKind = _parseFundingKind(fundingKindStr);
  if (fundingKind == null) {
    print('Invalid funding kind: $fundingKindStr (expected p2wpkh or p2tr)');
    return;
  }

  final request = PrepareRecoverFundsRequest(
    feeRateSatPerVbyte: BigInt.parse(results.option('fee-rate')!),
    fundingKind: fundingKind,
    destination: results.option('destination')!,
    selection: _recoverySelection(all, leafIds),
  );
  await _recoverFunds(sdk, request, fundingKindStr, results.option('output-file'));
}

Future<void> _recoverFunds(
  BreezSdk sdk,
  PrepareRecoverFundsRequest request,
  String fundingKindStr,
  String? outputFile,
) async {
  var prepared = await sdk.prepareRecoverFunds(request: request);
  if (prepared.leaves.isEmpty) {
    print(
      'Nothing to recover: each selected leaf is finished, not worth recovering at this fee rate, '
      'or its funds were not found.',
    );
    return;
  }
  _printQuote(prepared);
  if (outputFile == null) {
    print('Without --output-file the recovery is only printed: check-recover-funds cannot read it back.');
  }

  final fundingInputs = <CpfpInput>[];
  Uint8List? signerSecretKey;
  final singleUtxoSats = prepared.funding?.singleUtxoSats;
  if (singleUtxoSats != null) {
    final utxoLine = prompt(
      'Funding UTXO(s) of at least $singleUtxoSats sats, as txid:vout:value:pubkey '
      '(space-separated; for P2TR the internal key; blank to skip the unilateral exit): ',
    );
    if (utxoLine.trim().isEmpty) {
      final cooperative = [
        for (final leaf in prepared.leaves)
          if (leaf.method == RecoveryMethod.cooperative) leaf.leafId,
      ];
      if (cooperative.isEmpty) {
        print('Nothing to recover without funding.');
        return;
      }
      print('Recovering only the cooperative leaves:');
      prepared = await sdk.prepareRecoverFunds(
        request: PrepareRecoverFundsRequest(
          feeRateSatPerVbyte: request.feeRateSatPerVbyte,
          fundingKind: request.fundingKind,
          destination: request.destination,
          selection: ExitLeafSelection.specific(leafIds: cooperative),
        ),
      );
      _printQuote(prepared);
    } else {
      for (final u in utxoLine.split(RegExp(r'\s+'))) {
        if (u.isEmpty) continue;
        final input = _parseCpfpInput(u, fundingKindStr);
        if (input == null) return;
        fundingInputs.add(input);
      }
      final keyLine = prompt('Hex secret key for the funding UTXO(s): ');
      signerSecretKey = _hexDecode(keyLine.trim());
    }
  }

  final answer = prompt('Sign this recovery? (y/n): ', defaultValue: 'y');
  if (answer.toLowerCase() != 'y') return;

  final response = await sdk.recoverFunds(
    request: RecoverFundsRequest(prepared: prepared, fundingInputs: fundingInputs),
    signerSecretKey: signerSecretKey,
  );
  _printRecovery(response);
  if (outputFile != null) {
    _writeRecovery(outputFile, response);
    print(
      'Next: broadcast the Ready packages. After new blocks, run check-recover-funds '
      '--input-file $outputFile to see what is ready next.',
    );
  } else {
    print('Next: broadcast the Ready packages.');
  }
}

void _printQuote(PrepareRecoverFundsResponse prepared) {
  printValue(prepared);
  final cooperative = prepared.leaves.where((leaf) => leaf.method == RecoveryMethod.cooperative).length;
  print(
    '${prepared.leaves.length} leaf(s), $cooperative cooperative and '
    '${prepared.leaves.length - cooperative} unilateral: '
    'recovering ${prepared.recoverableValueSats} sats for ${prepared.totalFeeSats} sats in fees',
  );
}

ExitLeafSelection _recoverySelection(bool all, List<String> leafIds) {
  if (all) return const ExitLeafSelection.all();
  if (leafIds.isEmpty) return const ExitLeafSelection.recoverableOnly();
  return ExitLeafSelection.specific(leafIds: leafIds);
}

// --- check-recover-funds ---

Future<void> _handleCheckRecoverFunds(BreezSdk sdk, List<String> args) async {
  final parser =
      ArgParser(usageLineLength: 80)
        ..addOption('input-file', mandatory: true, help: 'File the recovery was written to')
        ..addOption('output-file', help: 'File to write the updated recovery to. Defaults to --input-file.');
  final results = _parseArgs(
    parser,
    args,
    'advanced check-recover-funds --input-file <path> [--output-file <path>]',
  );
  if (results == null) return;

  await _checkRecoverFunds(sdk, results.option('input-file')!, results.option('output-file'));
}

Future<void> _checkRecoverFunds(BreezSdk sdk, String inputFile, String? outputFile) async {
  final recovery = _readRecovery(inputFile);
  final checked = await sdk.checkRecoverFunds(request: CheckRecoverFundsRequest(recovery: recovery));

  final verdict = checked.verdict;
  if (verdict is RecoveryVerdict_Redo) {
    final reason = verdict.reason.name;
    print('Verdict: Redo { reason: ${reason[0].toUpperCase()}${reason.substring(1)} }');
    print('  (this recovery cannot finish: run ${_redoCommand(checked.recovery)})');
  } else if (verdict is RecoveryVerdict_Done) {
    print('Verdict: Done');
  } else {
    print('Verdict: Valid');
  }
  _printRecovery(checked.recovery);
  _writeRecovery(outputFile ?? inputFile, checked.recovery);
}

String _redoCommand(RecoverFundsResponse recovery) => [
  'recover-funds --fee-rate ${recovery.feeRateSatPerVbyte} --destination ${recovery.destination}',
  for (final leaf in recovery.leaves) '--leaf ${leaf.leafId}',
].join(' ');

// --- recovery file I/O ---

void _writeRecovery(String path, RecoverFundsResponse recovery) {
  final temporary = File('$path.tmp');
  temporary.writeAsStringSync(const JsonEncoder.withIndent('  ').convert(_recoveryToJson(recovery)));
  temporary.renameSync(path);
  print('Wrote the recovery to $path');
}

RecoverFundsResponse _readRecovery(String path) {
  return _recoveryFromJson(jsonDecode(File(path).readAsStringSync()) as Map<String, dynamic>);
}

Map<String, dynamic> _recoveryToJson(RecoverFundsResponse r) => {
  'recoverableValueSats': r.recoverableValueSats.toString(),
  'totalFeeSats': r.totalFeeSats.toString(),
  'cooperativeFeeSats': r.cooperativeFeeSats.toString(),
  'cpfpFeeSats': r.cpfpFeeSats.toString(),
  'fanoutFeeSats': r.fanoutFeeSats.toString(),
  'refundFeeSats': r.refundFeeSats.toString(),
  'sweepFeeSats': r.sweepFeeSats.toString(),
  'leaves': [
    for (final l in r.leaves)
      {'leafId': l.leafId, 'valueSats': l.valueSats.toString(), 'method': l.method.name},
  ],
  'failed': [for (final f in r.failed) _failureToJson(f)],
  'transactions': [for (final t in r.transactions) _recoveryTxToJson(t)],
  'fundingInputs': [for (final i in r.fundingInputs) _cpfpInputToJson(i)],
  'feeRateSatPerVbyte': r.feeRateSatPerVbyte.toString(),
  'destination': r.destination,
};

RecoverFundsResponse _recoveryFromJson(Map<String, dynamic> j) {
  return RecoverFundsResponse(
    recoverableValueSats: BigInt.parse(j['recoverableValueSats'] as String),
    totalFeeSats: BigInt.parse(j['totalFeeSats'] as String),
    cooperativeFeeSats: BigInt.parse(j['cooperativeFeeSats'] as String),
    cpfpFeeSats: BigInt.parse(j['cpfpFeeSats'] as String),
    fanoutFeeSats: BigInt.parse(j['fanoutFeeSats'] as String),
    refundFeeSats: BigInt.parse(j['refundFeeSats'] as String),
    sweepFeeSats: BigInt.parse(j['sweepFeeSats'] as String),
    leaves:
        (j['leaves'] as List).map((e) {
          final l = e as Map<String, dynamic>;
          return RecoverFundsLeaf(
            leafId: l['leafId'] as String,
            valueSats: BigInt.parse(l['valueSats'] as String),
            method: RecoveryMethod.values.byName(l['method'] as String),
          );
        }).toList(),
    failed: (j['failed'] as List).map((e) => _failureFromJson(e as Map<String, dynamic>)).toList(),
    transactions:
        (j['transactions'] as List).map((e) => _recoveryTxFromJson(e as Map<String, dynamic>)).toList(),
    fundingInputs:
        (j['fundingInputs'] as List).map((e) => _cpfpInputFromJson(e as Map<String, dynamic>)).toList(),
    feeRateSatPerVbyte: BigInt.parse(j['feeRateSatPerVbyte'] as String),
    destination: j['destination'] as String,
  );
}

Map<String, dynamic> _recoveryTxToJson(RecoveryTransaction tx) => {
  'kind': tx.kind.name,
  'nodeId': tx.nodeId,
  'txid': tx.txid,
  'txHex': tx.txHex,
  'cpfpTxHex': tx.cpfpTxHex,
  'csvTimelockBlocks': tx.csvTimelockBlocks,
  'dependsOn': tx.dependsOn,
  'status': _statusToJson(tx.status),
};

RecoveryTransaction _recoveryTxFromJson(Map<String, dynamic> j) => RecoveryTransaction(
  kind: RecoveryTxKind.values.byName(j['kind'] as String),
  nodeId: j['nodeId'] as String?,
  txid: j['txid'] as String,
  txHex: j['txHex'] as String,
  cpfpTxHex: j['cpfpTxHex'] as String?,
  csvTimelockBlocks: j['csvTimelockBlocks'] as int?,
  dependsOn: (j['dependsOn'] as List).cast<String>(),
  status: _statusFromJson(j['status'] as Map<String, dynamic>),
);

Map<String, dynamic> _failureToJson(CooperativeRecoveryFailure f) => {
  'leafId': f.leafId,
  'outputTxid': f.outputTxid,
  'outputVout': f.outputVout,
  'error': _cooperativeRecoveryErrorToJson(f.error),
};

CooperativeRecoveryFailure _failureFromJson(Map<String, dynamic> j) => CooperativeRecoveryFailure(
  leafId: j['leafId'] as String,
  outputTxid: j['outputTxid'] as String,
  outputVout: j['outputVout'] as int,
  error: _cooperativeRecoveryErrorFromJson(j['error'] as Map<String, dynamic>),
);

Map<String, dynamic> _cooperativeRecoveryErrorToJson(CooperativeRecoveryError error) {
  if (error is CooperativeRecoveryError_ReplacementFeeTooLow) {
    return {
      'type': 'ReplacementFeeTooLow',
      'requiredFeeSats': error.requiredFeeSats.toString(),
      'requiredFeeRateSatPerVbyte': error.requiredFeeRateSatPerVbyte.toString(),
    };
  } else if (error is CooperativeRecoveryError_OperatorsUnavailable) {
    return {'type': 'OperatorsUnavailable', 'message': error.message};
  } else if (error is CooperativeRecoveryError_Generic) {
    return {'type': 'Generic', 'message': error.message};
  }
  throw StateError('Unknown CooperativeRecoveryError variant: ${error.runtimeType}');
}

CooperativeRecoveryError _cooperativeRecoveryErrorFromJson(Map<String, dynamic> j) {
  switch (j['type'] as String) {
    case 'ReplacementFeeTooLow':
      return CooperativeRecoveryError.replacementFeeTooLow(
        requiredFeeSats: BigInt.parse(j['requiredFeeSats'] as String),
        requiredFeeRateSatPerVbyte: BigInt.parse(j['requiredFeeRateSatPerVbyte'] as String),
      );
    case 'OperatorsUnavailable':
      return CooperativeRecoveryError.operatorsUnavailable(message: j['message'] as String);
    case 'Generic':
      return CooperativeRecoveryError.generic(message: j['message'] as String);
    default:
      throw StateError("Unknown CooperativeRecoveryError type: ${j['type']}");
  }
}

String _cooperativeRecoveryErrorMessage(CooperativeRecoveryError error) {
  if (error is CooperativeRecoveryError_ReplacementFeeTooLow) {
    return 'A recovery of this output is already on the network: replacing it takes at least '
        '${error.requiredFeeSats} sats or ${error.requiredFeeRateSatPerVbyte} sats/vbyte';
  } else if (error is CooperativeRecoveryError_OperatorsUnavailable) {
    return 'Operators unavailable: ${error.message}';
  } else if (error is CooperativeRecoveryError_Generic) {
    return 'Generic error: ${error.message}';
  }
  throw StateError('Unknown CooperativeRecoveryError variant: ${error.runtimeType}');
}

void _printRecovery(RecoverFundsResponse response) {
  print(
    'Recoverable ${response.recoverableValueSats} sats, '
    'total fee ${response.totalFeeSats} sats '
    '(cooperative ${response.cooperativeFeeSats}, cpfp ${response.cpfpFeeSats}, '
    'fanout ${response.fanoutFeeSats}, sweep ${response.sweepFeeSats}), '
    '${response.transactions.length} transaction(s):',
  );
  for (var i = 0; i < response.transactions.length; i++) {
    final tx = response.transactions[i];
    final after = tx.dependsOn.isEmpty ? '' : ', after ${tx.dependsOn.join(",")}';
    final csv = tx.csvTimelockBlocks != null ? ', csv ${tx.csvTimelockBlocks} blocks' : '';
    final node = tx.nodeId != null ? ' node=${tx.nodeId}' : '';
    print('  [$i] ${tx.kind}$node status=${tx.status} txid=${tx.txid}$after$csv');
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
  if (response.failed.isNotEmpty) {
    print('Not recovered, ${response.failed.length} leaf(s):');
  }
  for (final failure in response.failed) {
    print(
      '  leaf ${failure.leafId} '
      '(output ${failure.outputTxid}:${failure.outputVout}): '
      '${_cooperativeRecoveryErrorMessage(failure.error)}',
    );
  }
}

// --- helpers ---

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
      'valueSats': input.valueSats.toString(),
      'pubkey': input.pubkey,
    };
  } else if (input is CpfpInput_P2wpkh) {
    return {
      'type': 'P2wpkh',
      'txid': input.txid,
      'vout': input.vout,
      'valueSats': input.valueSats.toString(),
      'pubkey': input.pubkey,
    };
  } else if (input is CpfpInput_Custom) {
    return {
      'type': 'Custom',
      'txid': input.txid,
      'vout': input.vout,
      'valueSats': input.valueSats.toString(),
      'scriptPubkeyHex': input.scriptPubkeyHex,
      'signedInputWeight': input.signedInputWeight.toString(),
    };
  }
  throw StateError('Unknown CpfpInput variant: ${input.runtimeType}');
}

CpfpInput _cpfpInputFromJson(Map<String, dynamic> j) {
  final valueSats = BigInt.parse(j['valueSats'] as String);
  switch (j['type'] as String) {
    case 'P2tr':
      return CpfpInput.p2Tr(
        txid: j['txid'] as String,
        vout: j['vout'] as int,
        valueSats: valueSats,
        pubkey: j['pubkey'] as String,
      );
    case 'P2wpkh':
      return CpfpInput.p2Wpkh(
        txid: j['txid'] as String,
        vout: j['vout'] as int,
        valueSats: valueSats,
        pubkey: j['pubkey'] as String,
      );
    case 'Custom':
      return CpfpInput.custom(
        txid: j['txid'] as String,
        vout: j['vout'] as int,
        valueSats: valueSats,
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
      return CpfpInput.p2Wpkh(txid: txid, vout: vout, valueSats: value, pubkey: pubkey);
    case 'p2tr':
      return CpfpInput.p2Tr(txid: txid, vout: vout, valueSats: value, pubkey: pubkey);
    default:
      print('Invalid funding kind: $kindStr');
      return null;
  }
}

Uint8List _hexDecode(String hexStr) {
  final bytes = Uint8List(hexStr.length ~/ 2);
  for (var i = 0; i < bytes.length; i++) {
    bytes[i] = int.parse(hexStr.substring(i * 2, i * 2 + 2), radix: 16);
  }
  return bytes;
}
