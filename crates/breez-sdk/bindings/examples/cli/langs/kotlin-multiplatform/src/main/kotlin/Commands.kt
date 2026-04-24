import breez_sdk_spark.*
import com.ionspin.kotlin.bignum.integer.BigInteger
import org.jline.reader.LineReader

/**
 * Represents a single CLI command.
 */
data class CliCommand(
    val name: String,
    val description: String,
    val run: suspend (sdk: BreezSdk, reader: LineReader, args: List<String>) -> Unit
)

/**
 * All top-level command names (used for help display).
 */
val COMMAND_NAMES = listOf(
    "get-info",
    "get-payment",
    "sync",
    "list-payments",
    "receive",
    "pay",
    "lnurl-pay",
    "lnurl-withdraw",
    "lnurl-auth",
    "claim-htlc-payment",
    "claim-deposit",
    "parse",
    "refund-deposit",
    "list-unclaimed-deposits",
    "buy-bitcoin",
    "check-lightning-address-available",
    "get-lightning-address",
    "register-lightning-address",
    "delete-lightning-address",
    "list-fiat-currencies",
    "list-fiat-rates",
    "recommended-fees",
    "get-tokens-metadata",
    "fetch-conversion-limits",
    "get-user-settings",
    "set-user-settings",
    "get-spark-status",
)

/**
 * Builds the command registry mapping command names to their handlers.
 */
