/**
 * Contacts subcommands.
 *
 * Mirrors the Rust CLI `contacts` subcommands:
 *   add, update, delete, list
 */

import type { BreezSdk } from '@breeztech/breez-sdk-spark-react-native'
import { formatValue } from './serialization'

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
 * @param sdk - The BreezSdk instance
 * @returns A string result to display
 */
export async function dispatchContactsCommand(
  args: string[],
  sdk: BreezSdk
): Promise<string> {
  if (args.length === 0 || args[0] === 'help') {
    return printContactsHelp()
  }

  const subcommand = args[0]
  const subArgs = args.slice(1)

  switch (subcommand) {
    case 'add':
      return handleAddContact(sdk, subArgs)
    case 'update':
      return handleUpdateContact(sdk, subArgs)
    case 'delete':
      return handleDeleteContact(sdk, subArgs)
    case 'list':
      return handleListContacts(sdk, subArgs)
    default:
      return `Unknown contacts subcommand: ${subcommand}. Use 'contacts help' for available commands.`
  }
}

function printContactsHelp(): string {
  const lines = [
    '',
    'Contacts subcommands:',
    '  contacts add <name> <payment_identifier>           Add a new contact',
    '  contacts update <id> <name> <payment_identifier>   Update an existing contact',
    '  contacts delete <id>                               Delete a contact',
    '  contacts list [<offset> <limit>]                   List contacts',
    '',
  ]
  return lines.join('\n')
}

// --- add ---

async function handleAddContact(sdk: BreezSdk, args: string[]): Promise<string> {
  if (args.length < 2) {
    return 'Usage: contacts add <name> <payment_identifier>'
  }

  const name = args[0]
  const paymentIdentifier = args[1]

  const contact = await sdk.addContact({
    name,
    paymentIdentifier,
  })
  return formatValue(contact)
}

// --- update ---

async function handleUpdateContact(sdk: BreezSdk, args: string[]): Promise<string> {
  if (args.length < 3) {
    return 'Usage: contacts update <id> <name> <payment_identifier>'
  }

  const id = args[0]
  const name = args[1]
  const paymentIdentifier = args[2]

  const contact = await sdk.updateContact({
    id,
    name,
    paymentIdentifier,
  })
  return formatValue(contact)
}

// --- delete ---

async function handleDeleteContact(sdk: BreezSdk, args: string[]): Promise<string> {
  if (args.length < 1) {
    return 'Usage: contacts delete <id>'
  }

  await sdk.deleteContact(args[0])
  return 'Contact deleted successfully'
}

// --- list ---

async function handleListContacts(sdk: BreezSdk, args: string[]): Promise<string> {
  let offset: number | undefined
  let limit: number | undefined

  if (args.length >= 1) {
    offset = parseInt(args[0], 10)
    if (isNaN(offset)) {
      return `Invalid offset: ${args[0]}`
    }
  }
  if (args.length >= 2) {
    limit = parseInt(args[1], 10)
    if (isNaN(limit)) {
      return `Invalid limit: ${args[1]}`
    }
  }

  const contacts = await sdk.listContacts({
    offset,
    limit,
  })
  return formatValue(contacts)
}
