'use strict'

const { Command } = require('commander')
const { printValue } = require('./serialization')

/**
 * Register all contacts subcommands on the given commander program.
 *
 * @param {Command} program - The parent commander program (or subcommand)
 * @param {() => object} getSdk - Function that returns the SDK instance
 */
function registerContactsCommands(program, getSdk) {
  const contacts = program
    .command('contacts')
    .description('Contacts related commands')

  // --- add ---
  contacts
    .command('add')
    .description('Add a new contact')
    .argument('<name>', 'Name of the contact')
    .argument('<payment_identifier>', 'Lightning address (user@domain)')
    .action(async (name, paymentIdentifier) => {
      const sdk = getSdk()
      const contact = await sdk.addContact({
        name,
        paymentIdentifier
      })
      printValue(contact)
    })

  // --- update ---
  contacts
    .command('update')
    .description('Update an existing contact')
    .argument('<id>', 'ID of the contact to update')
    .argument('<name>', 'New name for the contact')
    .argument('<payment_identifier>', 'New Lightning address (user@domain)')
    .action(async (id, name, paymentIdentifier) => {
      const sdk = getSdk()
      const contact = await sdk.updateContact({
        id,
        name,
        paymentIdentifier
      })
      printValue(contact)
    })

  // --- delete ---
  contacts
    .command('delete')
    .description('Delete a contact')
    .argument('<id>', 'ID of the contact to delete')
    .action(async (id) => {
      const sdk = getSdk()
      await sdk.deleteContact(id)
      console.log('Contact deleted successfully')
    })

  // --- list ---
  contacts
    .command('list')
    .description('List contacts')
    .argument('[offset]', 'Number of contacts to skip')
    .argument('[limit]', 'Maximum number of contacts to return')
    .action(async (offset, limit) => {
      const sdk = getSdk()
      const contacts = await sdk.listContacts({
        offset: offset != null ? parseInt(offset, 10) : undefined,
        limit: limit != null ? parseInt(limit, 10) : undefined
      })
      printValue(contacts)
    })
}

module.exports = { registerContactsCommands }
