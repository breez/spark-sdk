import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'helper.dart';

Future<void> listUnclaimedDeposits(BreezSdk sdk) async {
  // ANCHOR: list-unclaimed-deposits
  final request = ListUnclaimedDepositsRequest();
  final response = await sdk.listUnclaimedDeposits(request: request);

  for (DepositInfo deposit in response.deposits) {
    print("Unclaimed deposit: ${deposit.txid}:${deposit.vout}");
    print("Amount: ${deposit.amountSats} sats");

    final claimError = deposit.claimError;
    if (claimError is DepositClaimError_MaxDepositClaimFeeExceeded) {
      final maxFeeStr = claimError.maxFee != null
          ? (claimError.maxFee is Fee_Fixed
              ? '${(claimError.maxFee as Fee_Fixed).amount} sats'
              : '${(claimError.maxFee as Fee_Rate).satPerVbyte} sats/vByte')
          : 'none';
      print(
          "Max claim fee exceeded. Max: $maxFeeStr, Required: ${claimError.requiredFeeSats} sats or ${claimError.requiredFeeRateSatPerVbyte} sats/vByte");
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
    final requiredFee = claimError.requiredFeeSats;

    // Show UI to user with the required fee and get approval
    bool userApproved = true; // Replace with actual user approval logic

    if (userApproved) {
      final claimRequest = ClaimDepositRequest(
        txid: deposit.txid,
        vout: deposit.vout,
        maxFee: MaxFee.fixed(amount: requiredFee),
      );
      await sdk.claimDeposit(request: claimRequest);
    }
  }
  // ANCHOR_END: handle-fee-exceeded
}

Future<void> refundDeposit(BreezSdk sdk) async {
  // ANCHOR: refund-deposit
  String txid = "your_deposit_txid";
  int vout = 0;
  String destinationAddress = "bc1qexample..."; // Your Bitcoin address

  // Set the fee for the refund transaction using the half-hour feerate
  final recommendedFees = await sdk.recommendedFees();
  Fee fee = Fee.rate(satPerVbyte: recommendedFees.halfHourFee);
  // or using a fixed amount
  //Fee fee = Fee.fixed(amount: BigInt.from(500));
  //
  // Important: The total fee must be at least 194 sats to ensure the
  // transaction can be relayed by the Bitcoin network. If the fee is
  // lower, the refund request will be rejected.

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

Future<void> setMaxFeeToRecommendedFees() async {
  // ANCHOR: set-max-fee-to-recommended-fees
  // Create the default config
  var config = defaultConfig(network: Network.mainnet);
  config = config.copyWith(apiKey: "<breez api key>");

  // Set the maximum fee to the fastest network recommended fee at the time of claim
  // with a leeway of 1 sats/vbyte
  config = config.copyWith(
      maxDepositClaimFee:
          MaxFee.networkRecommended(leewaySatPerVbyte: BigInt.from(1)));
  // ANCHOR_END: set-max-fee-to-recommended-fees
  print("Config: $config");
}

Future<void> customClaimLogic(BreezSdk sdk, DepositInfo deposit) async {
  // ANCHOR: custom-claim-logic
  final claimError = deposit.claimError;
  if (claimError is DepositClaimError_MaxDepositClaimFeeExceeded) {
    final requiredFeeRate = claimError.requiredFeeRateSatPerVbyte;

    final recommendedFees = await sdk.recommendedFees();

    if (requiredFeeRate <= recommendedFees.fastestFee) {
      final claimRequest = ClaimDepositRequest(
        txid: deposit.txid,
        vout: deposit.vout,
        maxFee: MaxFee.rate(satPerVbyte: requiredFeeRate),
      );
      await sdk.claimDeposit(request: claimRequest);
    }
  }
  // ANCHOR_END: custom-claim-logic
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
