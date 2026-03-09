import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger
import org.jline.reader.LineReader

/**
 * Represents a single issuer subcommand.
 */
data class IssuerCliCommand(
    val name: String,
    val description: String,
    val run: suspend (issuer: TokenIssuer, reader: LineReader, args: List<String>) -> Unit
)

/**
 * All issuer subcommand names.
 */
val ISSUER_COMMAND_NAMES = listOf(
    "token-balance",
    "token-metadata",
    "create-token",
    "mint-token",
    "burn-token",
    "freeze-token",
    "unfreeze-token",
)

/**
 * Builds the issuer command registry.
 */
fun buildIssuerRegistry(): Map<String, IssuerCliCommand> {
    return mapOf(
        "token-balance" to IssuerCliCommand("token-balance", "Get issuer token balance", ::handleTokenBalance),
        "token-metadata" to IssuerCliCommand("token-metadata", "Get issuer token metadata", ::handleTokenMetadata),
        "create-token" to IssuerCliCommand("create-token", "Create a new issuer token", ::handleCreateToken),
        "mint-token" to IssuerCliCommand("mint-token", "Mint supply of the issuer token", ::handleMintToken),
        "burn-token" to IssuerCliCommand("burn-token", "Burn supply of the issuer token", ::handleBurnToken),
        "freeze-token" to IssuerCliCommand("freeze-token", "Freeze tokens at an address", ::handleFreezeToken),
        "unfreeze-token" to IssuerCliCommand("unfreeze-token", "Unfreeze tokens at an address", ::handleUnfreezeToken),
    )
}

/**
 * Dispatches an issuer subcommand.
 */
suspend fun dispatchIssuerCommand(
    args: List<String>,
    issuer: TokenIssuer,
    registry: Map<String, IssuerCliCommand>,
    reader: LineReader,
) {
    if (args.isEmpty() || args[0] == "help") {
        println()
        println("Issuer subcommands:")
        registry.keys.sorted().forEach { name ->
            val cmd = registry[name]!!
            println("  issuer %-30s %s".format(name, cmd.description))
        }
        println()
        return
    }

    val subName = args[0]
    val subArgs = args.drop(1)

    val cmd = registry[subName]
    if (cmd == null) {
        println("Unknown issuer subcommand: $subName. Use 'issuer help' for available commands.")
        return
    }

    try {
        cmd.run(issuer, reader, subArgs)
    } catch (e: Exception) {
        println("Error: ${e.message}")
    }
}

// ---------------------------------------------------------------------------
// Issuer command handlers
// ---------------------------------------------------------------------------

// --- token-balance ---

suspend fun handleTokenBalance(issuer: TokenIssuer, reader: LineReader, args: List<String>) {
    val result = issuer.getIssuerTokenBalance()
    printValue(result)
}

// --- token-metadata ---

suspend fun handleTokenMetadata(issuer: TokenIssuer, reader: LineReader, args: List<String>) {
    val result = issuer.getIssuerTokenMetadata()
    printValue(result)
}

// --- create-token ---

suspend fun handleCreateToken(issuer: TokenIssuer, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val name = fp.getString("name")
    val ticker = fp.getString("ticker")
    val decimals = fp.getUInt("decimals") ?: 6u
    val freezable = fp.hasFlag("freezable")
    val maxSupplyStr = fp.getString("max-supply")

    if (name == null || ticker == null) {
        println("Usage: issuer create-token --name <name> --ticker <ticker> [--decimals N] [--freezable] [--max-supply N]")
        return
    }

    val maxSupply = if (maxSupplyStr != null) {
        try {
            BigInteger.parseString(maxSupplyStr)
        } catch (e: Exception) {
            println("Invalid max-supply: $maxSupplyStr")
            return
        }
    } else {
        BigInteger.ZERO
    }

    val result = issuer.createIssuerToken(
        CreateIssuerTokenRequest(
            name = name,
            ticker = ticker,
            decimals = decimals,
            isFreezable = freezable,
            maxSupply = maxSupply,
        )
    )
    printValue(result)
}

// --- mint-token ---

suspend fun handleMintToken(issuer: TokenIssuer, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: issuer mint-token <amount>")
        return
    }

    val amount = try {
        BigInteger.parseString(args[0])
    } catch (e: Exception) {
        println("Invalid amount: ${args[0]}")
        return
    }

    val result = issuer.mintIssuerToken(MintIssuerTokenRequest(amount = amount))
    printValue(result)
}

// --- burn-token ---

suspend fun handleBurnToken(issuer: TokenIssuer, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: issuer burn-token <amount>")
        return
    }

    val amount = try {
        BigInteger.parseString(args[0])
    } catch (e: Exception) {
        println("Invalid amount: ${args[0]}")
        return
    }

    val result = issuer.burnIssuerToken(BurnIssuerTokenRequest(amount = amount))
    printValue(result)
}

// --- freeze-token ---

suspend fun handleFreezeToken(issuer: TokenIssuer, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: issuer freeze-token <address>")
        return
    }

    val result = issuer.freezeIssuerToken(FreezeIssuerTokenRequest(address = args[0]))
    printValue(result)
}

// --- unfreeze-token ---

suspend fun handleUnfreezeToken(issuer: TokenIssuer, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: issuer unfreeze-token <address>")
        return
    }

    val result = issuer.unfreezeIssuerToken(UnfreezeIssuerTokenRequest(address = args[0]))
    printValue(result)
}
