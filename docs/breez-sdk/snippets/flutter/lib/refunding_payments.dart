import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> listUnclaimedDeposits(BreezSdk sdk) async {
  // ANCHOR: list-unclaimed-deposits
  final request = ListUnclaimedDepositsRequest();
  final response = await sdk.listUnclaimedDeposits(request: request);

  for (DepositInfo deposit in response.deposits) {
    print("Unclaimed deposit: ${deposit.txid}:${deposit.vout}");
    print("Amount: ${deposit.amountSats} sats");

    final claimError = deposit.claimError;
    if (claimError is DepositClaimError_DepositClaimFeeExceeded) {
      print(
          "Max claim fee exceeded. Max: ${claimError.maxFee}, Actual: ${claimError.actualFee} sats");
    } else if (claimError is DepositClaimError_MissingUtxo) {
      print("UTXO not found when claiming deposit");
    } else if (claimError is DepositClaimError_Generic) {
      print("Claim failed: ${claimError.message}");
    }
  }
  // ANCHOR_END: list-unclaimed-deposits
}

Future<void> claimDeposit(BreezSdk sdk) async {
  // ANCHOR: claim-deposit
  String txid = "your_deposit_txid";
  int vout = 0;

  Fee maxFee = Fee.fixed(amount: BigInt.from(5000));
  final request = ClaimDepositRequest(
    txid: txid,
    vout: vout,
    maxFee: maxFee,
  );

  final response = await sdk.claimDeposit(request: request);
  print("Deposit claimed successfully. Payment: ${response.payment}");
  // ANCHOR_END: claim-deposit
}

Future<void> refundDeposit(BreezSdk sdk) async {
  // ANCHOR: refund-deposit
  String txid = "your_deposit_txid";
  int vout = 0;
  String destinationAddress = "bc1qexample..."; // Your Bitcoin address

  // Set the fee for the refund transaction
  Fee fee = Fee.fixed(amount: BigInt.from(500));

  final request = RefundDepositRequest(
    txid: txid,
    vout: vout,
    destinationAddress: destinationAddress,
    fee: fee,
  );

  final response = await sdk.refundDeposit(request: request);
  print("Refund transaction created:");
  print("Transaction ID: ${response.txId}");
  print("Transaction hex: ${response.txHex}");
  // ANCHOR_END: refund-deposit
}
