import breez_sdk_spark.*
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

    if (feeRate == null || destination == null) {
        println("Usage: advanced unilateral-exit --fee-rate <sat/vByte> --destination <address> [options]")
        println("Options:")
        println("  --funding-kind <p2wpkh|p2tr>   Funding UTXO kind (default: p2tr)")
        println("  --leaf <id>                     Leaf id to exit (repeatable, omit for auto)")
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
    }
}

fun printExitTransactions(response: UnilateralExitResponse) {
    println(
        "Recoverable ${response.recoverableValueSat} sats, " +
        "total fee ${response.totalFeeSat} sats, " +
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
        if (tx.status == ConfirmationStatus.CONFIRMED) {
            println("      (already confirmed, nothing to broadcast)")
            continue
        }
        val pkg = if (tx.cpfpTxHex != null) {
            "${tx.txHex},${tx.cpfpTxHex}"
        } else {
            tx.txHex
        }
        println("      Package: $pkg")
    }
}
