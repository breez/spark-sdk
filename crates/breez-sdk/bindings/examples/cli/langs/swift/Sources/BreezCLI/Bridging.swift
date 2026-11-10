import Foundation
import BreezSdkSpark
import BigNumber

// MARK: - Bridging command names (for REPL completion)

let bridgingCommandNames: [String] = [
    "bridging create",
    "bridging from-cash-app",
    "bridging to-cash-app",
    "bridging status",
]

// MARK: - Dispatch

func dispatchBridgingCommand(_ args: [String], sdk: BreezSdk) async {
    if args.isEmpty || args[0] == "help" {
        printBridgingHelp()
        return
    }

    let subName = args[0]
    let subArgs = Array(args.dropFirst())

    do {
        switch subName {
        case "create":
            try await handleBridgingCreate(sdk, subArgs)
        case "from-cash-app":
            try await handleBridgingFromCashApp(sdk, subArgs)
        case "to-cash-app":
            try await handleBridgingToCashApp(sdk, subArgs)
        case "status":
            try await handleBridgingStatus(sdk, subArgs)
        default:
            print("Unknown bridging subcommand: \(subName). Use 'bridging help' for available commands.")
        }
    } catch {
        print("Error: \(error)")
    }
}

// MARK: - Help

func printBridgingHelp() {
    print("\nBridging commands:")
    print("  \("create".padding(toLength: 30, withPad: " ", startingAt: 0))Create a bridge that pays a recipient from another chain or over Lightning")
    print("  \("from-cash-app".padding(toLength: 30, withPad: " ", startingAt: 0))Bridge from Cash App to USDC/USDT on an external chain")
    print("  \("to-cash-app".padding(toLength: 30, withPad: " ", startingAt: 0))Bridge USDC/USDT from an external chain to a Cash App user")
    print("  \("status".padding(toLength: 30, withPad: " ", startingAt: 0))Read a bridge's status")
    print()
}

// MARK: - Command handlers

// --- bridging create ---

func handleBridgingCreate(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let recipient = fp.positional.first,
          let amountStr = fp.get("amount"),
          let amount = BInt(amountStr) else {
        print("Usage: bridging create <recipient> --amount <amount> [--refund-address <address>] [--fees-included] [--max-slippage-bps <bps>]")
        return
    }

    let refundAddress = fp.get("refund-address")
    let feesIncluded = fp.has("fees-included")
    let maxSlippageBps = fp.get("max-slippage-bps").flatMap { UInt32($0) }

    let client = try sdk.getBridgingClient()
    let routes = try await client.getRoutes(
        request: GetBridgeRoutesRequest(recipient: recipient)
    ).routes
    let route = try chooseBridgeRoute(routes)
    let feePolicy: FeePolicy? = feesIncluded ? .feesIncluded : nil

    let response = try await client.createBridge(
        request: CreateBridgeRequest(
            recipient: recipient,
            route: route,
            amount: amount,
            refundAddress: refundAddress,
            feePolicy: feePolicy,
            maxSlippageBps: maxSlippageBps
        ))

    print("Ask the payer to pay:")
    print(response.paymentRequest)
    printValue(response)
    printBridgeTracking(response)
}

// --- bridging from-cash-app ---

func handleBridgingFromCashApp(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let recipient = fp.positional.first,
          let amountStr = fp.get("amount"),
          let amount = BInt(amountStr) else {
        print("Usage: bridging from-cash-app <recipient> --amount <amount> [--refund-address <address>] [--fees-included] [--max-slippage-bps <bps>]")
        return
    }

    let refundAddress = fp.get("refund-address")
    let feesIncluded = fp.has("fees-included")
    let maxSlippageBps = fp.get("max-slippage-bps").flatMap { UInt32($0) }

    let client = try sdk.getBridgingClient()
    let routes = try await client.getRoutes(
        request: GetBridgeRoutesRequest(recipient: recipient)
    ).routes.filter { $0.source.chain == "lightning" }
    let route = try chooseBridgeRoute(routes)
    let feePolicy: FeePolicy? = feesIncluded ? .feesIncluded : nil

    let response = try await client.bridgeFromCashApp(
        request: CreateBridgeRequest(
            recipient: recipient,
            route: route,
            amount: amount,
            refundAddress: refundAddress,
            feePolicy: feePolicy,
            maxSlippageBps: maxSlippageBps
        ))

    let info = response.info
    print("Open this URL in a browser to complete the purchase:")
    print(response.paymentRequest)
    let serviceFeeDenom = serviceFeeDenomination(
        asset: info.serviceFeeAsset,
        decimals: info.serviceFeeAssetDecimals
    )
    print("Deposit ~\(info.depositAmount) sats; recipient receives ~\(info.expectedReceivedAmount) \(info.destinationAsset), service fee \(info.serviceFeeAmount) \(serviceFeeDenom), expires \(info.expiresAt)")
    printBridgeTracking(response)
}

