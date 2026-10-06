import BigNumber
import BreezSdkSpark

func createTokenAllowance(sdk: BreezSdk) async throws {
    // ANCHOR: create-token-allowance
    let response = try await sdk.createTokenAllowance(
        request: CreateTokenAllowanceRequest(
            spenderPublicKey: "<spender identity public key>",
            tokenIdentifier: "<token identifier>",
            maxPerPayment: .amount(amount: BInt(5_000_000)),
            maxTotal: .amount(amount: BInt(100_000_000)),
            expiryTime: 1_798_761_600,
            allowedRecipients: []
        ))
    print("Allowance id: \(response.allowance.id)")
    // ANCHOR_END: create-token-allowance
}

func listTokenAllowances(sdk: BreezSdk) async throws {
    // ANCHOR: list-token-allowances
    let response = try await sdk.listTokenAllowances(
        request: ListTokenAllowancesRequest(
            role: .owner,
            counterpartyPublicKey: nil,
            tokenIdentifier: nil,
            includeInactive: false,
            offset: nil,
            limit: nil
        ))
    for allowance in response.allowances {
        print("\(allowance.id): spent \(allowance.spentAmount)")
    }
    // ANCHOR_END: list-token-allowances
}

func revokeTokenAllowance(sdk: BreezSdk) async throws {
    // ANCHOR: revoke-token-allowance
    try await sdk.revokeTokenAllowance(
        request: RevokeTokenAllowanceRequest(allowanceId: "<allowance id>"))
    // ANCHOR_END: revoke-token-allowance
}

func pullPayment(sdk: BreezSdk) async throws {
    // ANCHOR: pull-payment
    let prepareResponse = try await sdk.preparePullPayment(
        request: PreparePullPaymentRequest(
            payerPublicKey: "<payer identity public key>",
            tokenIdentifier: "<token identifier>",
            receivers: [
                PullReceiver(amount: BInt(5_000_000), receiverPublicKey: nil)
            ]
        ))
    print("Pulling \(prepareResponse.amount)")

    let response = try await sdk.pullPayment(
        request: PullPaymentRequest(prepareResponse: prepareResponse))
    print("Pull transaction: \(response.txHash)")
    // ANCHOR_END: pull-payment
}
