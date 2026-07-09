using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class ClientSigning
    {
        async Task<SignedTransferPackage> SignPackage(
            ExternalSparkSigner signer,
            UnsignedTransferPackage unsigned)
        {
            // ANCHOR: client-signing-sign-package
            TransferSignature signature;
            switch (unsigned)
            {
                case UnsignedTransferPackage.Transfer transfer:
                    // Show the user what they are approving before signing
                    var destination = transfer.target switch
                    {
                        TransferTarget.Spark spark => spark.address,
                        TransferTarget.Lightning lightning => lightning.bolt11,
                        TransferTarget.CoopExit coopExit => coopExit.address,
                        _ => throw new Exception("Unknown transfer target")
                    };
                    Console.WriteLine($"Approve sending {transfer.amountSat} sats " +
                        $"(fee {transfer.feeSat} sats) to {destination}");
                    signature = new TransferSignature.Transfer(
                        signed: await signer.PrepareTransfer(transfer.prepareTransfer)
                    );
                    break;
                case UnsignedTransferPackage.Swap swap:
                    Console.WriteLine("Approve re-shaping funds for a " +
                        $"{swap.amountSat} sat send (fee {swap.feeSat} sats)");
                    signature = new TransferSignature.Transfer(
                        signed: await signer.PrepareTransfer(swap.prepareTransfer)
                    );
                    break;
                case UnsignedTransferPackage.Token token:
                    if (token.isSwap)
                    {
                        Console.WriteLine("Approve combining token outputs for a " +
                            $"{token.tokenIdentifier} send");
                    }
                    else
                    {
                        Console.WriteLine($"Approve sending {token.amount} of token " +
                            $"{token.tokenIdentifier} (fee {token.fee})");
                    }
                    signature = new TransferSignature.Token(
                        signed: await signer.PrepareTokenTransaction(token.prepareTokenTransaction)
                    );
                    break;
                default:
                    throw new Exception("Unknown transfer package");
            }

            var signedPackage = new SignedTransferPackage(unsigned: unsigned, signature: signature);
            // ANCHOR_END: client-signing-sign-package
            return signedPackage;
        }

        async Task<Payment> SendWithClientSigning(BreezSdk sdk, ExternalSparkSigner signer)
        {
            // ANCHOR: client-signing-send
            var prepareResponse = await sdk.PrepareSendPayment(
                request: new PrepareSendPaymentRequest(
                    paymentRequest: new PaymentRequest.Input(input: "<spark address or invoice>"),
                    amount: 5_000UL,
                    tokenIdentifier: null,
                    conversionOptions: null,
                    feePolicy: null
                )
            );

            while (true)
            {
                var unsigned = await sdk.BuildUnsignedTransferPackage(
                    request: new BuildUnsignedTransferPackageRequest(
                        prepareResponse: prepareResponse,
                        options: null
                    )
                );

                // Send the package to the user, who reviews and signs it
                var signedPackage = await SignPackage(signer, unsigned);

                var response = await sdk.PublishSignedTransferPackage(
                    request: new PublishSignedTransferPackageRequest(signedPackage: signedPackage)
                );

                switch (response)
                {
                    // The wallet's funds were re-shaped first: build the payment again
                    case PublishSignedTransferPackageResponse.SwapCompleted:
                        continue;
                    case PublishSignedTransferPackageResponse.PaymentSent paymentSent:
                        return paymentSent.payment;
                }
            }
            // ANCHOR_END: client-signing-send
        }

        async Task BuildOnchainPackage(BreezSdk sdk, PrepareSendPaymentResponse prepareResponse)
        {
            // ANCHOR: client-signing-build-onchain-options
            // For Bitcoin address sends, the confirmation speed is chosen when
            // building the package: the fee depends on it
            var unsigned = await sdk.BuildUnsignedTransferPackage(
                request: new BuildUnsignedTransferPackageRequest(
                    prepareResponse: prepareResponse,
                    options: new BuildTransferPackageOptions.BitcoinAddress(
                        confirmationSpeed: OnchainConfirmationSpeed.Medium
                    )
                )
            );
            // ANCHOR_END: client-signing-build-onchain-options
            Console.WriteLine($"Unsigned package: {unsigned}");
        }

        async Task BuildBolt11Package(BreezSdk sdk, PrepareSendPaymentResponse prepareResponse)
        {
            // ANCHOR: client-signing-build-bolt11-options
            var unsigned = await sdk.BuildUnsignedTransferPackage(
                request: new BuildUnsignedTransferPackageRequest(
                    prepareResponse: prepareResponse,
                    options: new BuildTransferPackageOptions.Bolt11Invoice(
                        preferSpark: true,
                        completionTimeoutSecs: 10
                    )
                )
            );
            // ANCHOR_END: client-signing-build-bolt11-options
            Console.WriteLine($"Unsigned package: {unsigned}");
        }

        async Task<LnurlPayResponse> LnurlPayWithClientSigning(
            BreezSdk sdk,
            ExternalSparkSigner signer,
            PrepareLnurlPayResponse prepareResponse)
        {
            // ANCHOR: client-signing-lnurl-pay
            while (true)
            {
                var unsigned = await sdk.BuildUnsignedLnurlPayPackage(
                    request: new BuildUnsignedLnurlPayPackageRequest(
                        prepareResponse: prepareResponse
                    )
                );

                var signedPackage = await SignPackage(signer, unsigned);

                var response = await sdk.PublishSignedLnurlPayPackage(
                    request: new PublishSignedLnurlPayPackageRequest(signedPackage: signedPackage)
                );

                switch (response)
                {
                    case PublishSignedLnurlPayResponse.SwapCompleted:
                        continue;
                    case PublishSignedLnurlPayResponse.PaymentSent paymentSent:
                        return paymentSent.response;
                }
            }
            // ANCHOR_END: client-signing-lnurl-pay
        }
    }
}
