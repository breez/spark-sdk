import breez_sdk_spark.*
import org.jline.reader.LineReader

data class StableBalanceCliCommand(
    val name: String,
    val description: String,
    val run: suspend (sdk: BreezSdk, reader: LineReader, args: List<String>) -> Unit
)

val STABLE_BALANCE_COMMAND_NAMES = listOf(
    "get",
    "set",
    "unset",
)

fun buildStableBalanceRegistry(): Map<String, StableBalanceCliCommand> {
    return mapOf(
        "get" to StableBalanceCliCommand("get", "Get the stable balance active label", ::handleStableBalanceGet),
        "set" to StableBalanceCliCommand("set", "Set the stable balance active label", ::handleStableBalanceSet),
        "unset" to StableBalanceCliCommand("unset", "Unset stable balance", ::handleStableBalanceUnset),
    )
}

suspend fun dispatchStableBalanceCommand(
    args: List<String>,
    sdk: BreezSdk,
    registry: Map<String, StableBalanceCliCommand>,
    reader: LineReader,
) {
    if (args.isEmpty() || args[0] == "help") {
        println()
        println("Stable balance subcommands:")
        registry.keys.sorted().forEach { name ->
            val cmd = registry[name]!!
            println("  stable-balance %-20s %s".format(name, cmd.description))
        }
        println()
        return
    }

    val subName = args[0]
    val subArgs = args.drop(1)

    val cmd = registry[subName]
    if (cmd == null) {
        println("Unknown stable-balance subcommand: $subName. Use 'stable-balance help' for available commands.")
        return
    }

    try {
        cmd.run(sdk, reader, subArgs)
    } catch (e: Exception) {
        println("Error: ${e.message}")
    }
}

// ---------------------------------------------------------------------------
// Stable balance command handlers
// ---------------------------------------------------------------------------

suspend fun handleStableBalanceGet(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val settings = sdk.getUserSettings()
    printValue(settings.stableBalanceActiveLabel)
}

suspend fun handleStableBalanceSet(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: stable-balance set <label>")
        return
    }

    sdk.updateUserSettings(
        UpdateUserSettingsRequest(
            sparkPrivateModeEnabled = null,
            stableBalanceActiveLabel = StableBalanceActiveLabel.Set(label = args[0]),
        )
    )
    val settings = sdk.getUserSettings()
    printValue(settings)
}

suspend fun handleStableBalanceUnset(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    sdk.updateUserSettings(
        UpdateUserSettingsRequest(
            sparkPrivateModeEnabled = null,
            stableBalanceActiveLabel = StableBalanceActiveLabel.Unset,
        )
    )
    val settings = sdk.getUserSettings()
    printValue(settings)
}
