import breez_sdk_spark.*
import com.google.gson.JsonObject
import com.google.gson.JsonParser
import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import org.jline.reader.LineReader

/**
 * Represents a single advanced subcommand.
 */
data class AdvancedCliCommand(
    val name: String,
    val description: String,
    val run: suspend (sdk: BreezSdk, reader: LineReader, args: List<String>) -> Unit
)

/**
 * All advanced subcommand names.
 */
val ADVANCED_COMMAND_NAMES = listOf(
    "unilateral-exit",
    "check-unilateral-exit",
    "recover-funds",
    "check-recover-funds",
    "export-unilateral-exit-state",
    "import-unilateral-exit-state",
)

/**
 * Builds the advanced command registry.
 */
fun buildAdvancedRegistry(): Map<String, AdvancedCliCommand> {
    return mapOf(
        "unilateral-exit" to AdvancedCliCommand(
            "unilateral-exit",
            "Build and sign a unilateral exit",
            ::handleUnilateralExit,
        ),
        "check-unilateral-exit" to AdvancedCliCommand(
            "check-unilateral-exit",
            "Check a signed exit against the chain",
            ::handleCheckUnilateralExit,
        ),
        "recover-funds" to AdvancedCliCommand(
            "recover-funds",
            "Recover the funds that left the balance, or with --all every leaf",
            ::handleRecoverFunds,
        ),
        "check-recover-funds" to AdvancedCliCommand(
            "check-recover-funds",
            "Read a recovery written by recover-funds back against the chain",
            ::handleCheckRecoverFunds,
        ),
        "export-unilateral-exit-state" to AdvancedCliCommand(
            "export-unilateral-exit-state",
            "Export the wallet's unilateral exit state to a file",
            ::handleExportUnilateralExitState,
        ),
        "import-unilateral-exit-state" to AdvancedCliCommand(
            "import-unilateral-exit-state",
            "Import a unilateral exit state from a file",
            ::handleImportUnilateralExitState,
        ),
    )
}

/**
 * Dispatches an advanced subcommand.
 */
suspend fun dispatchAdvancedCommand(
    args: List<String>,
    sdk: BreezSdk,
    registry: Map<String, AdvancedCliCommand>,
    reader: LineReader,
) {
    if (args.isEmpty() || args[0] == "help") {
        println()
        println("Advanced subcommands (expert-only, misuse can strand or lose funds):")
        registry.keys.sorted().forEach { name ->
            val cmd = registry[name]!!
            println("  advanced %-26s %s".format(name, cmd.description))
        }
        println()
        return
    }

    val subName = args[0]
    val subArgs = args.drop(1)

    val cmd = registry[subName]
    if (cmd == null) {
        println("Unknown advanced subcommand: $subName. Use 'advanced help' for available commands.")
        return
    }

    try {
        cmd.run(sdk, reader, subArgs)
    } catch (e: Exception) {
        println("Error: ${e.message}")
    }
}

// ---------------------------------------------------------------------------
// Advanced command handlers
// ---------------------------------------------------------------------------

// --- unilateral-exit ---

