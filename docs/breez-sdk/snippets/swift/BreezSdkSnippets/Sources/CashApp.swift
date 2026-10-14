import BreezSdkSpark
import Foundation

func bridgeFromCashApp(sdk: BreezSdk) async throws {
    // ANCHOR: bridge-from-cash-app
    // Parse the recipient's external-chain address (EVM/Solana/Tron).
    let parsed = try await sdk.parse(input: "<recipient address>")
    guard case let .crossChainAddress(v1: addressDetails) = parsed else {
        throw NSError(domain: "CashApp", code: 1)
    }

    // List the stablecoin destinations Cash App can fund over Lightning and
    // pick one, e.g. USDC on Base.
    let routes = try await sdk.getCrossChainRoutes(
        filter: .send(
            addressDetails: addressDetails,
            deliveryMethod: .lightning
        ))
    guard let route = routes.first(where: { $0.asset == "USDC" && $0.chain == "base" }) else {
        throw NSError(domain: "CashApp", code: 2)
    }

    // Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
    // route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
    // about $10.
    let response = try await sdk.bridgeFromCashApp(
        request: BridgeFromCashAppRequest(
            address: addressDetails.address,
            route: route,
            amount: 10_000_000,
            feePolicy: nil,
            maxSlippageBps: nil
        ))

    // Open this Cash App URL to pay. The recipient then receives the stablecoin.
    print("Open this URL in Cash App: \(response.url)")
    print("Recipient receives ~\(response.estimatedOut) \(response.asset)")
    // ANCHOR_END: bridge-from-cash-app
}

func bridgeToCashApp(sdk: BreezSdk) async throws {
    // ANCHOR: bridge-to-cash-app
    // List the stablecoin sources that can pay a Cash App user over Lightning
    // and pick one, e.g. USDC on Base.
    let routes = try await sdk.getCrossChainRoutes(
        filter: .receive(
            contractAddress: nil,
            deliveryMethod: .lightning
        ))
    guard let route = routes.first(where: { $0.asset == "USDC" && $0.chain == "base" }) else {
        throw NSError(domain: "CashApp", code: 3)
    }

    // Pay $10 of USDC to the Cash App user $alice. The amount is in the route
    // asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
    // The deposit is refunded to the payer's address if delivery fails.
    let response = try await sdk.bridgeToCashApp(
        request: BridgeToCashAppRequest(
            recipient: "$alice",
            route: route,
            amount: 10_000_000,
            feePolicy: nil,
            refundAddress: "<payer address>",
            maxSlippageBps: nil
        ))

    // Show the payer what to pay. The recipient then receives Bitcoin.
    let info = response.info
    print("Pay with: \(response.paymentRequest)")
    print(
        "Deposit \(info.depositAmount) to \(info.depositAddress), "
            + "recipient receives ~\(info.expectedReceivedAmount) sats"
    )
    // ANCHOR_END: bridge-to-cash-app
}
