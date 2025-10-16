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

func sendTokenPayment(sdk: BreezSdk) async throws {
    // ANCHOR: send-token-payment
    let paymentRequest = "<spark address>"
    // The token identifier (e.g., asset ID or token contract)
    let tokenIdentifier = "<token identifier>"
    // Set the amount of tokens you wish to send (requires 'import BigNumber')
    let amount = BInt(1_000)

    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: paymentRequest,
            amount: amount,
            tokenIdentifier: tokenIdentifier
        ))

    // If the fees are acceptable, continue to send the token payment
    if case let .sparkAddress(address, fee, tokenId) = prepareResponse.paymentMethod {
        print("Token ID: \(String(describing: tokenId))")
        print("Fees: \(fee) sats")
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
