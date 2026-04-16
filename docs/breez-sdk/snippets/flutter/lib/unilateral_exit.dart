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
  for (UnilateralExitLeaf leaf in response.leaves) {
    print("Leaf ${leaf.leafId}: ${leaf.value} sats (exit cost: ~${leaf.estimatedCost} sats)");
    for (UnilateralExitTransaction tx in leaf.transactions) {
      if (tx.csvTimelockBlocks != null) {
        print("Timelock: wait ${tx.csvTimelockBlocks} blocks");
      }
      // tx.txHex: pre-signed Spark transaction
      // tx.cpfpTxHex: signed CPFP transaction — broadcast alongside parent
    }
  }

  // Check if any node confirmations couldn't be verified
  if (response.unverifiedNodeIds.isNotEmpty) {
    print("Warning: could not verify confirmation status for ${response.unverifiedNodeIds.length} nodes");
  }

  // response.sweepTxHex: signed sweep transaction for the final step
  // Broadcast after refund transactions confirm and CSV timelocks expire.
  // ANCHOR_END: prepare-unilateral-exit
  return response;
}
