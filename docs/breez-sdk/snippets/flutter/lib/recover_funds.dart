import 'dart:typed_data';

import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:convert/convert.dart';

Future<void> fetchRecoverableFunds(BreezSdk sdk) async {
  // ANCHOR: recoverable-funds
  GetInfoResponse info = await sdk.getInfo(request: GetInfoRequest(ensureSynced: false));

  if (info.recoverableFundsSats > BigInt.zero) {
    print("${info.recoverableFundsSats} sats can be recovered on-chain");
  }
  // ANCHOR_END: recoverable-funds
}

Future<PrepareRecoverFundsResponse> prepareRecovery(BreezSdk sdk) async {
  // ANCHOR: prepare-recover-funds
  PrepareRecoverFundsRequest request = PrepareRecoverFundsRequest(
    feeRateSatPerVbyte: BigInt.from(2),
    fundingKind: const CpfpFundingKind.p2Wpkh(),
    destination: "bc1q...your-destination-address",
    selection: const ExitLeafSelection.recoverableOnly(),
  );

  PrepareRecoverFundsResponse quote = await sdk.prepareRecoverFunds(request: request);

  if (quote.leaves.isEmpty) {
    print("Nothing to recover");
    return quote;
  }
  for (RecoverFundsLeaf leaf in quote.leaves) {
    print("${leaf.leafId}: ${leaf.valueSats} sats, ${leaf.method}");
  }
  print("Recovering ${quote.recoverableValueSats} sats"
      " for ${quote.totalFeeSats} sats in fees");
  RecoveryFunding? funding = quote.funding;
  if (funding != null) {
    print("Fund one UTXO of at least ${funding.singleUtxoSats} sats");
  }
  // ANCHOR_END: prepare-recover-funds

  return quote;
}

Future<void> recoverCooperatively(BreezSdk sdk, PrepareRecoverFundsResponse quote) async {
  // ANCHOR: recover-cooperatively
  // A quote with funding holds a unilateral exit: prepare the
  // cooperative leaves alone to recover them without it.
  if (quote.funding != null) {
    List<String> leafIds = quote.leaves
        .where((leaf) => leaf.method == RecoveryMethod.cooperative)
        .map((leaf) => leaf.leafId)
        .toList();
    if (leafIds.isEmpty) {
      return;
    }
    quote = await sdk.prepareRecoverFunds(
      request: PrepareRecoverFundsRequest(
        feeRateSatPerVbyte: quote.feeRateSatPerVbyte,
        fundingKind: null,
        destination: quote.destination,
        selection: ExitLeafSelection.specific(leafIds: leafIds),
      ),
    );
  }
  RecoverFundsResponse response = await sdk.recoverFunds(
    request: RecoverFundsRequest(prepared: quote, fundingInputs: []),
    signerSecretKey: null,
  );

  // Keep the whole response: checkRecoverFunds follows the recovery from it.
  for (RecoveryTransaction tx in response.transactions) {
    print("Broadcast ${tx.txid}: ${tx.txHex}");
  }
  for (CooperativeRecoveryFailure failure in response.failed) {
    print("Leaf ${failure.leafId} was not recovered: ${failure.error}");
  }
  // ANCHOR_END: recover-cooperatively
}

Future<RecoverFundsResponse> recoverWithFunding(
  BreezSdk sdk,
  PrepareRecoverFundsResponse quote,
) async {
  // ANCHOR: recover-funds
  List<int> secretKeyBytes = hex.decode("your-secret-key-hex");

  RecoverFundsResponse response = await sdk.recoverFunds(
    request: RecoverFundsRequest(
      prepared: quote,
      fundingInputs: [
        CpfpInput.p2Wpkh(
          txid: "your-utxo-txid",
          vout: 0,
          valueSats: BigInt.from(50000),
          pubkey: "your-compressed-pubkey-hex",
        ),
      ],
    ),
    signerSecretKey: Uint8List.fromList(secretKeyBytes),
  );

  // Keep the whole response: checkRecoverFunds follows the recovery from it.
  for (RecoveryTransaction tx in response.transactions) {
    if (tx.csvTimelockBlocks != null) {
      print("${tx.txid}: wait ${tx.csvTimelockBlocks} blocks after its parents confirm");
    }
  }
  // ANCHOR_END: recover-funds

  return response;
}

Future<void> checkRecovery(BreezSdk sdk, RecoverFundsResponse stored) async {
  // ANCHOR: check-recover-funds
  CheckRecoverFundsResponse checked = await sdk.checkRecoverFunds(
    request: CheckRecoverFundsRequest(recovery: stored),
  );

  // Store this one in place of the one you had.
  RecoverFundsResponse recovery = checked.recovery;

  RecoveryVerdict verdict = checked.verdict;
  if (verdict is RecoveryVerdict_Valid) {
    for (RecoveryTransaction tx in recovery.transactions) {
      if (tx.status is ExitTransactionStatus_Ready) {
        print("ready to broadcast: ${tx.txid}");
      }
    }
  } else if (verdict is RecoveryVerdict_Done) {
    print("Every transaction confirmed: the recovery is done");
  } else if (verdict is RecoveryVerdict_Redo) {
    // Prepare and build again, naming the same leaves. Pass
    // recovery.fundingInputs back and the SDK follows them to whatever they
    // have become.
    print("Build the recovery again: ${verdict.reason}");
  }
  // ANCHOR_END: check-recover-funds
}

Future<String> backUpExitState(BreezSdk sdk) async {
  // ANCHOR: export-exit-state
  ExportUnilateralExitStateResponse exported = await sdk.exportUnilateralExitState();

  // Keep the state somewhere the wallet's own storage cannot take with it.
  print("Exit state is ${exported.exitState.length} bytes");
  // ANCHOR_END: export-exit-state

  return exported.exitState;
}

Future<void> restoreExitState(BreezSdk sdk, String exitState) async {
  // ANCHOR: import-exit-state
  ImportUnilateralExitStateResponse imported = await sdk.importUnilateralExitState(
    request: ImportUnilateralExitStateRequest(exitState: exitState),
  );

  print("Imported ${imported.importedLeaves} leaves,"
      " skipped ${imported.skippedForeignLeaves}");
  // ANCHOR_END: import-exit-state
}

Future<void> collectExitData(BreezSdk sdk) async {
  // ANCHOR: sync-exit-data
  // With automatic collection off, an explicit sync is what collects the data
  // a unilateral exit needs, and it waits for the collection to finish. Needs
  // the Spark operators reachable, so run it on a schedule rather than at the
  // moment an exit is needed.
  await sdk.syncWallet(request: SyncWalletRequest());
  // ANCHOR_END: sync-exit-data
}

// ANCHOR: custom-cpfp-signer
Future<RecoverFundsResponse> recoverWithFundingSigner(
  BreezSdk sdk,
  PrepareRecoverFundsResponse quote,
) async {
  RecoverFundsResponse response = await sdk.recoverFundsWithSigner(
    request: RecoverFundsRequest(
      prepared: quote,
      fundingInputs: [
        CpfpInput.p2Wpkh(
          txid: "your-utxo-txid",
          vout: 0,
          valueSats: BigInt.from(50000),
          pubkey: "your-compressed-pubkey-hex",
        ),
      ],
    ),
    signPsbt: (Uint8List psbtBytes) async {
      Uint8List signedPsbtBytes = await signWithFundingKeys(psbtBytes);
      return signedPsbtBytes;
    },
  );

  return response;
}

Future<Uint8List> signWithFundingKeys(Uint8List psbtBytes) async {
  return psbtBytes;
}
// ANCHOR_END: custom-cpfp-signer
