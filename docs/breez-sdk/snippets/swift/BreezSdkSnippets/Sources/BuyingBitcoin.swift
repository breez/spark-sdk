import BreezSdkSpark

func buyBitcoinBasic(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin-basic
    let request = BuyBitcoinRequest(
        address: "bc1qexample...",  // Your Bitcoin address
        lockedAmountSat: nil,
        maxAmountSat: nil,
        redirectUrl: nil
    )

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin-basic
}

func buyBitcoinWithAmount(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin-with-amount
    // Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
    let request = BuyBitcoinRequest(
        address: "bc1qexample...",
        lockedAmountSat: 100_000,  // Pre-fill with 100,000 sats
        maxAmountSat: nil,
        redirectUrl: nil
    )

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin-with-amount
}

func buyBitcoinWithLimits(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin-with-limits
    // Set both a locked amount and maximum amount
    let request = BuyBitcoinRequest(
        address: "bc1qexample...",
        lockedAmountSat: 50_000,   // Pre-fill with 50,000 sats
        maxAmountSat: 500_000,     // Limit to 500,000 sats max
        redirectUrl: nil
    )

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin-with-limits
}

func buyBitcoinWithRedirect(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin-with-redirect
    // Provide a custom redirect URL for after the purchase
    let request = BuyBitcoinRequest(
        address: "bc1qexample...",
        lockedAmountSat: 100_000,
        maxAmountSat: nil,
        redirectUrl: "https://example.com/purchase-complete"
    )

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin-with-redirect
}