fun buildCommandRegistry(): Map<String, CliCommand> {
    return mapOf(
        "get-info" to CliCommand("get-info", "Get balance information", ::handleGetInfo),
        "get-payment" to CliCommand("get-payment", "Get the payment with the given ID", ::handleGetPayment),
        "sync" to CliCommand("sync", "Sync wallet state", ::handleSync),
        "list-payments" to CliCommand("list-payments", "List payments", ::handleListPayments),
        "receive" to CliCommand("receive", "Receive a payment", ::handleReceive),
        "pay" to CliCommand("pay", "Pay the given payment request", ::handlePay),
        "lnurl-pay" to CliCommand("lnurl-pay", "Pay using LNURL", ::handleLnurlPay),
        "lnurl-withdraw" to CliCommand("lnurl-withdraw", "Withdraw using LNURL", ::handleLnurlWithdraw),
        "lnurl-auth" to CliCommand("lnurl-auth", "Authenticate using LNURL", ::handleLnurlAuth),
        "claim-htlc-payment" to CliCommand("claim-htlc-payment", "Claim an HTLC payment", ::handleClaimHtlcPayment),
        "claim-deposit" to CliCommand("claim-deposit", "Claim an on-chain deposit", ::handleClaimDeposit),
        "parse" to CliCommand("parse", "Parse an input (invoice, address, LNURL)", ::handleParse),
        "refund-deposit" to CliCommand("refund-deposit", "Refund an on-chain deposit", ::handleRefundDeposit),
        "list-unclaimed-deposits" to CliCommand("list-unclaimed-deposits", "List unclaimed on-chain deposits", ::handleListUnclaimedDeposits),
        "buy-bitcoin" to CliCommand("buy-bitcoin", "Buy Bitcoin via MoonPay", ::handleBuyBitcoin),
        "check-lightning-address-available" to CliCommand("check-lightning-address-available", "Check if a lightning address username is available", ::handleCheckLightningAddress),
        "get-lightning-address" to CliCommand("get-lightning-address", "Get registered lightning address", ::handleGetLightningAddress),
        "register-lightning-address" to CliCommand("register-lightning-address", "Register a lightning address", ::handleRegisterLightningAddress),
        "delete-lightning-address" to CliCommand("delete-lightning-address", "Delete lightning address", ::handleDeleteLightningAddress),
        "list-fiat-currencies" to CliCommand("list-fiat-currencies", "List fiat currencies", ::handleListFiatCurrencies),
        "list-fiat-rates" to CliCommand("list-fiat-rates", "List available fiat rates", ::handleListFiatRates),
        "recommended-fees" to CliCommand("recommended-fees", "Get recommended BTC fees", ::handleRecommendedFees),
        "get-tokens-metadata" to CliCommand("get-tokens-metadata", "Get metadata for token(s)", ::handleGetTokensMetadata),
        "fetch-conversion-limits" to CliCommand("fetch-conversion-limits", "Fetch conversion limits for a token", ::handleFetchConversionLimits),
        "get-user-settings" to CliCommand("get-user-settings", "Get user settings", ::handleGetUserSettings),
        "set-user-settings" to CliCommand("set-user-settings", "Update user settings", ::handleSetUserSettings),
        "get-spark-status" to CliCommand("get-spark-status", "Get Spark network service status", ::handleGetSparkStatus),
    )
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/**
 * Reads a line from the LineReader with a prompt.
 */
fun readlinePrompt(reader: LineReader, prompt: String): String {
    return reader.readLine(prompt).trim()
}

/**
 * Reads a line with a default value pre-filled.
 */
fun readlineWithDefault(reader: LineReader, prompt: String, defaultVal: String): String {
    val line = reader.readLine(prompt).trim()
    return if (line.isEmpty()) defaultVal else line
}

/**
 * Simple flag parser. Extracts named flags from args and returns remaining positional args.
 */
class FlagParser(args: List<String>) {
    private val flags = mutableMapOf<String, String?>()
    val positional = mutableListOf<String>()

    init {
        var i = 0
        while (i < args.size) {
            val arg = args[i]
            if (arg.startsWith("--")) {
                val key = arg.substring(2)
                // Check if next arg is a value (not another flag)
                if (i + 1 < args.size && !args[i + 1].startsWith("--")) {
                    flags[key] = args[i + 1]
                    i += 2
                } else {
                    flags[key] = null // Boolean flag
                    i++
                }
            } else if (arg.startsWith("-") && arg.length == 2) {
                val key = arg.substring(1)
                if (i + 1 < args.size && !args[i + 1].startsWith("-")) {
                    flags[key] = args[i + 1]
                    i += 2
                } else {
                    flags[key] = null
                    i++
                }
            } else {
                positional.add(arg)
                i++
            }
        }
    }

    fun getString(vararg names: String): String? {
        for (name in names) {
            flags[name]?.let { return it }
        }
        return null
    }

    fun getUInt(vararg names: String): UInt? = getString(*names)?.toUIntOrNull()

    fun getULong(vararg names: String): ULong? = getString(*names)?.toULongOrNull()

    fun getBigInteger(vararg names: String): BigInteger? {
        val s = getString(*names) ?: return null
        return try {
            BigInteger.parseString(s)
        } catch (e: Exception) {
            null
        }
    }

    fun hasFlag(vararg names: String): Boolean {
        return names.any { it in flags }
    }

    fun getBool(vararg names: String): Boolean? {
        for (name in names) {
            if (name in flags) {
                val v = flags[name]
                return if (v == null) true else v.lowercase() == "true"
            }
        }
        return null
    }
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

// --- get-info ---

suspend fun handleGetInfo(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val ensureSynced = fp.getBool("ensure-synced", "e")
    val result = sdk.getInfo(GetInfoRequest(ensureSynced))
    printValue(result)
}

// --- get-payment ---

suspend fun handleGetPayment(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: get-payment <payment_id>")
        return
    }
    val result = sdk.getPayment(GetPaymentRequest(paymentId = args[0]))
    printValue(result)
}

// --- sync ---

suspend fun handleSync(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val result = sdk.syncWallet(SyncWalletRequest)
    printValue(result)
}

// --- list-payments ---

suspend fun handleListPayments(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val limit = fp.getUInt("l", "limit") ?: 10u
    val offset = fp.getUInt("o", "offset") ?: 0u
    val sortAscending = fp.getBool("sort-ascending")
    val fromTimestamp = fp.getULong("from-timestamp")
    val toTimestamp = fp.getULong("to-timestamp")
    val typeFilterStr = fp.getString("t", "type-filter")
    val statusFilterStr = fp.getString("s", "status-filter")
    val assetFilterStr = fp.getString("a", "asset-filter")
    val htlcStatusStr = fp.getString("spark-htlc-status-filter")
    val txHash = fp.getString("tx-hash")
    val txTypeStr = fp.getString("tx-type")

    // Parse type filter (comma-separated)
    val typeFilter = typeFilterStr?.split(",")?.map { s ->
        PaymentType.valueOf(s.trim().uppercase())
    }

    // Parse status filter (comma-separated)
    val statusFilter = statusFilterStr?.split(",")?.map { s ->
        PaymentStatus.valueOf(s.trim().uppercase())
    }

    // Parse asset filter
    val assetFilter = when (assetFilterStr?.lowercase()) {
        "bitcoin" -> AssetFilter.Bitcoin
        null -> null
        else -> AssetFilter.Token(tokenIdentifier = assetFilterStr)
    }

    // Build payment details filters
    val paymentDetailsFilter = mutableListOf<PaymentDetailsFilter>()
    if (htlcStatusStr != null) {
        val statuses = htlcStatusStr.split(",").map { s ->
            SparkHtlcStatus.valueOf(s.trim().uppercase())
        }
        paymentDetailsFilter.add(PaymentDetailsFilter.Spark(
            htlcStatus = statuses,
            conversionRefundNeeded = null
        ))
    }
    if (txHash != null) {
        paymentDetailsFilter.add(PaymentDetailsFilter.Token(
            conversionRefundNeeded = null,
            txType = null,
            txHash = txHash
        ))
    }
    if (txTypeStr != null) {
        val txType = TokenTransactionType.valueOf(txTypeStr.trim().uppercase())
        paymentDetailsFilter.add(PaymentDetailsFilter.Token(
            conversionRefundNeeded = null,
            txType = txType,
            txHash = null
        ))
    }

    val result = sdk.listPayments(
        ListPaymentsRequest(
            limit = limit,
            offset = offset,
            typeFilter = typeFilter,
            statusFilter = statusFilter,
            assetFilter = assetFilter,
            paymentDetailsFilter = paymentDetailsFilter.ifEmpty { null },
            fromTimestamp = fromTimestamp,
            toTimestamp = toTimestamp,
            sortAscending = sortAscending,
        )
    )
    printValue(result)
}

// --- receive ---

suspend fun handleReceive(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val method = fp.getString("m", "method")
    val description = fp.getString("d", "description")
    val amountStr = fp.getString("a", "amount")
    val tokenIdentifier = fp.getString("t", "token-identifier")
    val expirySecs = fp.getUInt("e", "expiry-secs")
    val senderPublicKey = fp.getString("s", "sender-public-key")
    val hodl = fp.hasFlag("hodl")
    val newAddress = fp.hasFlag("new-address")

    if (method == null) {
        println("Usage: receive -m <method> [options]")
        println("Methods: sparkaddress, sparkinvoice, bitcoin, bolt11")
        println("Options:")
        println("  -d, --description <desc>        Optional description")
        println("  -a, --amount <amount>            Amount in sats or token base units")
        println("  -t, --token-identifier <id>      Token identifier (spark invoice only)")
        println("  -e, --expiry-secs <secs>         Expiry in seconds")
        println("  -s, --sender-public-key <key>    Sender public key (spark invoice only)")
        println("  --hodl                           Create a HODL invoice (bolt11 only)")
        println("  --new-address                    Get a new bitcoin deposit address")
        return
    }

    val amount = if (amountStr != null) {
        try {
            BigInteger.parseString(amountStr)
        } catch (e: Exception) {
            println("Invalid amount: $amountStr")
            return
        }
    } else null

    val paymentMethod: ReceivePaymentMethod = when (method.lowercase()) {
        "sparkaddress" -> ReceivePaymentMethod.SparkAddress

        "sparkinvoice" -> {
            val expiryTime = if (expirySecs != null) {
                val now = System.currentTimeMillis() / 1000
                (now.toULong() + expirySecs.toULong())
            } else null

            ReceivePaymentMethod.SparkInvoice(
                amount = amount,
                tokenIdentifier = tokenIdentifier,
                expiryTime = expiryTime,
                description = description,
                senderPublicKey = senderPublicKey,
            )
        }

        "bitcoin" -> ReceivePaymentMethod.BitcoinAddress(newAddress = newAddress)

        "bolt11" -> {
            var paymentHash: String? = null
            if (hodl) {
                val random = java.security.SecureRandom()
                val preimageBytes = ByteArray(32)
                random.nextBytes(preimageBytes)
                val preimage = preimageBytes.joinToString("") { "%02x".format(it) }

                val digest = java.security.MessageDigest.getInstance("SHA-256")
                val hashBytes = digest.digest(preimageBytes)
                paymentHash = hashBytes.joinToString("") { "%02x".format(it) }

                println("HODL invoice preimage: $preimage")
                println("Payment hash: $paymentHash")
                println("Save the preimage! Use `claim-htlc-payment` with it to settle.")
            }

            val amountSats = if (amount != null) {
                try {
                    amount.longValue().toULong()
                } catch (e: Exception) {
                    println("Invalid amount for bolt11: $amountStr")
                    return
                }
            } else null

            ReceivePaymentMethod.Bolt11Invoice(
                description = description ?: "",
                amountSats = amountSats,
                expirySecs = expirySecs,
                paymentHash = paymentHash,
            )
        }

        else -> {
            println("Invalid payment method: $method")
            println("Available methods: sparkaddress, sparkinvoice, bitcoin, bolt11")
            return
        }
    }

    val result = sdk.receivePayment(ReceivePaymentRequest(paymentMethod = paymentMethod))

    if (result.fee > BigInteger.ZERO) {
        println("Prepared payment requires fee of ${result.fee} sats/token base units")
    }

    printValue(result)
}

// --- pay ---

suspend fun handlePay(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val paymentRequest = fp.getString("r", "payment-request")
    val amountStr = fp.getString("a", "amount")
    val tokenId = fp.getString("t", "token-identifier")
    val idempotencyKey = fp.getString("i", "idempotency-key")
    val convertFromBitcoin = fp.hasFlag("from-bitcoin")
    val convertFromToken = fp.getString("from-token")
    val maxSlippageBps = fp.getUInt("s", "convert-max-slippage-bps")
    val feesIncluded = fp.hasFlag("fees-included")

    if (paymentRequest == null) {
        println("Usage: pay -r <payment_request> [options]")
        println("Options:")
        println("  -a, --amount <amount>            Optional amount")
        println("  -t, --token-identifier <id>      Optional token identifier")
        println("  -i, --idempotency-key <key>      Optional idempotency key")
        println("  --from-bitcoin                   Convert from Bitcoin")
        println("  --from-token <token_id>          Convert from token to Bitcoin")
        println("  -s, --convert-max-slippage-bps   Max slippage in basis points")
        println("  --fees-included                  Deduct fees from amount")
        return
    }

    val amount = if (amountStr != null) {
        try {
            BigInteger.parseString(amountStr)
        } catch (e: Exception) {
            println("Invalid amount: $amountStr")
            return
        }
    } else null

    val conversionOptions = when {
        convertFromBitcoin -> ConversionOptions(
            conversionType = ConversionType.FromBitcoin,
            maxSlippageBps = maxSlippageBps,
            completionTimeoutSecs = null,
        )
        convertFromToken != null -> ConversionOptions(
            conversionType = ConversionType.ToBitcoin(fromTokenIdentifier = convertFromToken),
            maxSlippageBps = maxSlippageBps,
            completionTimeoutSecs = null,
        )
        else -> null
    }

    val feePolicy = if (feesIncluded) FeePolicy.FEES_INCLUDED else null

    val prepareResponse = sdk.prepareSendPayment(
        PrepareSendPaymentRequest(
            paymentRequest = paymentRequest,
            amount = amount,
            tokenIdentifier = tokenId,
            conversionOptions = conversionOptions,
            feePolicy = feePolicy,
        )
    )

    // Show conversion estimate if applicable
    prepareResponse.conversionEstimate?.let { conversionEstimate ->
        val units = if (conversionEstimate.options.conversionType == ConversionType.FromBitcoin) "sats" else "token base units"
        println("Estimated conversion of ${conversionEstimate.amountIn} $units → ${conversionEstimate.amountOut} $units with a ${conversionEstimate.fee} $units fee")
        val line = readlineWithDefault(reader, "Do you want to continue (y/n): ", "y").lowercase()
        if (line != "y") {
            println("Payment cancelled")
            return
        }
    }

    // Read payment options
    val paymentOptions = readPaymentOptions(prepareResponse.paymentMethod, reader)

    val result = sdk.sendPayment(
        SendPaymentRequest(
            prepareResponse = prepareResponse,
            options = paymentOptions,
            idempotencyKey = idempotencyKey,
        )
    )
    printValue(result)
}

// --- lnurl-pay ---

suspend fun handleLnurlPay(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val comment = fp.getString("c", "comment")
    val validateStr = fp.getString("v", "validate")
    val idempotencyKey = fp.getString("i", "idempotency-key")
    val convertFromToken = fp.getString("from-token")
    val maxSlippageBps = fp.getUInt("s", "convert-max-slippage-bps")
    val feesIncluded = fp.hasFlag("fees-included")

    if (fp.positional.isEmpty()) {
        println("Usage: lnurl-pay <lnurl> [options]")
        println("Options:")
        println("  -c, --comment <comment>          Comment for the invoice")
        println("  -v, --validate <true/false>      Validate success action URL")
        println("  -i, --idempotency-key <key>      Idempotency key")
        println("  --from-token <token_id>          Convert from token to Bitcoin")
        println("  -s, --convert-max-slippage-bps   Max slippage in basis points")
        println("  --fees-included                  Deduct fees from amount")
        return
    }
    val lnurl = fp.positional[0]

    val conversionOptions = convertFromToken?.let { tokenId ->
        ConversionOptions(
            conversionType = ConversionType.ToBitcoin(fromTokenIdentifier = tokenId),
            maxSlippageBps = maxSlippageBps,
            completionTimeoutSecs = null,
        )
    }

    val feePolicy = if (feesIncluded) FeePolicy.FEES_INCLUDED else null

    val parsed = sdk.parse(lnurl)

    val payRequest: LnurlPayRequestDetails = when (parsed) {
        is InputType.LnurlPay -> parsed.v1
        is InputType.LightningAddress -> parsed.v1.payRequest
        else -> {
            println("Input is not an LNURL-pay or lightning address")
            return
        }
    }

    val minSendable = (payRequest.minSendable + 999u) / 1000u
    val maxSendable = payRequest.maxSendable / 1000u
    val amountLine = readlinePrompt(reader, "Amount to pay (min $minSendable sat, max $maxSendable sat): ")
    val amountSats = amountLine.toULongOrNull()
    if (amountSats == null) {
        println("Invalid amount: $amountLine")
        return
    }

    val validateSuccessUrl = if (validateStr != null) validateStr.lowercase() == "true" else null

    val prepareResponse = sdk.prepareLnurlPay(
        PrepareLnurlPayRequest(
            amount = BigInteger.fromLong(amountSats.toLong()),
            payRequest = payRequest,
            comment = comment,
            validateSuccessActionUrl = validateSuccessUrl,
            tokenIdentifier = null,
            conversionOptions = conversionOptions,
            feePolicy = feePolicy,
        )
    )

    // Show conversion estimate if applicable
    prepareResponse.conversionEstimate?.let { conversionEstimate ->
        println("Estimated conversion of ${conversionEstimate.amountIn} token base units → ${conversionEstimate.amountOut} sats with a ${conversionEstimate.fee} token base units fee")
        val line = readlineWithDefault(reader, "Do you want to continue (y/n): ", "y").lowercase()
        if (line != "y") {
            println("Payment cancelled")
            return
        }
    }

    println("Prepared payment:")
    printValue(prepareResponse)
    println("Do you want to continue? (y/n)")
    val line = readlineWithDefault(reader, "", "y").lowercase()
    if (line != "y") {
        return
    }

    val result = sdk.lnurlPay(
        LnurlPayRequest(
            prepareResponse = prepareResponse,
            idempotencyKey = idempotencyKey,
        )
    )
    printValue(result)
}

// --- lnurl-withdraw ---

suspend fun handleLnurlWithdraw(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val timeoutSecs = fp.getUInt("t", "timeout")

    if (fp.positional.isEmpty()) {
        println("Usage: lnurl-withdraw <lnurl> [--timeout N]")
        return
    }

    val parsed = sdk.parse(fp.positional[0])
    val withdrawData = when (parsed) {
        is InputType.LnurlWithdraw -> parsed.v1
        else -> {
            println("Input is not an LNURL-withdraw")
            return
        }
    }

    printValue(withdrawData)

    val minWithdrawable = (withdrawData.minWithdrawable + 999u) / 1000u
    val maxWithdrawable = withdrawData.maxWithdrawable / 1000u
    val amountLine = readlinePrompt(reader, "Amount to withdraw (min $minWithdrawable sat, max $maxWithdrawable sat): ")
    val amountSats = amountLine.toULongOrNull()
    if (amountSats == null) {
        println("Invalid amount: $amountLine")
        return
    }

    val result = sdk.lnurlWithdraw(
        LnurlWithdrawRequest(
            amountSats = amountSats,
            withdrawRequest = withdrawData,
            completionTimeoutSecs = timeoutSecs,
        )
    )
    printValue(result)
}

// --- lnurl-auth ---

suspend fun handleLnurlAuth(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: lnurl-auth <lnurl>")
        return
    }

    val parsed = sdk.parse(args[0])
    val authData = when (parsed) {
        is InputType.LnurlAuth -> parsed.v1
        else -> {
            println("Input is not an LNURL-auth")
            return
        }
    }

    val action = authData.action ?: "auth"
    val line = readlineWithDefault(reader, "Authenticate with ${authData.domain} (action: $action)? (y/n): ", "y").lowercase()
    if (line != "y") {
        return
    }

    val result = sdk.lnurlAuth(authData)
    printValue(result)
}

