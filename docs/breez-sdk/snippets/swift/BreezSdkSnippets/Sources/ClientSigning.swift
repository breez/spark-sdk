import BigNumber
import BreezSdkSpark

func signPackage(signer: ExternalSparkSigner, unsigned: UnsignedTransferPackage)
    async throws -> SignedTransferPackage
{
    // ANCHOR: client-signing-sign-package
    let signature: TransferSignature
    switch unsigned {
    case let .transfer(prepareTransfer, amountSat, feeSat, target):
        // Show the user what they are approving before signing
        let destination: String
        switch target {
        case let .spark(address, _):
            destination = address
        case let .lightning(bolt11, _, _, _):
            destination = bolt11
        case let .coopExit(address, _, _):
            destination = address
        }
        print("Approve sending \(amountSat) sats (fee \(feeSat) sats) to \(destination)")
        signature = TransferSignature.transfer(
            signed: try await signer.prepareTransfer(request: prepareTransfer)
        )
    case let .swap(prepareTransfer, _, amountSat, feeSat):
        print("Approve re-shaping funds for a \(amountSat) sat send (fee \(feeSat) sats)")
        signature = TransferSignature.transfer(
            signed: try await signer.prepareTransfer(request: prepareTransfer)
        )
    case let .token(prepareTokenTransaction, _, tokenIdentifier, amount, fee, isSwap):
        if isSwap {
            print("Approve combining token outputs for a \(tokenIdentifier) send")
        } else {
            print("Approve sending \(amount) of token \(tokenIdentifier) (fee \(fee))")
        }
        signature = TransferSignature.token(
            signed: try await signer.prepareTokenTransaction(request: prepareTokenTransaction)
        )
    case let .tokenBatch(prepareTokenTransaction, _, totals, isSwap):
        if isSwap {
            print("Approve combining token outputs before the batch is sent")
        } else {
            for total in totals {
                print("Approve sending \(total.amount) of token \(total.tokenIdentifier)")
            }
        }
        signature = TransferSignature.token(
            signed: try await signer.prepareTokenTransaction(request: prepareTokenTransaction)
        )
    }

    let signedPackage = SignedTransferPackage(
        unsigned: unsigned,
        signature: signature
    )
    // ANCHOR_END: client-signing-sign-package
    return signedPackage
}

func sendWithClientSigning(sdk: BreezSdk, signer: ExternalSparkSigner) async throws -> Payment {
    // ANCHOR: client-signing-send
    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: .input(input: "<spark address or invoice>"),
            amount: BInt(5_000),
            tokenIdentifier: nil,
            conversionOptions: nil,
            feePolicy: nil
        ))

    while true {
        let unsigned = try await sdk.buildUnsignedTransferPackage(
            request: BuildUnsignedTransferPackageRequest(
                prepareResponse: prepareResponse,
                options: nil
            ))

        // Send the package to the user, who reviews and signs it
        let signedPackage = try await signPackage(signer: signer, unsigned: unsigned)

        let publishResponse = try await sdk.publishSignedTransferPackage(
            request: PublishSignedTransferPackageRequest(signedPackage: signedPackage))

        switch publishResponse {
        // The wallet's funds were re-shaped first: build the payment again
        case .swapCompleted:
            continue
        case let .paymentSent(payment):
            return payment
        // Only a batch package pays several recipients at once
        case .paymentsSent:
            throw SdkError.InvalidInput("unexpected batch response for a single payment")
        }
    }
    // ANCHOR_END: client-signing-send
}

func buildOnchainPackage(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) async throws {
    // ANCHOR: client-signing-build-onchain-options
    // For Bitcoin address sends, the confirmation speed is chosen when
    // building the package: the fee depends on it
    let unsigned = try await sdk.buildUnsignedTransferPackage(
        request: BuildUnsignedTransferPackageRequest(
            prepareResponse: prepareResponse,
            options: BuildTransferPackageOptions.bitcoinAddress(
                confirmationSpeed: OnchainConfirmationSpeed.medium
            )
        ))
    // ANCHOR_END: client-signing-build-onchain-options
    print("Unsigned package: \(unsigned)")
}

func buildBolt11Package(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) async throws {
    // ANCHOR: client-signing-build-bolt11-options
    let unsigned = try await sdk.buildUnsignedTransferPackage(
        request: BuildUnsignedTransferPackageRequest(
            prepareResponse: prepareResponse,
            options: BuildTransferPackageOptions.bolt11Invoice(
                preferSpark: true,
                completionTimeoutSecs: 10
            )
        ))
    // ANCHOR_END: client-signing-build-bolt11-options
    print("Unsigned package: \(unsigned)")
}

func lnurlPayWithClientSigning(
    sdk: BreezSdk, signer: ExternalSparkSigner, prepareResponse: PrepareLnurlPayResponse
) async throws -> LnurlPayResponse {
    // ANCHOR: client-signing-lnurl-pay
    while true {
        let unsigned = try await sdk.buildUnsignedLnurlPayPackage(
            request: BuildUnsignedLnurlPayPackageRequest(
                prepareResponse: prepareResponse
            ))

        let signedPackage = try await signPackage(signer: signer, unsigned: unsigned)

        let publishResponse = try await sdk.publishSignedLnurlPayPackage(
            request: PublishSignedLnurlPayPackageRequest(signedPackage: signedPackage))

        switch publishResponse {
        case .swapCompleted:
            continue
        case let .paymentSent(response):
            return response
        }
    }
    // ANCHOR_END: client-signing-lnurl-pay
}
