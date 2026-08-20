import BreezSdkSpark
import Foundation

func preparePaymentLinkViaCashapp(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-payment-link-cashapp
    // Parse the recipient's external-chain address (EVM/Solana/Tron).
    let parsed = try await sdk.parse(input: "<recipient address>")
    guard case let .crossChainAddress(v1: addressDetails) = parsed else {
        throw NSError(domain: "PreparePaymentLink", code: 1)
    }

    // List the stablecoin destinations you can send to and pick one, e.g. USDC on Base.
    // Restrict to routes fundable over Lightning.
    let routes = try await sdk.getCrossChainRoutes(
        filter: .paymentLink(addressDetails: addressDetails))
    guard let route = routes.first(where: { $0.asset == "USDC" && $0.chain == "base" }) else {
        throw NSError(domain: "PreparePaymentLink", code: 2)
    }

    // Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
    // route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
    let response = try await sdk.preparePaymentLink(
        request: PreparePaymentLinkRequest(
            address: addressDetails.address,
            route: route,
            amount: 10_000_000,
            feePolicy: nil,
            maxSlippageBps: nil
        ))
    print("Open this URL in Cash App: \(response.url)")
    print("Recipient receives ~\(response.estimatedOut) \(response.asset)")
    // ANCHOR_END: prepare-payment-link-cashapp
}