// --- claim-htlc-payment ---

suspend fun handleClaimHtlcPayment(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: claim-htlc-payment <preimage>")
        return
    }

    val result = sdk.claimHtlcPayment(ClaimHtlcPaymentRequest(preimage = args[0]))
    printValue(result.payment)
}

// --- claim-deposit ---

suspend fun handleClaimDeposit(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val feeSat = fp.getULong("fee-sat")
    val satPerVbyte = fp.getULong("sat-per-vbyte")
    val recommendedFeeLeeway = fp.getULong("recommended-fee-leeway")

    if (fp.positional.size < 2) {
        println("Usage: claim-deposit <txid> <vout> [--fee-sat N | --sat-per-vbyte N | --recommended-fee-leeway N]")
        return
    }

    val txid = fp.positional[0]
    val vout = fp.positional[1].toUIntOrNull()
    if (vout == null) {
        println("Invalid vout: ${fp.positional[1]}")
        return
    }

    val maxFee: MaxFee? = when {
        recommendedFeeLeeway != null -> {
            if (feeSat != null || satPerVbyte != null) {
                println("Cannot specify fee_sat or sat_per_vbyte when using recommended fee")
                return
            }
            MaxFee.NetworkRecommended(leewaySatPerVbyte = recommendedFeeLeeway)
        }
        feeSat != null && satPerVbyte != null -> {
            println("Cannot specify both --fee-sat and --sat-per-vbyte")
            return
        }
        feeSat != null -> MaxFee.Fixed(amount = feeSat)
        satPerVbyte != null -> MaxFee.Rate(satPerVbyte = satPerVbyte)
        else -> null
    }

    val result = sdk.claimDeposit(
        ClaimDepositRequest(
            txid = txid,
            vout = vout,
            maxFee = maxFee,
        )
    )
    printValue(result)
}

