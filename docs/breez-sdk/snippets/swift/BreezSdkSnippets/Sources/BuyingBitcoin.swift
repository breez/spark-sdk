import BigNumber
import BreezSdkSpark

func buyBitcoin(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin
    // Optionally, prefill the purchase amount
    let optionalAmountSat: UInt64? = 100_000
    // Optionally, set a redirect URL for after the purchase is completed
    let optionalRedirectUrl: String? = "https://example.com/purchase-complete"

    let request = BuyBitcoinRequest.moonpay(
        delivery: .bitcoin(amountSat: optionalAmountSat),
        redirectUrl: optionalRedirectUrl
    )

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin
}

func buyBitcoinViaCrossChain(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin-cross-chain
    // USD amount to receive, in 6-decimal base units ($50)
    let amount = BInt(50_000_000)

    let request = BuyBitcoinRequest.moonpay(
        delivery: .crossChain(amount: amount, feeMode: nil),
        redirectUrl: nil
    )

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in a browser to complete the purchase:")
    print("\(response.url)")

    if let info = response.crossChainInfo {
        print("USDC to buy: \(info.depositAmount)")
        print(
            "Expected to receive: \(info.expectedReceivedAmount) "
                + "\(info.destinationAsset)"
        )
        print("Conversion fee: \(info.serviceFeeAmount)")
    }
    // ANCHOR_END: buy-bitcoin-cross-chain
}

func buyBitcoinViaCashapp(sdk: BreezSdk) async throws {
    // ANCHOR: buy-bitcoin-cashapp
    // Cash App requires the amount to be specified up front.
    let amountSats: UInt64 = 50_000

    let request = BuyBitcoinRequest.cashApp(amountSats: amountSats)

    let response = try await sdk.buyBitcoin(request: request)
    print("Open this URL in Cash App to complete the purchase:")
    print("\(response.url)")
    // ANCHOR_END: buy-bitcoin-cashapp
}
