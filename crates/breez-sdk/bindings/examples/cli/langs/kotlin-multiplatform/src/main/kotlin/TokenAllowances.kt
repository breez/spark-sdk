import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger
import org.jline.reader.LineReader

data class TokenAllowancesCliCommand(
    val name: String,
    val description: String,
    val run: suspend (sdk: BreezSdk, reader: LineReader, args: List<String>) -> Unit
)

val TOKEN_ALLOWANCES_COMMAND_NAMES = listOf(
    "create",
    "revoke",
    "list",
)

fun buildTokenAllowancesRegistry(): Map<String, TokenAllowancesCliCommand> {
    return mapOf(
        "create" to TokenAllowancesCliCommand("create", "Grant a spender an allowance over this wallet's tokens", ::handleCreateTokenAllowance),
        "revoke" to TokenAllowancesCliCommand("revoke", "Revoke a token allowance this wallet granted", ::handleRevokeTokenAllowance),
        "list" to TokenAllowancesCliCommand("list", "List token allowances this wallet granted or received", ::handleListTokenAllowances),
    )
}

suspend fun dispatchTokenAllowancesCommand(
    args: List<String>,
    sdk: BreezSdk,
    registry: Map<String, TokenAllowancesCliCommand>,
    reader: LineReader,
) {
    if (args.isEmpty() || args[0] == "help") {
        println()
        println("Token allowance subcommands:")
        registry.keys.sorted().forEach { name ->
            val cmd = registry[name]!!
            println("  token-allowances %-20s %s".format(name, cmd.description))
        }
        println()
        return
    }

    val subName = args[0]
    val subArgs = args.drop(1)

    val cmd = registry[subName]
    if (cmd == null) {
        println("Unknown token-allowances subcommand: $subName. Use 'token-allowances help' for available commands.")
        return
    }

    try {
        cmd.run(sdk, reader, subArgs)
    } catch (e: Exception) {
        println("Error: ${e.message}")
    }
}

// ---------------------------------------------------------------------------
// Token allowance command handlers
// ---------------------------------------------------------------------------

private fun parseAllowanceLimit(s: String): TokenAllowanceLimit {
    if (s.equals("unlimited", ignoreCase = true)) {
        return TokenAllowanceLimit.Unlimited
    }
    val amount = try {
        BigInteger.parseString(s)
    } catch (e: Exception) {
        throw IllegalArgumentException("Invalid limit '$s': expected an amount or 'unlimited'")
    }
    return TokenAllowanceLimit.Amount(amount = amount)
}

// --- create ---

suspend fun handleCreateTokenAllowance(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val maxPerPaymentStr = fp.getString("max-per-payment")
    val maxTotalStr = fp.getString("max-total")
    val allowedRecipients = fp.getAll("r", "recipient")

    if (fp.positional.size < 3 || maxPerPaymentStr == null || maxTotalStr == null) {
        println("Usage: token-allowances create <spender_address> <token_identifier> <expiry_time> --max-per-payment <limit> --max-total <limit> [-r <recipient> ...]")
        println("  limit: an amount in token base units, or 'unlimited'")
        return
    }

    val spenderAddress = fp.positional[0]
    val tokenIdentifier = fp.positional[1]
    val expiryTime = fp.positional[2].toULongOrNull()
    if (expiryTime == null) {
        println("Invalid expiry_time: ${fp.positional[2]}")
        return
    }

    val maxPerPayment = try {
        parseAllowanceLimit(maxPerPaymentStr)
    } catch (e: Exception) {
        println("Error: ${e.message}")
        return
    }
    val maxTotal = try {
        parseAllowanceLimit(maxTotalStr)
    } catch (e: Exception) {
        println("Error: ${e.message}")
        return
    }

    val response = sdk.createTokenAllowance(
        CreateTokenAllowanceRequest(
            spenderAddress = spenderAddress,
            tokenIdentifier = tokenIdentifier,
            maxPerPayment = maxPerPayment,
            maxTotal = maxTotal,
            expiryTime = expiryTime,
            allowedRecipients = allowedRecipients,
        )
    )
    printValue(response.allowance)
}

// --- revoke ---

suspend fun handleRevokeTokenAllowance(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: token-allowances revoke <allowance_id>")
        return
    }

    sdk.revokeTokenAllowance(RevokeTokenAllowanceRequest(allowanceId = args[0]))
    println("Token allowance revoked")
}

// --- list ---

suspend fun handleListTokenAllowances(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val roleStr = fp.positional.firstOrNull() ?: "owner"
    val counterpartyAddress = fp.getString("counterparty-address")
    val tokenIdentifier = fp.getString("token-identifier")
    val includeInactive = fp.getBool("include-inactive")
    val offset = fp.getUInt("offset")
    val limit = fp.getUInt("limit")

    val role = when (roleStr.lowercase()) {
        "owner" -> TokenAllowanceRole.OWNER
        "spender" -> TokenAllowanceRole.SPENDER
        else -> {
            println("Invalid role '$roleStr': expected 'owner' or 'spender'")
            return
        }
    }

    val response = sdk.listTokenAllowances(
        ListTokenAllowancesRequest(
            role = role,
            counterpartyAddress = counterpartyAddress,
            tokenIdentifier = tokenIdentifier,
            includeInactive = includeInactive,
            offset = offset,
            limit = limit,
        )
    )
    printValue(response.allowances)
}
