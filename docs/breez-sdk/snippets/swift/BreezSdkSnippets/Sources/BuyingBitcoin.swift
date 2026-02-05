import BreezSdkSpark

func buyBitcoinBasic(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin-basic
    // Buy Bitcoin using the SDK's auto-generated deposit address
    let request = BuyBitcoinRequest()

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin-basic
}

func buyBitcoinWithAmount(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin-with-amount
    // Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
    let request = BuyBitcoinRequest(
        lockedAmountSat: 100_000
    )

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin-with-amount
}

func buyBitcoinWithRedirect(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin-with-redirect
    // Provide a custom redirect URL for after the purchase
    let request = BuyBitcoinRequest(
        lockedAmountSat: 100_000,
        redirectUrl: "https://example.com/purchase-complete"
    )

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin-with-redirect
}

func buyBitcoinWithAddress(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin-with-address
    // Specify a custom Bitcoin address to receive funds
    let request = BuyBitcoinRequest(
        address: "bc1qexample...",
        lockedAmountSat: 100_000
    )

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin-with-address
}
