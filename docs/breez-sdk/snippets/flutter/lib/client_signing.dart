import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

abstract class PackageSigner {
  Future<ExternalPreparedTransfer> prepareTransfer(
      ExternalPrepareTransferRequest request);
  Future<ExternalPreparedTokenTransaction> prepareTokenTransaction(
      ExternalPrepareTokenTransactionRequest request);
}

Future<SignedTransferPackage> signPackage(
    PackageSigner signer, UnsignedTransferPackage unsigned) async {
  // ANCHOR: client-signing-sign-package
  TransferSignature signature;
  if (unsigned is UnsignedTransferPackage_Transfer) {
    // Show the user what they are approving before signing
    final target = unsigned.target;
    String destination = "";
    if (target is TransferTarget_Spark) {
      destination = target.address;
    } else if (target is TransferTarget_Lightning) {
      destination = target.bolt11;
    } else if (target is TransferTarget_CoopExit) {
      destination = target.address;
    }
    print("Approve sending ${unsigned.amountSat} sats"
        " (fee ${unsigned.feeSat} sats) to $destination");
    signature = TransferSignature.transfer(
        signed: await signer.prepareTransfer(unsigned.prepareTransfer));
  } else if (unsigned is UnsignedTransferPackage_Swap) {
    print("Approve re-shaping funds for a ${unsigned.amountSat} sat send"
        " (fee ${unsigned.feeSat} sats)");
    signature = TransferSignature.transfer(
        signed: await signer.prepareTransfer(unsigned.prepareTransfer));
  } else if (unsigned is UnsignedTransferPackage_Token) {
    if (unsigned.isSwap) {
      print("Approve combining token outputs for a ${unsigned.tokenIdentifier} send");
    } else {
      print("Approve sending ${unsigned.amount} of token"
          " ${unsigned.tokenIdentifier} (fee ${unsigned.fee})");
    }
    signature = TransferSignature.token(
        signed: await signer
            .prepareTokenTransaction(unsigned.prepareTokenTransaction));
  } else {
    throw Exception("Unknown transfer package variant");
  }

  final signedPackage =
      SignedTransferPackage(unsigned: unsigned, signature: signature);
  // ANCHOR_END: client-signing-sign-package
  return signedPackage;
}

Future<Payment> sendWithClientSigning(BreezSdk sdk, PackageSigner signer) async {
  // ANCHOR: client-signing-send
  final prepareResponse = await sdk.prepareSendPayment(
      request: PrepareSendPaymentRequest(
          paymentRequest:
              PaymentRequest.input(input: "<spark address or invoice>"),
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
    if (result is PublishSignedTransferPackageResponse_SwapCompleted) {
      // The wallet's funds were re-shaped first: build the payment again
      continue;
    }
    if (result is PublishSignedTransferPackageResponse_PaymentSent) {
      return result.payment;
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
          options: BuildTransferPackageOptions.bitcoinAddress(
              confirmationSpeed: OnchainConfirmationSpeed.medium)));
  // ANCHOR_END: client-signing-build-onchain-options
  print("Unsigned package: $unsigned");
}

Future<void> buildBolt11Package(
    BreezSdk sdk, PrepareSendPaymentResponse prepareResponse) async {
  // ANCHOR: client-signing-build-bolt11-options
  final unsigned = await sdk.buildUnsignedTransferPackage(
      request: BuildUnsignedTransferPackageRequest(
          prepareResponse: prepareResponse,
          options: BuildTransferPackageOptions.bolt11Invoice(
              preferSpark: true, completionTimeoutSecs: 10)));
  // ANCHOR_END: client-signing-build-bolt11-options
  print("Unsigned package: $unsigned");
}

Future<LnurlPayResponse> lnurlPayWithClientSigning(BreezSdk sdk,
    PackageSigner signer, PrepareLnurlPayResponse prepareResponse) async {
  // ANCHOR: client-signing-lnurl-pay
  while (true) {
    final unsigned = await sdk.buildUnsignedLnurlPayPackage(
        request:
            BuildUnsignedLnurlPayPackageRequest(prepareResponse: prepareResponse));

    final signedPackage = await signPackage(signer, unsigned);

    final result = await sdk.publishSignedLnurlPayPackage(
        request:
            PublishSignedLnurlPayPackageRequest(signedPackage: signedPackage));
    if (result is PublishSignedLnurlPayResponse_SwapCompleted) {
      continue;
    }
    if (result is PublishSignedLnurlPayResponse_PaymentSent) {
      return result.response;
    }
  }
  // ANCHOR_END: client-signing-lnurl-pay
}
