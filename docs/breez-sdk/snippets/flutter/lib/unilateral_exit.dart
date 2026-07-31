import 'dart:typed_data';

import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:convert/convert.dart';

Future<PrepareUnilateralExitResponse> quoteExit(BreezSdk sdk) async {
  // ANCHOR: prepare-unilateral-exit
  PrepareUnilateralExitRequest request = PrepareUnilateralExitRequest(
    feeRateSatPerVbyte: BigInt.from(2),
    fundingKind: const CpfpFundingKind.p2Wpkh(),
    destination: "bc1q...your-destination-address",
    selection: const ExitLeafSelection.auto(),
  );

  PrepareUnilateralExitResponse quote = await sdk.prepareUnilateralExit(request: request);

  print("Recovering ${quote.recoverableValueSat} sats for ${quote.totalFeeSat} sats in fees");
  print("Fund a single UTXO of at least ${quote.singleUtxoFundingSat} sats");
  // ANCHOR_END: prepare-unilateral-exit
  return quote;
}

Future<void> buildExit(BreezSdk sdk, PrepareUnilateralExitResponse quote) async {
  // ANCHOR: unilateral-exit
  List<int> secretKeyBytes = hex.decode("your-secret-key-hex");

  UnilateralExitResponse response = await sdk.unilateralExit(
    request: UnilateralExitRequest(
      prepared: quote,
      fundingInputs: [
        CpfpInput.p2Wpkh(
          txid: "your-utxo-txid",
          vout: 0,
          value: BigInt.from(50000),
          pubkey: "your-compressed-pubkey-hex",
        ),
      ],
    ),
    signerSecretKey: Uint8List.fromList(secretKeyBytes),
  );

  for (UnilateralExitTransaction tx in response.transactions) {
    if (tx.csvTimelockBlocks != null) {
      print("${tx.txid}: wait ${tx.csvTimelockBlocks} blocks after its parents confirm");
    }
  }
  // ANCHOR_END: unilateral-exit
}

Future<String> exportExitState(BreezSdk sdk) async {
  // ANCHOR: export-unilateral-exit-state
  ExportUnilateralExitStateResponse exported = await sdk.exportUnilateralExitState();

  // Keep the state somewhere the wallet's own storage cannot take with it.
  print("Exit state is ${exported.exitState.length} bytes");
  // ANCHOR_END: export-unilateral-exit-state

  return exported.exitState;
}

Future<void> importExitState(BreezSdk sdk, String exitState) async {
  // ANCHOR: import-unilateral-exit-state
  ImportUnilateralExitStateResponse imported = await sdk.importUnilateralExitState(
    request: ImportUnilateralExitStateRequest(exitState: exitState),
  );

  print("Imported ${imported.importedLeaves} leaves, skipped ${imported.skippedLeaves}");
  // ANCHOR_END: import-unilateral-exit-state
}

// ANCHOR: custom-cpfp-signer
Future<void> buildExitWithSigner(BreezSdk sdk, PrepareUnilateralExitResponse quote) async {
  // Flutter cannot pass a foreign CpfpSigner, so it takes a signPsbt callback.
  UnilateralExitResponse response = await sdk.unilateralExitWithSigner(
    request: UnilateralExitRequest(
      prepared: quote,
      fundingInputs: [
        CpfpInput.p2Wpkh(
          txid: "your-utxo-txid",
          vout: 0,
          value: BigInt.from(50000),
          pubkey: "your-compressed-pubkey-hex",
        ),
      ],
    ),
    signPsbt: (Uint8List psbtBytes) async {
      return signPsbtWithYourKeys(psbtBytes);
    },
  );

  for (UnilateralExitTransaction tx in response.transactions) {
    if (tx.csvTimelockBlocks != null) {
      print("${tx.txid}: wait ${tx.csvTimelockBlocks} blocks after its parents confirm");
    }
  }
}

// Receives the serialized PSBT, signs the inputs that are not already
// finalized, and returns the serialized signed PSBT.
Future<Uint8List> signPsbtWithYourKeys(Uint8List psbtBytes) async {
  return psbtBytes;
}
// ANCHOR_END: custom-cpfp-signer
