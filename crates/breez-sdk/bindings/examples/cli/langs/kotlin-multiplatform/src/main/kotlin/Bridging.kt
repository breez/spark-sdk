import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger
import org.jline.reader.LineReader

/**
 * Represents a single bridging subcommand.
 */
data class BridgingCliCommand(
    val name: String,
    val description: String,
    val run: suspend (sdk: BreezSdk, reader: LineReader, args: List<String>) -> Unit
)

/**
 * All bridging subcommand names.
 */
val BRIDGING_COMMAND_NAMES = listOf(
    "create",
    "from-cash-app",
    "to-cash-app",
    "status",
)

/**
 * Builds the bridging command registry.
 */
fun buildBridgingRegistry(): Map<String, BridgingCliCommand> {
    return mapOf(
        "create" to BridgingCliCommand("create", "Create a bridge that pays a recipient from another chain or over Lightning", ::handleBridgingCreate),
        "from-cash-app" to BridgingCliCommand("from-cash-app", "Bridge from Cash App to USDC/USDT on an external chain", ::handleBridgingFromCashApp),
        "to-cash-app" to BridgingCliCommand("to-cash-app", "Bridge USDC/USDT from an external chain to a Cash App user", ::handleBridgingToCashApp),
        "status" to BridgingCliCommand("status", "Read a bridge's status", ::handleBridgingStatus),
    )
}

/**
 * Dispatches a bridging subcommand.
 */
suspend fun dispatchBridgingCommand(
    args: List<String>,
    sdk: BreezSdk,
    registry: Map<String, BridgingCliCommand>,
    reader: LineReader,
) {
    if (args.isEmpty() || args[0] == "help") {
        println()
        println("Bridging subcommands:")
        registry.keys.sorted().forEach { name ->
            val cmd = registry[name]!!
            println("  bridging %-26s %s".format(name, cmd.description))
        }
        println()
        return
    }

    val subName = args[0]
    val subArgs = args.drop(1)

    val cmd = registry[subName]
    if (cmd == null) {
        println("Unknown bridging subcommand: $subName. Use 'bridging help' for available commands.")
        return
    }

    try {
        cmd.run(sdk, reader, subArgs)
    } catch (e: Exception) {
        println("Error: ${e.message}")
    }
}

// ---------------------------------------------------------------------------
// Bridging command handlers
// ---------------------------------------------------------------------------

// --- create ---

suspend fun handleBridgingCreate(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val amountStr = fp.getString("amount")
    val refundAddress = fp.getString("refund-address")
    val feesIncluded = fp.hasFlag("fees-included")
    val maxSlippageBps = fp.getUInt("max-slippage-bps")

    if (fp.positional.isEmpty() || amountStr == null) {
        println("Usage: bridging create <recipient> --amount <amount> [--refund-address <address>] [--fees-included] [--max-slippage-bps <bps>]")
        return
    }

    val recipient = fp.positional[0]
    val amount = try {
        BigInteger.parseString(amountStr)
    } catch (e: Exception) {
        println("Invalid amount: $amountStr")
        return
    }

    val client = sdk.getBridgingClient()
    val routes = client.getRoutes(GetBridgeRoutesRequest(recipient = recipient)).routes
    val route = chooseBridgeRoute(reader, routes)
    val feePolicy = if (feesIncluded) FeePolicy.FEES_INCLUDED else null

    val response = client.createBridge(
        CreateBridgeRequest(
            recipient = recipient,
            route = route,
            amount = amount,
            refundAddress = refundAddress,
            feePolicy = feePolicy,
            maxSlippageBps = maxSlippageBps,
        )
    )
    println("Ask the payer to pay:")
    println(response.paymentRequest)
    printValue(response)
    printBridgeTracking(response)
}

// --- from-cash-app ---

suspend fun handleBridgingFromCashApp(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val amountStr = fp.getString("amount")
    val feesIncluded = fp.hasFlag("fees-included")
    val maxSlippageBps = fp.getUInt("max-slippage-bps")

    if (fp.positional.isEmpty() || amountStr == null) {
        println("Usage: bridging from-cash-app <recipient> --amount <amount> [--fees-included] [--max-slippage-bps <bps>]")
        return
    }

    val recipient = fp.positional[0]
    val amount = try {
        BigInteger.parseString(amountStr)
    } catch (e: Exception) {
        println("Invalid amount: $amountStr")
        return
    }

    val client = sdk.getBridgingClient()
    val routes = client.getRoutes(GetBridgeRoutesRequest(recipient = recipient)).routes
        .filter { it.source.chain == "lightning" }
    val route = chooseBridgeRoute(reader, routes)
    val feePolicy = if (feesIncluded) FeePolicy.FEES_INCLUDED else null

    val response = client.bridgeFromCashApp(
        CreateBridgeRequest(
            recipient = recipient,
            route = route,
            amount = amount,
            refundAddress = null,
            feePolicy = feePolicy,
            maxSlippageBps = maxSlippageBps,
        )
    )
    val info = response.info
    println("Open this URL in a browser to complete the purchase:")
    println(response.paymentRequest)
    println(
        "Deposit ~${info.depositAmount} sats; recipient receives ~${info.expectedReceivedAmount} ${info.destinationAsset}, " +
        "service fee ${info.serviceFeeAmount} ${serviceFeeDenomination(info.serviceFeeAsset, info.serviceFeeAssetDecimals)}, " +
        "expires ${info.expiresAt}"
    )
    printBridgeTracking(response)
}