@OptIn(kotlin.ExperimentalStdlibApi::class)
suspend fun handleUnilateralExit(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val feeRate = fp.getULong("fee-rate")
    val fundingKindStr = fp.getString("funding-kind") ?: "p2tr"
    val destination = fp.getString("destination")
    val leafIds = fp.getAll("leaf")
    val outputFile = fp.getString("output-file")

    if (feeRate == null || destination == null) {
        println("Usage: advanced unilateral-exit --fee-rate <sat/vByte> --destination <address> [options]")
        println("Options:")
        println("  --funding-kind <p2wpkh|p2tr>   Funding UTXO kind (default: p2tr)")
        println("  --leaf <id>                     Leaf id to exit (repeatable, omit for auto)")
        println("  --output-file <path>            File to write the signed exit to")
        return
    }

    val fundingKind = when (fundingKindStr.lowercase()) {
        "p2wpkh" -> CpfpFundingKind.P2wpkh
        "p2tr" -> CpfpFundingKind.P2tr
        else -> {
            println("Invalid funding kind: $fundingKindStr (expected p2wpkh or p2tr)")
            return
        }
    }

    val selection = if (leafIds.isEmpty()) {
        ExitLeafSelection.All
    } else {
        ExitLeafSelection.Specific(leafIds = leafIds)
    }

    val prepared = sdk.prepareUnilateralExit(
        PrepareUnilateralExitRequest(
            feeRateSatPerVbyte = feeRate,
            fundingKind = fundingKind,
            destination = destination,
            selection = selection,
        )
    )
    printValue(prepared)

    if (prepared.leaves.isEmpty()) {
        println("No leaves to exit.")
        return
    }

    val utxoLine = readlinePrompt(
        reader,
        "Funding UTXO(s) as txid:vout:value:pubkey (space-separated, blank to stop): ",
    )
    if (utxoLine.isBlank()) {
        println("No funding provided; showing the quote only.")
        return
    }

    val fundingInputs = try {
        utxoLine.split("\\s+".toRegex()).map { parseCpfpInput(it, fundingKind) }
    } catch (e: Exception) {
        println("Error parsing funding UTXOs: ${e.message}")
        return
    }

    val keyLine = readlinePrompt(reader, "Hex secret key for the funding UTXO(s): ")
    val signer = try {
        singleKeyCpfpSigner(keyLine.trim().hexToByteArray())
    } catch (e: Exception) {
        println("Error creating signer: ${e.message}")
        return
    }

    val response = sdk.unilateralExit(
        UnilateralExitRequest(
            prepared = prepared,
            fundingInputs = fundingInputs,
        ),
        signer,
    )
    printExitTransactions(response)
    if (outputFile != null) {
        writeExit(outputFile, response)
    }
}

fun parseCpfpInput(s: String, kind: CpfpFundingKind): CpfpInput {
    val parts = s.split(":")
    if (parts.size != 4) {
        throw IllegalArgumentException("invalid funding UTXO '$s', expected txid:vout:value:pubkey")
    }
    val txid = parts[0]
    val vout = parts[1].toUIntOrNull()
        ?: throw IllegalArgumentException("invalid vout in '$s'")
    val value = parts[2].toULongOrNull()
        ?: throw IllegalArgumentException("invalid value in '$s'")
    val pubkey = parts[3]

    return when (kind) {
        CpfpFundingKind.P2wpkh -> CpfpInput.P2wpkh(
            txid = txid,
            vout = vout,
            valueSats = value,
            pubkey = pubkey,
        )
        CpfpFundingKind.P2tr -> CpfpInput.P2tr(
            txid = txid,
            vout = vout,
            valueSats = value,
            pubkey = pubkey,
        )
        is CpfpFundingKind.Custom ->
            throw IllegalArgumentException("custom funding kind is not supported by this CLI")
    }
}

// --- check-unilateral-exit ---

suspend fun handleCheckUnilateralExit(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val inputFile = fp.getString("input-file")
    val outputFile = fp.getString("output-file")

    if (inputFile == null) {
        println("Usage: advanced check-unilateral-exit --input-file <path> [--output-file <path>]")
        return
    }

    val exit = readExit(inputFile)
    val checked = sdk.checkUnilateralExit(CheckUnilateralExitRequest(exit = exit))
    println("Verdict: ${checked.verdict}")
    if (checked.verdict is UnilateralExitVerdict.Redo) {
        println("  (this exit cannot be finished, quote and build it again)")
    }
    printExitTransactions(checked.exit)
    writeExit(outputFile ?: inputFile, checked.exit)
}