// --- parse ---

suspend fun handleParse(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: parse <input>")
        return
    }

    val result = sdk.parse(args[0])
    printValue(result)
}

// --- refund-deposit ---

suspend fun handleRefundDeposit(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val feeSat = fp.getULong("fee-sat")
    val satPerVbyte = fp.getULong("sat-per-vbyte")

    if (fp.positional.size < 3) {
        println("Usage: refund-deposit <txid> <vout> <destination_address> [--fee-sat N | --sat-per-vbyte N]")
        return
    }

    val txid = fp.positional[0]
    val vout = fp.positional[1].toUIntOrNull()
    if (vout == null) {
        println("Invalid vout: ${fp.positional[1]}")
        return
    }
    val destAddr = fp.positional[2]

    val fee: Fee = when {
        feeSat != null && satPerVbyte != null -> {
            println("Cannot specify both --fee-sat and --sat-per-vbyte")
            return
        }
        feeSat != null -> Fee.Fixed(amount = feeSat)
        satPerVbyte != null -> Fee.Rate(satPerVbyte = satPerVbyte)
        else -> {
            println("Must specify either --fee-sat or --sat-per-vbyte")
            return
        }
    }

    val result = sdk.refundDeposit(
        RefundDepositRequest(
            txid = txid,
            vout = vout,
            destinationAddress = destAddr,
            fee = fee,
        )
    )
    printValue(result)
}

