/**
 * Contacts subcommands.
 *
 * Note: Contact management (add, update, delete, list) is not yet supported
 * in the React Native SDK. These handlers return a "not yet supported" message.
 */

/** All contacts subcommand names for help and completion. */
export const CONTACTS_COMMAND_NAMES = [
  'add',
  'update',
  'delete',
  'list',
]

/**
 * Dispatch a contacts subcommand.
 *
 * @param args - The arguments after "contacts" (e.g., ["add", "Alice", "alice@example.com"])
 * @param _sdk - The BreezSdk instance (unused - contacts not yet supported)
 * @returns A string result to display
 */
export async function dispatchContactsCommand(
  args: string[],
  _sdk: unknown
): Promise<string> {
  if (args.length === 0 || args[0] === 'help') {
    return printContactsHelp()
  }

  const subcommand = args[0]

  switch (subcommand) {
    case 'add':
    case 'update':
    case 'delete':
    case 'list':
      return 'Not yet supported in React Native'
    default:
      return `Unknown contacts subcommand: ${subcommand}. Use 'contacts help' for available commands.`
  }
}

function printContactsHelp(): string {
  const lines = [
    '',
    'Contacts subcommands (not yet supported in React Native):',
    '  contacts add <name> <payment_identifier>           Add a new contact',
    '  contacts update <id> <name> <payment_identifier>   Update an existing contact',
    '  contacts delete <id>                               Delete a contact',
    '  contacts list [<offset> <limit>]                   List contacts',
    '',
  ]
  return lines.join('\n')
}