fun readExit(path: String): UnilateralExitResponse {
    val root = JsonParser.parseString(File(path).readText()).asJsonObject
    return UnilateralExitResponse(
        recoverableValueSat = root["recoverable_value_sat"].asLong.toULong(),
        totalFeeSat = root["total_fee_sat"].asLong.toULong(),
        cpfpFeeSat = root["cpfp_fee_sat"].asLong.toULong(),
        fanoutFeeSat = root["fanout_fee_sat"].asLong.toULong(),
        sweepFeeSat = root["sweep_fee_sat"].asLong.toULong(),
        leaves = root["leaves"].asJsonArray.map { deserializeLeaf(it.asJsonObject) },
        transactions = root["transactions"].asJsonArray.map { deserializeTx(it.asJsonObject) },
        fundingInputs = root["funding_inputs"].asJsonArray.map { deserializeFundingInput(it.asJsonObject) },
    )
}

fun writeExit(path: String, exit: UnilateralExitResponse) {
    File(path).writeText(serialize(exit))
    println("Wrote the exit to $path")
}

private fun deserializeLeaf(obj: JsonObject): UnilateralExitLeaf {
    return UnilateralExitLeaf(
        leafId = obj["leaf_id"].asString,
        value = obj["value"].asLong.toULong(),
    )
}

private fun deserializeTx(obj: JsonObject): UnilateralExitTransaction {
    return UnilateralExitTransaction(
        kind = UnilateralExitTxKind.valueOf(obj["kind"].asString),
        nodeId = obj["node_id"]?.takeIf { !it.isJsonNull }?.asString,
        txid = obj["txid"].asString,
        txHex = obj["tx_hex"].asString,
        cpfpTxHex = obj["cpfp_tx_hex"]?.takeIf { !it.isJsonNull }?.asString,
        csvTimelockBlocks = obj["csv_timelock_blocks"]?.takeIf { !it.isJsonNull }?.asLong?.toUInt(),
        dependsOn = obj["depends_on"].asJsonArray.map { it.asString },
        status = deserializeStatus(obj["status"].asJsonObject),
    )
}

private fun deserializeStatus(obj: JsonObject): ExitTransactionStatus {
    return when (obj["type"].asString) {
        "Confirmed" -> ExitTransactionStatus.Confirmed(
            blockHeight = obj["block_height"]?.takeIf { !it.isJsonNull }?.asLong?.toUInt(),
        )
        "Ready" -> ExitTransactionStatus.Ready
        "WaitingForDependencies" -> ExitTransactionStatus.WaitingForDependencies
        "WaitingForTimelock" -> ExitTransactionStatus.WaitingForTimelock(
            spendableAtHeight = obj["spendable_at_height"]?.takeIf { !it.isJsonNull }?.asLong?.toUInt(),
        )
        "Unverified" -> ExitTransactionStatus.Unverified
        else -> throw IllegalArgumentException("Unknown ExitTransactionStatus: ${obj["type"]}")
    }
}

private fun deserializeFundingInput(obj: JsonObject): CpfpInput {
    return when (obj["type"].asString) {
        "P2wpkh" -> CpfpInput.P2wpkh(
            txid = obj["txid"].asString,
            vout = obj["vout"].asLong.toUInt(),
            valueSats = obj["value_sats"].asLong.toULong(),
            pubkey = obj["pubkey"].asString,
        )
        "P2tr" -> CpfpInput.P2tr(
            txid = obj["txid"].asString,
            vout = obj["vout"].asLong.toUInt(),
            valueSats = obj["value_sats"].asLong.toULong(),
            pubkey = obj["pubkey"].asString,
        )
        "Custom" -> CpfpInput.Custom(
            txid = obj["txid"].asString,
            vout = obj["vout"].asLong.toUInt(),
            valueSats = obj["value_sats"].asLong.toULong(),
            scriptPubkeyHex = obj["script_pubkey_hex"].asString,
            signedInputWeight = obj["signed_input_weight"].asLong.toULong(),
        )
        else -> throw IllegalArgumentException("Unknown CpfpInput type: ${obj["type"]}")
    }
}

// --- recover-funds ---