// --- list-unclaimed-deposits ---

suspend fun handleListUnclaimedDeposits(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val result = sdk.listUnclaimedDeposits(ListUnclaimedDepositsRequest)
    printValue(result)
}

// --- buy-bitcoin ---

suspend fun handleBuyBitcoin(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val lockedAmount = fp.getULong("amount", "locked-amount-sat")
    val redirectUrl = fp.getString("redirect-url")

    val result = sdk.buyBitcoin(
        BuyBitcoinRequest.Moonpay(
            lockedAmountSat = lockedAmount,
            redirectUrl = redirectUrl,
        )
    )
    println("Open this URL in a browser to complete the purchase:")
    println(result.url)
}

// --- check-lightning-address-available ---

suspend fun handleCheckLightningAddress(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: check-lightning-address-available <username>")
        return
    }

    val available = sdk.checkLightningAddressAvailable(
        CheckLightningAddressRequest(username = args[0])
    )
    if (available) {
        println("Username '${args[0]}' is available")
    } else {
        println("Username '${args[0]}' is NOT available")
    }
}

// --- get-lightning-address ---

suspend fun handleGetLightningAddress(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val result = sdk.getLightningAddress()
    if (result == null) {
        println("No lightning address registered")
    } else {
        printValue(result)
    }
}

