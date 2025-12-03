import BigNumber
import BreezSdkSpark

func fetchTokenBalances(sdk: BreezSdk) async throws {
    // ANCHOR: fetch-token-balances
    // ensureSynced: true will ensure the SDK is synced with the Spark network
    // before returning the balance
    let info = try await sdk.getInfo(
        request: GetInfoRequest(
            ensureSynced: false
        ))

    // Token balances are a map of token identifier to balance
    let tokenBalances = info.tokenBalances
    for (tokenId, tokenBalance) in tokenBalances {
        print("Token ID: \(tokenId)")
        print("Balance: \(tokenBalance.balance)")
        print("Name: \(tokenBalance.tokenMetadata.name)")
        print("Ticker: \(tokenBalance.tokenMetadata.ticker)")
        print("Decimals: \(tokenBalance.tokenMetadata.decimals)")
    }
    // ANCHOR_END: fetch-token-balances
}

func fetchTokenMetadata(sdk: BreezSdk) async throws {
    // ANCHOR: fetch-token-metadata
    let response = try await sdk.getTokensMetadata(
        request: GetTokensMetadataRequest(tokenIdentifiers: [
            "<token identifier 1>", "<token identifier 2>",
        ]))

    let tokensMetadata = response.tokensMetadata
    for tokenMetadata in tokensMetadata {
        print("Token ID: \(tokenMetadata.identifier)")
        print("Name: \(tokenMetadata.name)")
        print("Ticker: \(tokenMetadata.ticker)")
        print("Decimals: \(tokenMetadata.decimals)")
        print("Max Supply: \(tokenMetadata.maxSupply)")
        print("Is Freezable: \(tokenMetadata.isFreezable)")
    }
    // ANCHOR_END: fetch-token-metadata
}

func receiveTokenPaymentSparkInvoice(sdk: BreezSdk) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-token-payment-spark-invoice
    let tokenIdentifier = "<token identifier>"
    let optionalDescription = "<invoice description>"
    let optionalAmount = BInt(5_000)
    let optionalExpiryTimeSeconds: UInt64 = 1_716_691_200
    let optionalSenderPublicKey = "<sender public key>"

    let response =
        try await sdk
        .receivePayment(
            request: ReceivePaymentRequest(
                paymentMethod: ReceivePaymentMethod.sparkInvoice(
                    amount: optionalAmount,
                    tokenIdentifier: tokenIdentifier,
                    expiryTime: optionalExpiryTimeSeconds,
                    description: optionalDescription,
                    senderPublicKey: optionalSenderPublicKey
                )
            ))

    let paymentRequest = response.paymentRequest
    print("Payment request: \(paymentRequest)")
    let receiveFeeSats = response.fee
    print("Fees: \(receiveFeeSats) token base units")
    // ANCHOR_END: receive-token-payment-spark-invoice

    return response
}

func sendTokenPayment(sdk: BreezSdk) async throws {
    // ANCHOR: send-token-payment
    let paymentRequest = "<spark address or invoice>"
    // Token identifier must match the invoice in case it specifies one.
    let tokenIdentifier = "<token identifier>"
    // Set the amount of tokens you wish to send. (requires 'import BigNumber')
    let optionalAmount = BInt(1_000)

    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: paymentRequest,
            amount: optionalAmount,
            tokenIdentifier: tokenIdentifier
        ))

    // If the fees are acceptable, continue to send the token payment
    if case let .sparkAddress(address, fee, tokenId) = prepareResponse.paymentMethod {
        print("Token ID: \(String(describing: tokenId))")
        print("Fees: \(fee) token base units")
    }
    if case let .sparkInvoice(invoice, fee, tokenId) = prepareResponse.paymentMethod {
        print("Token ID: \(String(describing: tokenId))")
        print("Fees: \(fee) token base units")
    }

    // Send the token payment
    let sendResponse = try await sdk.sendPayment(
        request: SendPaymentRequest(
            prepareResponse: prepareResponse,
            options: nil
        ))
    let payment = sendResponse.payment
    print("Payment: \(payment)")
    // ANCHOR_END: send-token-payment
}

func prepareTransferTokenToBitcoin(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-transfer-token-to-bitcoin
    let tokenIdentifier = "<token identifier>"
    // Amount in token base units
    let amount = BInt(10_000_000)

    let prepareResponse = try await sdk.prepareTransferToken(
        request: PrepareTransferTokenRequest(
            transferType: TransferType.toBitcoin,
            tokenIdentifier: tokenIdentifier,
            amount: amount
        ))

    let estimatedReceiveAmount = prepareResponse.estimatedReceiveAmount
    let fee = prepareResponse.fee
    print("Estimated receive amount: \(estimatedReceiveAmount) sats")
    print("Fee: \(fee) token base units")
    // ANCHOR_END: prepare-transfer-token-to-bitcoin
}

func prepareTransferTokenFromBitcoin(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-transfer-token-from-bitcoin
    let tokenIdentifier = "<token identifier>"
    // Amount in satoshis
    let amount = BInt(10_000)

    let prepareResponse = try await sdk.prepareTransferToken(
        request: PrepareTransferTokenRequest(
            transferType: TransferType.fromBitcoin,
            tokenIdentifier: tokenIdentifier,
            amount: amount
        ))

    let estimatedReceiveAmount = prepareResponse.estimatedReceiveAmount
    let fee = prepareResponse.fee
    print("Estimated receive amount: \(estimatedReceiveAmount) token base units")
    print("Fee: \(fee) sats")
    // ANCHOR_END: prepare-transfer-token-from-bitcoin
}

func transferToken(sdk: BreezSdk, prepareResponse: PrepareTransferTokenResponse) async throws {
    // ANCHOR: transfer-token
    // Set the maximum slippage to 1% in basis points
    let optionalMaxSlippageBps = UInt32(100)

    let response = try await sdk.transferToken(
        request: TransferTokenRequest(
            prepareResponse: prepareResponse,
            maxSlippageBps: optionalMaxSlippageBps
        ))
    
    let sentPayment = response.sentPayment
    let receivedPayment = response.receivedPayment
    print("Sent payment: \(sentPayment)")
    if let receivedPayment = receivedPayment {
        print("Received payment: \(receivedPayment)")
    }
    // ANCHOR_END: transfer-token
}