suspend fun handleRecoverFunds(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val feeRate = fp.getULong("fee-rate")
    val fundingKindStr = fp.getString("funding-kind") ?: "p2tr"
    val destination = fp.getString("destination")
    val all = fp.hasFlag("all")
    val leafIds = fp.getAll("leaf")
    val outputFile = fp.getString("output-file")

    if (feeRate == null || destination == null) {
        println("Usage: advanced recover-funds --fee-rate <sat/vByte> --destination <address> [options]")
        println("Options:")
        println("  --funding-kind <p2wpkh|p2tr>    Funding UTXO kind (default: p2tr)")
        println("  --all                           Recover every leaf worth it, including the ones still in the balance (only for when the operators are unreachable or refuse to serve the wallet)")
        println("  --leaf <id>                     Leaf id to recover (repeatable, omit to recover the leaves that left the balance)")
        println("  --output-file <path>            File to write the signed recovery to, for check-recover-funds to read back")
        return
    }

    if (all && leafIds.isNotEmpty()) {
        println("Error: --all and --leaf are mutually exclusive")
        return
    }

    val fundingKind = when (fundingKindStr.lowercase()) {
        "p2wpkh" -> CpfpFundingKind.P2wpkh
        "p2tr" -> CpfpFundingKind.P2tr
        else -> {
            println("Invalid funding kind: $fundingKindStr (expected p2wpkh or p2tr)")
            return
        }
    }

    val request = PrepareRecoverFundsRequest(
        feeRateSatPerVbyte = feeRate,
        fundingKind = fundingKind,
        destination = destination,
        selection = recoverySelection(all, leafIds),
    )
    recoverFunds(sdk, reader, request, fundingKind, outputFile)
}

@OptIn(kotlin.ExperimentalStdlibApi::class)
suspend fun recoverFunds(
    sdk: BreezSdk,
    reader: LineReader,
    request: PrepareRecoverFundsRequest,
    fundingKind: CpfpFundingKind,
    outputFile: String?,
) {
    var prepared = sdk.prepareRecoverFunds(request)
    if (prepared.leaves.isEmpty()) {
        println(
            "Nothing to recover: each selected leaf is finished, not worth recovering at this fee " +
            "rate, or its funds were not found."
        )
        return
    }
    printQuote(prepared)
    if (outputFile == null) {
        println("Without --output-file the recovery is only printed: check-recover-funds cannot read it back.")
    }

    var fundingInputs = emptyList<CpfpInput>()
    var signer: CpfpSigner? = null
    val singleUtxoSats = prepared.funding?.singleUtxoSats
    if (singleUtxoSats != null) {
        val utxoLine = readlinePrompt(
            reader,
            "Funding UTXO(s) of at least $singleUtxoSats sats, as txid:vout:value:pubkey " +
            "(space-separated; for P2TR the internal key; blank to skip the unilateral exit): ",
        )
        if (utxoLine.isBlank()) {
            val cooperative = prepared.leaves
                .filter { it.method == RecoveryMethod.COOPERATIVE }
                .map { it.leafId }
            if (cooperative.isEmpty()) {
                println("Nothing to recover without funding.")
                return
            }
            println("Recovering only the cooperative leaves:")
            prepared = sdk.prepareRecoverFunds(
                request.copy(selection = ExitLeafSelection.Specific(leafIds = cooperative))
            )
            printQuote(prepared)
        } else {
            fundingInputs = try {
                utxoLine.split("\\s+".toRegex()).map { parseCpfpInput(it, fundingKind) }
            } catch (e: Exception) {
                println("Error parsing funding UTXOs: ${e.message}")
                return
            }
            val keyLine = readlinePrompt(reader, "Hex secret key for the funding UTXO(s): ")
            signer = try {
                singleKeyCpfpSigner(keyLine.trim().hexToByteArray())
            } catch (e: Exception) {
                println("Error creating signer: ${e.message}")
                return
            }
        }
    }

    val answer = readlineWithDefault(reader, "Sign this recovery? (y/n): ", "y").lowercase()
    if (answer != "y") {
        return
    }
    val response = sdk.recoverFunds(
        RecoverFundsRequest(
            prepared = prepared,
            fundingInputs = fundingInputs,
        ),
        signer,
    )
    printRecovery(response)
    if (outputFile != null) {
        writeRecovery(outputFile, response)
        println(
            "Next: broadcast the Ready packages. After new blocks, run check-recover-funds " +
            "--input-file $outputFile to see what is ready next."
        )
    } else {
        println("Next: broadcast the Ready packages.")
    }
}

