import 'dart:typed_data';

import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:convert/convert.dart';

Future<PrepareUnilateralExitResponse> prepareExit(BreezSdk sdk) async {
  // ANCHOR: prepare-unilateral-exit
  // Create a signer from your UTXO private key (32-byte secret key)
  List<int> secretKeyBytes = hex.decode("your-secret-key-hex");

  PrepareUnilateralExitRequest request = PrepareUnilateralExitRequest(
    feeRate: BigInt.from(2),
    inputs: [
      UnilateralExitCpfpInput.p2wpkh(
        txid: "your-utxo-txid",
        vout: 0,
        value: BigInt.from(50000),
        pubkey: "your-compressed-pubkey-hex",
      ),
    ],
    destination: "bc1q...your-destination-address",
  );

  PrepareUnilateralExitResponse response = await sdk.prepareUnilateralExit(
    request: request,
    signerSecretKey: Uint8List.fromList(secretKeyBytes),
  );

  // The SDK automatically selects which leaves are profitable to exit.
  for (UnilateralExitLeafSummary leaf in response.selectedLeaves) {
    print("Leaf ${leaf.id}: ${leaf.value} sats (exit cost: ~${leaf.estimatedCost} sats)");
  }

  // The response contains signed transactions ready to broadcast:
  // - response.transactions: parent/child transaction pairs per leaf
  // - response.sweepTxHex: signed sweep transaction for the final step
  // Change from CPFP fee-bumping always goes back to the first input's address.
  for (UnilateralExitLeafTxCpfpPairs leaf in response.transactions) {
    for (UnilateralExitTxCpfpPair pair in leaf.txCpfpPairs) {
      if (pair.csvTimelockBlocks != null) {
        print("Timelock: wait ${pair.csvTimelockBlocks} blocks");
      }
      // pair.parentTxHex: pre-signed Spark transaction
      // pair.childTxHex: signed CPFP transaction — broadcast alongside parent
    }
  }
  // ANCHOR_END: prepare-unilateral-exit
  return response;
}
