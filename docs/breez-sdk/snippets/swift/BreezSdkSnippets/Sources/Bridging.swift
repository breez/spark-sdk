import BigNumber
import BreezSdkSpark
import Foundation

func newClient() async throws -> BridgingClient {
    // ANCHOR: new-bridging-client
    let config = defaultBridgingConfig(apiKey: "<your api key>")
    let client = try await newBridgingClient(config: config)
    // ANCHOR_END: new-bridging-client
    return client
}

func clientFromSdk(sdk: BreezSdk) throws -> BridgingClient {
    // ANCHOR: get-bridging-client
    let client = try sdk.getBridgingClient()
    // ANCHOR_END: get-bridging-client
    return client
}

func createBridge(client: BridgingClient) async throws {
    // ANCHOR: create-bridge
    // Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
    // address.
    let recipient = "<recipient>"

    // List the routes that pay them and pick what the payer pays in, e.g.
    // USDC on Base.
    let routes = try await client.getRoutes(
        request: GetBridgeRoutesRequest(recipient: recipient)
    ).routes
    guard
        let route = routes.first(where: {
            $0.source.asset == "USDC" && $0.source.chain == "base"
        })
    else {
        throw NSError(domain: "Bridging", code: 1)
    }

    // Ask for $10. The amount is in the recipient's stablecoin when they are
    // paid in one, the payer's otherwise (6 decimals for USDC). The deposit is
    // refunded to the payer's address if the recipient can't be paid.
    let response = try await client.createBridge(
        request: CreateBridgeRequest(
            recipient: recipient,
            route: route,
            amount: BInt(10_000_000),
            // Required to pay a Lightning address.
            refundAddress: "<payer address on Base>",
            feePolicy: nil,
            maxSlippageBps: nil
        ))

    // Show the payer what to pay, and store the bridge id and status token to
    // look up the status later.
    print("Pay with: \(response.paymentRequest)")
    print("Bridge id: \(response.bridgeId)")
    // ANCHOR_END: create-bridge
}

func getBridgeStatus(client: BridgingClient, bridgeId: String, statusToken: String?) async throws {
    // ANCHOR: get-bridge-status
    let response = try await client.getBridgeStatus(
        request: GetBridgeStatusRequest(bridgeId: bridgeId, statusToken: statusToken))
    switch response.status {
    case .completed:
        print("Delivered \(response.amountOut.map { "\($0)" } ?? "unknown")")
    case .refunded, .failed, .expired:
        print("Not delivered: \(response.status)")
    default:
        print("Not final yet: \(response.status)")
    }
    // ANCHOR_END: get-bridge-status
}