fun printQuote(prepared: PrepareRecoverFundsResponse) {
    printValue(prepared)
    val cooperative = prepared.leaves.count { it.method == RecoveryMethod.COOPERATIVE }
    println(
        "${prepared.leaves.size} leaf(s), $cooperative cooperative and " +
        "${prepared.leaves.size - cooperative} unilateral: " +
        "recovering ${prepared.recoverableValueSats} sats for ${prepared.totalFeeSats} sats in fees"
    )
}

fun recoverySelection(all: Boolean, leafIds: List<String>): ExitLeafSelection {
    return when {
        all -> ExitLeafSelection.All
        leafIds.isEmpty() -> ExitLeafSelection.RecoverableOnly
        else -> ExitLeafSelection.Specific(leafIds = leafIds)
    }
}

// --- check-recover-funds ---

suspend fun handleCheckRecoverFunds(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val inputFile = fp.getString("input-file")
    val outputFile = fp.getString("output-file")

    if (inputFile == null) {
        println("Usage: advanced check-recover-funds --input-file <path> [--output-file <path>]")
        return
    }

    checkRecoverFunds(sdk, inputFile, outputFile)
}

suspend fun checkRecoverFunds(sdk: BreezSdk, inputFile: String, outputFile: String?) {
    val recovery = readRecovery(inputFile)
    val checked = sdk.checkRecoverFunds(CheckRecoverFundsRequest(recovery = recovery))
    println("Verdict: ${checked.verdict}")
    if (checked.verdict is RecoveryVerdict.Redo) {
        println("  (this recovery cannot finish: run ${redoCommand(checked.recovery)})")
    }
    printRecovery(checked.recovery)
    writeRecovery(outputFile ?: inputFile, checked.recovery)
}

fun redoCommand(recovery: RecoverFundsResponse): String {
    val command = StringBuilder(
        "recover-funds --fee-rate ${recovery.feeRateSatPerVbyte} --destination ${recovery.destination}"
    )
    for (leaf in recovery.leaves) {
        command.append(" --leaf ").append(leaf.leafId)
    }
    return command.toString()
}

fun readRecovery(path: String): RecoverFundsResponse {
    val root = JsonParser.parseString(File(path).readText()).asJsonObject
    return RecoverFundsResponse(
        recoverableValueSats = root["recoverable_value_sats"].asLong.toULong(),
        totalFeeSats = root["total_fee_sats"].asLong.toULong(),
        cooperativeFeeSats = root["cooperative_fee_sats"].asLong.toULong(),
        cpfpFeeSats = root["cpfp_fee_sats"].asLong.toULong(),
        fanoutFeeSats = root["fanout_fee_sats"].asLong.toULong(),
        sweepFeeSats = root["sweep_fee_sats"].asLong.toULong(),
        leaves = root["leaves"].asJsonArray.map { deserializeRecoveryLeaf(it.asJsonObject) },
        failed = root["failed"].asJsonArray.map { deserializeRecoveryFailure(it.asJsonObject) },
        transactions = root["transactions"].asJsonArray.map { deserializeRecoveryTx(it.asJsonObject) },
        fundingInputs = root["funding_inputs"].asJsonArray.map { deserializeFundingInput(it.asJsonObject) },
        feeRateSatPerVbyte = root["fee_rate_sat_per_vbyte"].asLong.toULong(),
        destination = root["destination"].asString,
    )
}

fun writeRecovery(path: String, recovery: RecoverFundsResponse) {
    val temporary = File("$path.tmp")
    temporary.writeText(serialize(recovery))
    Files.move(
        temporary.toPath(),
        File(path).toPath(),
        StandardCopyOption.REPLACE_EXISTING,
        StandardCopyOption.ATOMIC_MOVE,
    )
    println("Wrote the recovery to $path")
}

