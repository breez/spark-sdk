import breez_sdk_spark.*
import org.jline.reader.LineReader

/**
 * Represents a single contacts subcommand.
 */
data class ContactsCliCommand(
    val name: String,
    val description: String,
    val run: suspend (sdk: BreezSdk, reader: LineReader, args: List<String>) -> Unit
)

/**
 * All contacts subcommand names.
 */
val CONTACTS_COMMAND_NAMES = listOf(
    "add",
    "update",
    "delete",
    "list",
)

/**
 * Builds the contacts command registry.
 */
fun buildContactsRegistry(): Map<String, ContactsCliCommand> {
    return mapOf(
        "add" to ContactsCliCommand("add", "Add a new contact", ::handleAddContact),
        "update" to ContactsCliCommand("update", "Update an existing contact", ::handleUpdateContact),
        "delete" to ContactsCliCommand("delete", "Delete a contact", ::handleDeleteContact),
        "list" to ContactsCliCommand("list", "List contacts", ::handleListContacts),
    )
}

/**
 * Dispatches a contacts subcommand.
 */
suspend fun dispatchContactsCommand(
    args: List<String>,
    sdk: BreezSdk,
    registry: Map<String, ContactsCliCommand>,
    reader: LineReader,
) {
    if (args.isEmpty() || args[0] == "help") {
        println()
        println("Contacts subcommands:")
        registry.keys.sorted().forEach { name ->
            val cmd = registry[name]!!
            println("  contacts %-26s %s".format(name, cmd.description))
        }
        println()
        return
    }

    val subName = args[0]
    val subArgs = args.drop(1)

    val cmd = registry[subName]
    if (cmd == null) {
        println("Unknown contacts subcommand: $subName. Use 'contacts help' for available commands.")
        return
    }

    try {
        cmd.run(sdk, reader, subArgs)
    } catch (e: Exception) {
        println("Error: ${e.message}")
    }
}

// ---------------------------------------------------------------------------
// Contacts command handlers
// ---------------------------------------------------------------------------

// --- add ---

suspend fun handleAddContact(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.size < 2) {
        println("Usage: contacts add <name> <payment_identifier>")
        return
    }

    val contact = sdk.addContact(
        AddContactRequest(
            name = args[0],
            paymentIdentifier = args[1],
        )
    )
    printValue(contact)
}

// --- update ---

suspend fun handleUpdateContact(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.size < 3) {
        println("Usage: contacts update <id> <name> <payment_identifier>")
        return
    }

    val contact = sdk.updateContact(
        UpdateContactRequest(
            id = args[0],
            name = args[1],
            paymentIdentifier = args[2],
        )
    )
    printValue(contact)
}

// --- delete ---

suspend fun handleDeleteContact(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    if (args.isEmpty()) {
        println("Usage: contacts delete <id>")
        return
    }

    sdk.deleteContact(args[0])
    println("Contact deleted successfully")
}

// --- list ---

suspend fun handleListContacts(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val offset = fp.getUInt("offset") ?: fp.positional.getOrNull(0)?.toUIntOrNull()
    val limit = fp.getUInt("limit") ?: fp.positional.getOrNull(1)?.toUIntOrNull()

    val contacts = sdk.listContacts(
        ListContactsRequest(
            offset = offset,
            limit = limit,
        )
    )
    printValue(contacts)
}
