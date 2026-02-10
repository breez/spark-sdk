import BreezSdkSpark

func buyBitcoin(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin
    // Buy Bitcoin with funds deposited directly into the user's wallet.
    // Optionally lock the purchase to a specific amount and provide a redirect URL.
    let request = BuyBitcoinRequest(
        lockedAmountSat: 100_000,
        redirectUrl: "https://example.com/purchase-complete"
    )

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin
}
