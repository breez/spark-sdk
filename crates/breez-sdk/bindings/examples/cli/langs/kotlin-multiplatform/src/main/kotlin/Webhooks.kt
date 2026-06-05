import breez_sdk_spark.*
import org.jline.reader.LineReader

data class WebhooksCliCommand(
    val name: String,
    val description: String,
    val run: suspend (sdk: BreezSdk, reader: LineReader, args: List<String>) -> Unit
)

val WEBHOOKS_COMMAND_NAMES = listOf(
    "register",
    "unregister",
    "list",
)

fun buildWebhooksRegistry(): Map<String, WebhooksCliCommand> {
    return mapOf(
        "register" to WebhooksCliCommand("register", "Register a new webhook", ::handleRegisterWebhook),
        "unregister" to WebhooksCliCommand("unregister", "Unregister a webhook", ::handleUnregisterWebhook),
        "list" to WebhooksCliCommand("list", "List all registered webhooks", ::handleListWebhooks),
    )
}

suspend fun dispatchWebhooksCommand(
    args: List<String>,
    sdk: BreezSdk,
    registry: Map<String, WebhooksCliCommand>,
    reader: LineReader,
) {
    if (args.isEmpty() || args[0] == "help") {
        println()
        println("Webhooks subcommands:")
        registry.keys.sorted().forEach { name ->
            val cmd = registry[name]!!
            println("  webhooks %-26s %s".format(name, cmd.description))
        }
        println()
        return
    }

    val subName = args[0]
    val subArgs = args.drop(1)

    val cmd = registry[subName]
    if (cmd == null) {
        println("Unknown webhooks subcommand: $subName. Use 'webhooks help' for available commands.")
        return
    }

    try {
        cmd.run(sdk, reader, subArgs)
    } catch (e: Exception) {
        println("Error: ${e.message}")
    }
}

// ---------------------------------------------------------------------------
// Webhooks command handlers
// ---------------------------------------------------------------------------

suspend fun handleRegisterWebhook(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val url = fp.getString("url") ?: fp.positional.getOrNull(0)
    val secret = fp.getString("secret") ?: fp.positional.getOrNull(1)
    val eventsStr = fp.getString("events") ?: fp.positional.drop(2).joinToString(",").ifEmpty { null }

    if (url == null || secret == null || eventsStr == null) {
        println("Usage: webhooks register --url <url> --secret <secret> --events <type1,type2,...>")
        println("Event types: lightning-receive, lightning-send, coop-exit, static-deposit")
        return
    }

    val eventTypes = eventsStr.split(",").map { s ->
        when (s.trim().lowercase()) {
            "lightning-receive" -> WebhookEventType.LightningReceiveFinished
            "lightning-send" -> WebhookEventType.LightningSendFinished
            "coop-exit" -> WebhookEventType.CoopExitFinished
            "static-deposit" -> WebhookEventType.StaticDepositFinished
            else -> {
                println("Unknown event type: ${s.trim()}")
                return
            }
        }
    }

    val result = sdk.registerWebhook(
        RegisterWebhookRequest(
            url = url,
            secret = secret,
            eventTypes = eventTypes,
        )
    )
    printValue(result)
}

suspend fun handleUnregisterWebhook(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: webhooks unregister <webhook_id>")
        return
    }

    sdk.unregisterWebhook(UnregisterWebhookRequest(webhookId = args[0]))
    println("Webhook unregistered successfully")
}

suspend fun handleListWebhooks(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val result = sdk.listWebhooks()
    printValue(result)
}
