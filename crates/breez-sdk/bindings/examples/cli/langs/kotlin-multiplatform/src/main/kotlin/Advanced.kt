import breez_sdk_spark.*
import com.google.gson.JsonObject
import com.google.gson.JsonParser
import java.io.File
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
        ExitLeafSelection.Auto
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
            value = value,
            pubkey = pubkey,
        )
        CpfpFundingKind.P2tr -> CpfpInput.P2tr(
            txid = txid,
            vout = vout,
            value = value,
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
            value = obj["value"].asLong.toULong(),
            pubkey = obj["pubkey"].asString,
        )
        "P2tr" -> CpfpInput.P2tr(
            txid = obj["txid"].asString,
            vout = obj["vout"].asLong.toUInt(),
            value = obj["value"].asLong.toULong(),
            pubkey = obj["pubkey"].asString,
        )
        "Custom" -> CpfpInput.Custom(
            txid = obj["txid"].asString,
            vout = obj["vout"].asLong.toUInt(),
            value = obj["value"].asLong.toULong(),
            scriptPubkeyHex = obj["script_pubkey_hex"].asString,
            signedInputWeight = obj["signed_input_weight"].asLong.toULong(),
        )
        else -> throw IllegalArgumentException("Unknown CpfpInput type: ${obj["type"]}")
    }
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