private fun deserializeRecoveryLeaf(obj: JsonObject): RecoverFundsLeaf {
    return RecoverFundsLeaf(
        leafId = obj["leaf_id"].asString,
        valueSats = obj["value_sats"].asLong.toULong(),
        method = RecoveryMethod.valueOf(obj["method"].asString),
    )
}

private fun deserializeRecoveryFailure(obj: JsonObject): CooperativeRecoveryFailure {
    return CooperativeRecoveryFailure(
        leafId = obj["leaf_id"].asString,
        outputTxid = obj["output_txid"].asString,
        outputVout = obj["output_vout"].asLong.toUInt(),
        error = deserializeRecoveryError(obj["error"].asJsonObject),
    )
}

private fun deserializeRecoveryError(obj: JsonObject): CooperativeRecoveryError {
    return when (obj["type"].asString) {
        "ReplacementFeeTooLow" -> CooperativeRecoveryError.ReplacementFeeTooLow(
            requiredFeeSats = obj["required_fee_sats"].asLong.toULong(),
            requiredFeeRateSatPerVbyte = obj["required_fee_rate_sat_per_vbyte"].asLong.toULong(),
        )
        "OperatorsUnavailable" -> CooperativeRecoveryError.OperatorsUnavailable(
            message = obj["message"].asString,
        )
        "Generic" -> CooperativeRecoveryError.Generic(
            message = obj["message"].asString,
        )
        else -> throw IllegalArgumentException("Unknown CooperativeRecoveryError: ${obj["type"]}")
    }
}

private fun deserializeRecoveryTx(obj: JsonObject): RecoveryTransaction {
    return RecoveryTransaction(
        kind = RecoveryTxKind.valueOf(obj["kind"].asString),
        nodeId = obj["node_id"]?.takeIf { !it.isJsonNull }?.asString,
        txid = obj["txid"].asString,
        txHex = obj["tx_hex"].asString,
        cpfpTxHex = obj["cpfp_tx_hex"]?.takeIf { !it.isJsonNull }?.asString,
        csvTimelockBlocks = obj["csv_timelock_blocks"]?.takeIf { !it.isJsonNull }?.asLong?.toUInt(),
        dependsOn = obj["depends_on"].asJsonArray.map { it.asString },
        status = deserializeStatus(obj["status"].asJsonObject),
    )
}

// --- export-unilateral-exit-state ---

suspend fun handleExportUnilateralExitState(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val outputFile = fp.getString("output-file")

    if (outputFile == null) {
        println("Usage: advanced export-unilateral-exit-state --output-file <path>")
        return
    }

    val exported = sdk.exportUnilateralExitState()
    File(outputFile).writeText(exported.exitState)
    println("Wrote ${exported.exitState.length} bytes to $outputFile")
}

// --- import-unilateral-exit-state ---

suspend fun handleImportUnilateralExitState(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val inputFile = fp.getString("input-file")

    if (inputFile == null) {
        println("Usage: advanced import-unilateral-exit-state --input-file <path>")
        return
    }

    val exitState = File(inputFile).readText()
    val imported = sdk.importUnilateralExitState(ImportUnilateralExitStateRequest(exitState))
    println(
        "Imported ${imported.importedLeaves} leaf(s), " +
        "skipped ${imported.skippedForeignLeaves} leaf(s) from a different wallet " +
        "and ${imported.skippedConflictingLeaves} that disagree with what this wallet holds, " +
        "left out the exit data of ${imported.skippedChains} leaf(s)"
    )
}