// --- register-lightning-address ---

suspend fun handleRegisterLightningAddress(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val description = fp.getString("d", "description")
    val transferPubkey = fp.getString("transfer-pubkey")
    val transferSignature = fp.getString("transfer-signature")

    if (fp.positional.isEmpty()) {
        println("Usage: register-lightning-address <username> [-d <description>] [--transfer-pubkey <pk> --transfer-signature <sig>]")
        return
    }

    if ((transferPubkey == null) != (transferSignature == null)) {
        println("Error: --transfer-pubkey and --transfer-signature must be provided together")
        return
    }
    val transfer = transferPubkey?.let {
        LightningAddressTransfer(pubkey = it, signature = transferSignature!!)
    }

    val result = sdk.registerLightningAddress(
        RegisterLightningAddressRequest(
            username = fp.positional[0],
            description = description,
            transfer = transfer,
        )
    )
    printValue(result)
}

// --- delete-lightning-address ---

suspend fun handleDeleteLightningAddress(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    sdk.deleteLightningAddress()
    println("Lightning address deleted")
}

// --- list-fiat-currencies ---

suspend fun handleListFiatCurrencies(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val result = sdk.listFiatCurrencies()
    printValue(result)
}

// --- list-fiat-rates ---

suspend fun handleListFiatRates(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val result = sdk.listFiatRates()
    printValue(result)
}

