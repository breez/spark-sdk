import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<List<Leaf>> listLeavesForExit(BreezSdk sdk) async {
  // ANCHOR: list-leaves
  ListLeavesRequest request = ListLeavesRequest(minValueSats: BigInt.from(10000));
  ListLeavesResponse response = await sdk.listLeaves(request: request);

  for (Leaf leaf in response.leaves) {
    print("Leaf ${leaf.id}: ${leaf.value} sats");
  }
  // ANCHOR_END: list-leaves
  return response.leaves;
}

Future<PrepareUnilateralExitResponse> prepareExit(BreezSdk sdk) async {
  // ANCHOR: prepare-unilateral-exit
  List<String> leafIds = ["leaf-id-1", "leaf-id-2"];

  PrepareUnilateralExitRequest request = PrepareUnilateralExitRequest(
    feeRate: BigInt.from(2),
    leafIds: leafIds,
    utxos: [
      UnilateralExitCpfpUtxo(
        txid: "your-utxo-txid",
        vout: 0,
        value: BigInt.from(50000),
        pubkey: "your-compressed-pubkey-hex",
        utxoType: UnilateralExitCpfpUtxoType.p2Wpkh,
      ),
    ],
    destination: "bc1q...your-destination-address",
  );

  PrepareUnilateralExitResponse response =
      await sdk.prepareUnilateralExit(request: request);

  // The response contains:
  // - response.leaves: transaction/PSBT pairs to sign and broadcast
  // - response.sweepTxHex: signed sweep transaction for the final step
  for (UnilateralExitLeafTxCpfpPsbts leaf in response.leaves) {
    for (UnilateralExitTxCpfpPsbt pair in leaf.txCpfpPsbts) {
      if (pair.csvTimelockBlocks != null) {
        print("Timelock: wait ${pair.csvTimelockBlocks} blocks");
      }
      // pair.parentTxHex: pre-signed Spark transaction
      // pair.childPsbtHex: unsigned CPFP PSBT — sign with your UTXO key
    }
  }
  // ANCHOR_END: prepare-unilateral-exit
  return response;
}