fun printExitTransactions(response: UnilateralExitResponse) {
    println(
        "Recoverable ${response.recoverableValueSat} sats, " +
        "total fee ${response.totalFeeSat} sats " +
        "(cpfp ${response.cpfpFeeSat}, fanout ${response.fanoutFeeSat}, sweep ${response.sweepFeeSat}), " +
        "${response.transactions.size} transaction(s):"
    )
    for ((i, tx) in response.transactions.withIndex()) {
        val after = if (tx.dependsOn.isEmpty()) {
            ""
        } else {
            ", after ${tx.dependsOn.joinToString(",")}"
        }
        val csv = tx.csvTimelockBlocks?.let { ", csv $it blocks" } ?: ""
        println("  [$i] ${tx.kind} status=${tx.status} txid=${tx.txid}$after$csv")
        when (val status = tx.status) {
            is ExitTransactionStatus.Confirmed -> {
                val height = status.blockHeight
                if (height != null) {
                    println("      (confirmed in block $height, nothing to broadcast)")
                } else {
                    println("      (already confirmed, nothing to broadcast)")
                }
                continue
            }
            is ExitTransactionStatus.WaitingForDependencies -> {
                println("      (waiting on the transactions it depends on)")
            }
            is ExitTransactionStatus.WaitingForTimelock -> {
                val height = status.spendableAtHeight
                if (height != null) {
                    println("      (waiting for its timelock, until block $height)")
                } else {
                    println("      (waiting for its timelock)")
                }
            }
            is ExitTransactionStatus.Ready,
            is ExitTransactionStatus.Unverified -> {}
        }
        val pkg = if (tx.cpfpTxHex != null) {
            "${tx.txHex},${tx.cpfpTxHex}"
        } else {
            tx.txHex
        }
        println("      Package: $pkg")
    }
}

fun printRecovery(response: RecoverFundsResponse) {
    println(
        "Recoverable ${response.recoverableValueSats} sats, " +
        "total fee ${response.totalFeeSats} sats " +
        "(cooperative ${response.cooperativeFeeSats}, cpfp ${response.cpfpFeeSats}, " +
        "fanout ${response.fanoutFeeSats}, sweep ${response.sweepFeeSats}), " +
        "${response.transactions.size} transaction(s):"
    )
    for ((i, tx) in response.transactions.withIndex()) {
        val after = if (tx.dependsOn.isEmpty()) {
            ""
        } else {
            ", after ${tx.dependsOn.joinToString(",")}"
        }
        val csv = tx.csvTimelockBlocks?.let { ", csv $it blocks" } ?: ""
        val node = tx.nodeId?.let { " node=$it" } ?: ""
        println("  [$i] ${tx.kind}$node status=${tx.status} txid=${tx.txid}$after$csv")
        when (val status = tx.status) {
            is ExitTransactionStatus.Confirmed -> {
                val height = status.blockHeight
                if (height != null) {
                    println("      (confirmed in block $height, nothing to broadcast)")
                } else {
                    println("      (already confirmed, nothing to broadcast)")
                }
                continue
            }
            is ExitTransactionStatus.WaitingForDependencies -> {
                println("      (waiting on the transactions it depends on)")
            }
            is ExitTransactionStatus.WaitingForTimelock -> {
                val height = status.spendableAtHeight
                if (height != null) {
                    println("      (waiting for its timelock, until block $height)")
                } else {
                    println("      (waiting for its timelock)")
                }
            }
            is ExitTransactionStatus.Ready,
            is ExitTransactionStatus.Unverified -> {}
        }
        val pkg = if (tx.cpfpTxHex != null) {
            "${tx.txHex},${tx.cpfpTxHex}"
        } else {
            tx.txHex
        }
        println("      Package: $pkg")
    }
    if (response.failed.isNotEmpty()) {
        println("Not recovered, ${response.failed.size} leaf(s):")
    }
    for (failure in response.failed) {
        println(
            "  leaf ${failure.leafId} " +
            "(output ${failure.outputTxid}:${failure.outputVout}): " +
            recoveryErrorMessage(failure.error)
        )
    }
}

private fun recoveryErrorMessage(error: CooperativeRecoveryError): String {
    return when (error) {
        is CooperativeRecoveryError.ReplacementFeeTooLow ->
            "A recovery of this output is already on the network: replacing it takes at least " +
            "${error.requiredFeeSats} sats or ${error.requiredFeeRateSatPerVbyte} sats/vbyte"
        is CooperativeRecoveryError.OperatorsUnavailable -> "Operators unavailable: ${error.message}"
        is CooperativeRecoveryError.Generic -> "Generic error: ${error.message}"
    }
}