// --- recommended-fees ---

suspend fun handleRecommendedFees(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val result = sdk.recommendedFees()
    printValue(result)
}

// --- get-tokens-metadata ---

suspend fun handleGetTokensMetadata(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: get-tokens-metadata <token_id> [<token_id2> ...]")
        return
    }

    val result = sdk.getTokensMetadata(
        GetTokensMetadataRequest(tokenIdentifiers = args)
    )
    printValue(result)
}

// --- fetch-conversion-limits ---

suspend fun handleFetchConversionLimits(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val fromBitcoin = fp.hasFlag("f", "from-bitcoin")
    val tokenId = fp.getString("token") ?: fp.positional.firstOrNull()

    if (tokenId == null) {
        println("Usage: fetch-conversion-limits --token <token_id> [--from-bitcoin]")
        return
    }

    val req = if (fromBitcoin) {
        FetchConversionLimitsRequest(
            conversionType = ConversionType.FromBitcoin,
            tokenIdentifier = tokenId,
        )
    } else {
        FetchConversionLimitsRequest(
            conversionType = ConversionType.ToBitcoin(fromTokenIdentifier = tokenId),
            tokenIdentifier = null,
        )
    }

    val result = sdk.fetchConversionLimits(req)
    printValue(result)
}

// --- get-user-settings ---

