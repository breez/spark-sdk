import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<SignedTransferPackage> signPackage(
    ExternalSparkSigner signer, UnsignedTransferPackage unsigned) async {
  // ANCHOR: client-signing-sign-package
  TransferSignature signature;
  if (unsigned is TransferUnsignedTransferPackage) {
    // Show the user what they are approving before signing
    final target = unsigned.target;
    String destination = "";
    if (target is SparkTransferTarget) {
      destination = target.address;
    } else if (target is LightningTransferTarget) {
      destination = target.bolt11;
    } else if (target is CoopExitTransferTarget) {
      destination = target.address;
    }
    print("Approve sending ${unsigned.amountSat} sats"
        " (fee ${unsigned.feeSat} sats) to $destination");
    signature = TransferTransferSignature(
        await signer.prepareTransfer(unsigned.prepareTransfer));
  } else if (unsigned is SwapUnsignedTransferPackage) {
    print("Approve re-shaping funds for a ${unsigned.amountSat} sat send"
        " (fee ${unsigned.feeSat} sats)");
    signature = TransferTransferSignature(
        await signer.prepareTransfer(unsigned.prepareTransfer));
  } else if (unsigned is TokenUnsignedTransferPackage) {
    if (unsigned.isSwap) {
      print("Approve combining token outputs for a ${unsigned.tokenIdentifier} send");
    } else {
      print("Approve sending ${unsigned.amount} of token"
          " ${unsigned.tokenIdentifier} (fee ${unsigned.fee})");
    }
    signature = TokenTransferSignature(
        await signer.prepareTokenTransaction(unsigned.prepareTokenTransaction));
  } else if (unsigned is TokenBatchUnsignedTransferPackage) {
    if (unsigned.isSwap) {
      print("Approve combining token outputs before the batch is sent");
    } else {
      for (final total in unsigned.totals) {
        print("Approve sending ${total.amount} of token"
            " ${total.tokenIdentifier}");
      }
    }
    signature = TokenTransferSignature(
        await signer.prepareTokenTransaction(unsigned.prepareTokenTransaction));
  } else if (unsigned is TokenPullUnsignedTransferPackage) {
    print("Approve pulling ${unsigned.amount} of token"
        " ${unsigned.tokenIdentifier} from ${unsigned.payerAddress}");
    for (final receiver in unsigned.receivers) {
      print("  ${receiver.amount} to ${receiver.receiverAddress}");
    }
    print("Expires at ${unsigned.expiryTime}");
    signature = TokenTransferSignature(
        await signer.prepareTokenTransaction(unsigned.prepareTokenTransaction));
  } else {
    throw Exception("Unknown transfer package variant");
  }

  final signedPackage =
      SignedTransferPackage(unsigned: unsigned, signature: signature);
  // ANCHOR_END: client-signing-sign-package
  return signedPackage;
}

Future<Payment> sendWithClientSigning(
    BreezSdk sdk, ExternalSparkSigner signer) async {
  // ANCHOR: client-signing-send
  final prepareResponse = await sdk.prepareSendPayment(
      request: PrepareSendPaymentRequest(
          paymentRequest: InputPaymentRequest("<spark address or invoice>"),
          amount: BigInt.from(5000),
          tokenIdentifier: null,
          conversionOptions: null,
          feePolicy: null));

  while (true) {
    final unsigned = await sdk.buildUnsignedTransferPackage(
        request: BuildUnsignedTransferPackageRequest(
            prepareResponse: prepareResponse, options: null));

    // Send the package to the user, who reviews and signs it
    final signedPackage = await signPackage(signer, unsigned);

    final result = await sdk.publishSignedTransferPackage(
        request:
            PublishSignedTransferPackageRequest(signedPackage: signedPackage));
    if (result is SwapCompletedPublishSignedTransferPackageResponse) {
      // The wallet's funds were re-shaped first: build the payment again
      continue;
    }
    if (result is PaymentSentPublishSignedTransferPackageResponse) {
      return result.payment;
    }
    // Only a batch package pays several recipients at once
    if (result is PaymentsSentPublishSignedTransferPackageResponse) {
      throw Exception("unexpected batch response for a single payment");
    }
  }
  // ANCHOR_END: client-signing-send
}

Future<void> buildOnchainPackage(
    BreezSdk sdk, PrepareSendPaymentResponse prepareResponse) async {
  // ANCHOR: client-signing-build-onchain-options
  // For Bitcoin address sends, the confirmation speed is chosen when
  // building the package: the fee depends on it
  final unsigned = await sdk.buildUnsignedTransferPackage(
      request: BuildUnsignedTransferPackageRequest(
          prepareResponse: prepareResponse,
          options: BitcoinAddressBuildTransferPackageOptions(
              OnchainConfirmationSpeed.medium)));
  // ANCHOR_END: client-signing-build-onchain-options
  print("Unsigned package: $unsigned");
}

Future<void> buildBolt11Package(
    BreezSdk sdk, PrepareSendPaymentResponse prepareResponse) async {
  // ANCHOR: client-signing-build-bolt11-options
  final unsigned = await sdk.buildUnsignedTransferPackage(
      request: BuildUnsignedTransferPackageRequest(
          prepareResponse: prepareResponse,
          options: Bolt11InvoiceBuildTransferPackageOptions(
              preferSpark: true, completionTimeoutSecs: 10)));
  // ANCHOR_END: client-signing-build-bolt11-options
  print("Unsigned package: $unsigned");
}

Future<LnurlPayResponse> lnurlPayWithClientSigning(BreezSdk sdk,
    ExternalSparkSigner signer, PrepareLnurlPayResponse prepareResponse) async {
  // ANCHOR: client-signing-lnurl-pay
  while (true) {
    final unsigned = await sdk.buildUnsignedLnurlPayPackage(
        request:
            BuildUnsignedLnurlPayPackageRequest(prepareResponse: prepareResponse));

    final signedPackage = await signPackage(signer, unsigned);

    final result = await sdk.publishSignedLnurlPayPackage(
        request:
            PublishSignedLnurlPayPackageRequest(signedPackage: signedPackage));
    if (result is SwapCompletedPublishSignedLnurlPayResponse) {
      continue;
    }
    if (result is PaymentSentPublishSignedLnurlPayResponse) {
      return result.response;
    }
  }
  // ANCHOR_END: client-signing-lnurl-pay
}

Future<PullPaymentResponse> pullWithClientSigning(BreezSdk sdk,
    ExternalSparkSigner signer, PreparePullPaymentResponse prepareResponse) async {
  // ANCHOR: client-signing-pull
  final unsigned = await sdk.buildUnsignedPullPackage(
      request:
          BuildUnsignedPullPackageRequest(prepareResponse: prepareResponse));
  final signedPackage = await signPackage(signer, unsigned);
  final response = await sdk.publishSignedPullPackage(
      request: PublishSignedPullPackageRequest(signedPackage: signedPackage));
  // ANCHOR_END: client-signing-pull
  return response;
}
