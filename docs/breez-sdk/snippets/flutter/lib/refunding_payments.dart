import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> listUnclaimedDeposits(BreezSdk sdk) async {
  // ANCHOR: list-unclaimed-deposits
  final request = ListUnclaimedDepositsRequest();
  final response = await sdk.listUnclaimedDeposits(request: request);

  for (DepositInfo deposit in response.deposits) {
    print("Unclaimed deposit: ${deposit.txid}:${deposit.vout}");
    print("Amount: ${deposit.amountSats} sats");

    final claimError = deposit.claimError;
    if (claimError is DepositClaimError_MaxDepositClaimFeeExceeded) {
      final maxFeeStr = claimError.maxFee != null ? '${claimError.maxFee} sats' : 'none';
      print(
          "Max claim fee exceeded. Max: $maxFeeStr, Required: ${claimError.requiredFee} sats");
    } else if (claimError is DepositClaimError_MissingUtxo) {
      print("UTXO not found when claiming deposit");
    } else if (claimError is DepositClaimError_Generic) {
      print("Claim failed: ${claimError.message}");
    }
  }
  // ANCHOR_END: list-unclaimed-deposits
}

Future<void> handleFeeExceeded(BreezSdk sdk, DepositInfo deposit) async {
  // ANCHOR: handle-fee-exceeded
  final claimError = deposit.claimError;
  if (claimError is DepositClaimError_MaxDepositClaimFeeExceeded) {
    final requiredFee = claimError.requiredFee;

    // Show UI to user with the required fee and get approval
    bool userApproved = true; // Replace with actual user approval logic

    if (userApproved) {
      final claimRequest = ClaimDepositRequest(
        txid: deposit.txid,
        vout: deposit.vout,
        maxFee: Fee.fixed(amount: requiredFee),
      );
      await sdk.claimDeposit(request: claimRequest);
    }
  }
  // ANCHOR_END: handle-fee-exceeded
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

  // Set the fee for the refund transaction using a rate
  Fee fee = Fee.rate(satPerVbyte: BigInt.from(5));
  // or using a fixed amount
  //Fee fee = Fee.fixed(amount: BigInt.from(500));

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

Future<void> recommendedFees(BreezSdk sdk) async {
  // ANCHOR: recommended-fees
  final response = await sdk.recommendedFees();
  print("Fastest fee: ${response.fastestFee} sats/vByte");
  print("Half-hour fee: ${response.halfHourFee} sats/vByte");
  print("Hour fee: ${response.hourFee} sats/vByte");
  print("Economy fee: ${response.economyFee} sats/vByte");
  print("Minimum fee: ${response.minimumFee} sats/vByte");
  // ANCHOR_END: recommended-fees
}