// --- bridging to-cash-app ---

func handleBridgingToCashApp(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let recipient = fp.positional.first,
          let amountStr = fp.get("amount"),
          let amount = BInt(amountStr) else {
        print("Usage: bridging to-cash-app <recipient> --amount <amount> --refund-address <address> [--fees-included] [--max-slippage-bps <bps>]")
        return
    }

    guard let refundAddress = fp.get("refund-address") else {
        print("--refund-address is required")
        return
    }

    let feesIncluded = fp.has("fees-included")
    let maxSlippageBps = fp.get("max-slippage-bps").flatMap { UInt32($0) }

    let client = try sdk.getBridgingClient()
    let routes = try await client.getRoutes(
        request: GetBridgeRoutesRequest(recipient: recipient)
    ).routes
    let route = try chooseBridgeRoute(routes)
    let feePolicy: FeePolicy? = feesIncluded ? .feesIncluded : nil

    let response = try await client.bridgeToCashApp(
        request: CreateBridgeRequest(
            recipient: recipient,
            route: route,
            amount: amount,
            refundAddress: refundAddress,
            feePolicy: feePolicy,
            maxSlippageBps: maxSlippageBps
        ))

    let info = response.info
    print("Ask the payer to pay:")
    print(response.paymentRequest)
    let serviceFeeDenom = serviceFeeDenomination(
        asset: info.serviceFeeAsset,
        decimals: info.serviceFeeAssetDecimals
    )
    print("Deposit \(info.depositAmount) \(route.source.asset) on \(route.source.chain) to \(info.depositAddress). Recipient receives ~\(info.expectedReceivedAmount) sats, service fee \(info.serviceFeeAmount) \(serviceFeeDenom), expires at \(info.expiresAt)")
    printBridgeTracking(response)
}

// --- bridging status ---

func handleBridgingStatus(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let bridgeId = fp.positional.first else {
        print("Usage: bridging status <bridge_id> [--status-token <token>]")
        return
    }

    let statusToken = fp.get("status-token")

    let client = try sdk.getBridgingClient()
    let response = try await client.getBridgeStatus(
        request: GetBridgeStatusRequest(bridgeId: bridgeId, statusToken: statusToken))
    printValue(response)
}

// MARK: - Bridge route selection

func chooseBridgeRoute(_ routes: [BridgeRoutePair]) throws -> BridgeRoutePair {
    if routes.isEmpty {
        throw NSError(domain: "BreezCLI", code: 1, userInfo: [
            NSLocalizedDescriptionKey: "No bridge routes available",
        ])
    }

    if routes.count == 1 {
        let route = routes[0]
        print("Auto-selected route: \(bridgeRouteLabel(route))")
        return route
    }

    print("Available bridge routes:")
    for (i, route) in routes.enumerated() {
        let idx = i + 1
        print("  \(idx). \(bridgeRouteLabel(route))")
    }

    guard let line = readlinePrompt("Select route: "),
          let choice = Int(line.trimmingCharacters(in: .whitespaces)),
          choice >= 1, choice <= routes.count else {
        throw NSError(domain: "BreezCLI", code: 1, userInfo: [
            NSLocalizedDescriptionKey: "Invalid selection",
        ])
    }

    return routes[choice - 1]
}

// MARK: - Helpers

private func bridgeRouteLabel(_ route: BridgeRoutePair) -> String {
    "\(bridgeSideLabel(route.source)) -> \(bridgeSideLabel(route.destination)) [\(route.provider)]"
}

private func bridgeSideLabel(_ side: BridgeRoutePairSide) -> String {
    "\(side.asset) on \(side.chain)\(maybeTruncateAddress(side.contractAddress))"
}

private func printBridgeTracking(_ response: CreateBridgeResponse) {
    var cmd = "bridging status \(response.bridgeId)"
    if let token = response.statusToken {
        cmd += " --status-token \(token)"
    }
    print("Track it with: \(cmd)")
}
