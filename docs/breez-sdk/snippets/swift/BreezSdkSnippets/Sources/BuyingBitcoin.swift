import BreezSdkSpark

func buyBitcoin(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin
    // Optionally, lock the purchase to a specific amount
    let optionalLockedAmountSat: UInt64 = 100_000
    // Optionally, set a redirect URL for after the purchase is completed
    let optionalRedirectUrl = "https://example.com/purchase-complete"

    let request = BuyBitcoinRequest(
        lockedAmountSat: optionalLockedAmountSat,
        redirectUrl: optionalRedirectUrl
    )

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin
}
