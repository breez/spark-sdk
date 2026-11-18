import BigNumber
import BreezSdkSpark
import Foundation

func bridgeFromCashApp(client: BridgingClient) async throws {
    // ANCHOR: bridge-from-cash-app
    // List the routes to the recipient's external-chain address
    // (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
    // Lightning, e.g. USDC on Base.
    let recipient = "<recipient address>"
    let routes = try await client.getRoutes(
        request: GetBridgeRoutesRequest(recipient: recipient)
    ).routes
    guard
        let route = routes.first(where: {
            $0.source.chain == "lightning"
                && $0.destination.asset == "USDC"
                && $0.destination.chain == "base"
        })
    else {
        throw NSError(domain: "CashApp", code: 1)
    }

    // Send $10 of USDC, funded by Cash App over Lightning. The amount is in
    // USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
    let response = try await client.bridgeFromCashApp(
        request: CreateBridgeRequest(
            recipient: recipient,
            route: route,
            amount: BInt(10_000_000),
            refundAddress: nil,
            feePolicy: nil,
            maxSlippageBps: nil
        ))

    // Open this Cash App link to pay. The recipient then receives the stablecoin.
    let info = response.info
    print("Open this URL in Cash App: \(response.paymentRequest)")
    print("Recipient receives ~\(info.expectedReceivedAmount) \(info.destinationAsset)")
    // ANCHOR_END: bridge-from-cash-app
}

func bridgeToCashApp(client: BridgingClient) async throws {
    // ANCHOR: bridge-to-cash-app
    // List the routes to the Cash App user and pick what the payer pays in,
    // e.g. USDC on Base.
    let recipient = "$alice"
    let routes = try await client.getRoutes(
        request: GetBridgeRoutesRequest(recipient: recipient)
    ).routes
    guard
        let route = routes.first(where: {
            $0.source.asset == "USDC" && $0.source.chain == "base"
        })
    else {
        throw NSError(domain: "CashApp", code: 2)
    }

    // Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
    // units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
    // refunded to the payer's address if delivery fails.
    let response = try await client.bridgeToCashApp(
        request: CreateBridgeRequest(
            recipient: recipient,
            route: route,
            amount: BInt(10_000_000),
            refundAddress: "<payer address>",
            feePolicy: nil,
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