suspend fun handleGetUserSettings(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val result = sdk.getUserSettings()
    printValue(result)
}

// --- set-user-settings ---

suspend fun handleSetUserSettings(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val privateMode = fp.getString("p", "private", "spark-private-mode")

    val req = UpdateUserSettingsRequest(
        sparkPrivateModeEnabled = if (privateMode != null) privateMode.lowercase() == "true" else null,
    )

    sdk.updateUserSettings(req)
    println("User settings updated")
}

// --- get-spark-status ---

suspend fun handleGetSparkStatus(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val result = getSparkStatus()
    printValue(result)
}

// ---------------------------------------------------------------------------
// readPaymentOptions -- interactive fee/option selection
// ---------------------------------------------------------------------------

suspend fun readPaymentOptions(
    paymentMethod: SendPaymentMethod,
    reader: LineReader,
): SendPaymentOptions? {
    return when (paymentMethod) {
        is SendPaymentMethod.BitcoinAddress -> {
            val feeQuote = paymentMethod.feeQuote
            val fastFee = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
            val mediumFee = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
            val slowFee = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat

            println("Please choose payment fee:")
            println("1. Fast: $fastFee sats")
            println("2. Medium: $mediumFee sats")
            println("3. Slow: $slowFee sats")

            val line = readlineWithDefault(reader, "Choose (1/2/3): ", "1").trim()
            val speed = when (line) {
                "1" -> OnchainConfirmationSpeed.FAST
                "2" -> OnchainConfirmationSpeed.MEDIUM
                "3" -> OnchainConfirmationSpeed.SLOW
                else -> throw IllegalArgumentException("Invalid confirmation speed")
            }
            SendPaymentOptions.BitcoinAddress(confirmationSpeed = speed)
        }

        is SendPaymentMethod.Bolt11Invoice -> {
            val sparkTransferFeeSats = paymentMethod.sparkTransferFeeSats
            if (sparkTransferFeeSats != null) {
                println("Choose payment option:")
                println("1. Spark transfer fee: $sparkTransferFeeSats sats")
                println("2. Lightning fee: ${paymentMethod.lightningFeeSats} sats")
                val line = readlineWithDefault(reader, "Choose (1/2): ", "1").trim()
                if (line == "1") {
                    return SendPaymentOptions.Bolt11Invoice(
                        preferSpark = true,
                        completionTimeoutSecs = 0u,
                    )
                }
            }
            SendPaymentOptions.Bolt11Invoice(
                preferSpark = false,
                completionTimeoutSecs = 0u,
            )
        }

        is SendPaymentMethod.SparkAddress -> {
            // HTLC options are only valid for Bitcoin payments, not token payments
            if (paymentMethod.tokenIdentifier != null) {
                return null
            }

            val line = readlineWithDefault(reader, "Do you want to create an HTLC transfer? (y/n): ", "n").lowercase()
            if (line != "y") {
                return null
            }

            val paymentHashInput = readlinePrompt(reader,
                "Please enter the HTLC payment hash (hex string) or leave empty to generate a new preimage and associated hash: ")

            val paymentHash = if (paymentHashInput.isEmpty()) {
                val random = java.security.SecureRandom()
                val preimageBytes = ByteArray(32)
                random.nextBytes(preimageBytes)
                val preimage = preimageBytes.joinToString("") { "%02x".format(it) }
                val digest = java.security.MessageDigest.getInstance("SHA-256")
                val hashBytes = digest.digest(preimageBytes)
                val hash = hashBytes.joinToString("") { "%02x".format(it) }

                println("Generated preimage: $preimage")
                println("Associated payment hash: $hash")
                hash
            } else {
                paymentHashInput
            }

            val expiryStr = readlinePrompt(reader, "Please enter the HTLC expiry duration in seconds: ")
            val expiryDurationSecs = expiryStr.toULongOrNull()
            if (expiryDurationSecs == null) {
                println("Invalid expiry duration: $expiryStr")
                return null
            }

            SendPaymentOptions.SparkAddress(
                htlcOptions = SparkHtlcOptions(
                    paymentHash = paymentHash,
                    expiryDurationSecs = expiryDurationSecs,
                )
            )
        }

        is SendPaymentMethod.SparkInvoice -> null

        else -> null
    }
}