// --- to-cash-app ---

suspend fun handleBridgingToCashApp(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val amountStr = fp.getString("amount")
    val refundAddress = fp.getString("refund-address")
    val feesIncluded = fp.hasFlag("fees-included")
    val maxSlippageBps = fp.getUInt("max-slippage-bps")

    if (fp.positional.isEmpty() || amountStr == null || refundAddress == null) {
        println("Usage: bridging to-cash-app <recipient> --amount <amount> --refund-address <address> [--fees-included] [--max-slippage-bps <bps>]")
        return
    }

    val recipient = fp.positional[0]
    val amount = try {
        BigInteger.parseString(amountStr)
    } catch (e: Exception) {
        println("Invalid amount: $amountStr")
        return
    }

    val client = sdk.getBridgingClient()
    val routes = client.getRoutes(GetBridgeRoutesRequest(recipient = recipient)).routes
    val route = chooseBridgeRoute(reader, routes)
    val feePolicy = if (feesIncluded) FeePolicy.FEES_INCLUDED else null

    val response = client.bridgeToCashApp(
        CreateBridgeRequest(
            recipient = recipient,
            route = route,
            amount = amount,
            refundAddress = refundAddress,
            feePolicy = feePolicy,
            maxSlippageBps = maxSlippageBps,
        )
    )
    val info = response.info
    println("Ask the payer to pay:")
    println(response.paymentRequest)
    println(
        "Deposit ${info.depositAmount} ${route.source.asset} on ${route.source.chain} to ${info.depositAddress}. " +
        "Recipient receives ~${info.expectedReceivedAmount} sats, " +
        "service fee ${info.serviceFeeAmount} ${serviceFeeDenomination(info.serviceFeeAsset, info.serviceFeeAssetDecimals)}, " +
        "expires at ${info.expiresAt}"
    )
    printBridgeTracking(response)
}

// --- status ---

suspend fun handleBridgingStatus(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val statusToken = fp.getString("status-token")

    if (fp.positional.isEmpty()) {
        println("Usage: bridging status <bridge_id> [--status-token <token>]")
        return
    }

    val bridgeId = fp.positional[0]
    val client = sdk.getBridgingClient()
    val response = client.getBridgeStatus(
        GetBridgeStatusRequest(
            bridgeId = bridgeId,
            statusToken = statusToken,
        )
    )
    printValue(response)
}

// ---------------------------------------------------------------------------
// Bridge route selection
// ---------------------------------------------------------------------------

fun chooseBridgeRoute(reader: LineReader, routes: List<BridgeRoutePair>): BridgeRoutePair {
    if (routes.isEmpty()) {
        throw Exception("No bridge routes available")
    }

    if (routes.size == 1) {
        val route = routes[0]
        println("Auto-selected route: ${bridgeRouteLabel(route)}")
        return route
    }

    println("Available bridge routes:")
    for ((i, route) in routes.withIndex()) {
        val idx = i + 1
        println("  $idx. ${bridgeRouteLabel(route)}")
    }

    val line = readlinePrompt(reader, "Select route: ")
    val choice = line.toIntOrNull() ?: throw Exception("Invalid selection")
    val index = choice - 1
    if (index < 0 || index >= routes.size) {
        throw Exception("Selection out of range")
    }
    return routes[index]
}

fun bridgeRouteLabel(route: BridgeRoutePair): String {
    return "${bridgeSideLabel(route.source)} -> ${bridgeSideLabel(route.destination)} [${route.provider}]"
}

fun bridgeSideLabel(side: BridgeRoutePairSide): String {
    return "${side.asset} on ${side.chain}${maybeTruncateAddress(side.contractAddress)}"
}

fun printBridgeTracking(response: CreateBridgeResponse) {
    val token = response.statusToken?.let { " --status-token $it" } ?: ""
    println("Track it with: bridging status ${response.bridgeId}$token")
}
