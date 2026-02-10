import { type BreezSdk } from '@breeztech/breez-sdk-spark-react-native'

const exampleAddContact = async (sdk: BreezSdk) => {
  // ANCHOR: add-contact
  const contact = await sdk.addContact({
    name: 'Alice',
    paymentIdentifier: 'alice@example.com'
  })
  console.log(`Contact added: ${JSON.stringify(contact)}`)
  // ANCHOR_END: add-contact
}

const exampleUpdateContact = async (sdk: BreezSdk) => {
  // ANCHOR: update-contact
  const contactId = 'contact-id'
  const contact = await sdk.updateContact({
    id: contactId,
    name: 'Alice Smith',
    paymentIdentifier: 'alice.smith@example.com'
  })
  console.log(`Contact updated: ${JSON.stringify(contact)}`)
  // ANCHOR_END: update-contact
}

const exampleDeleteContact = async (sdk: BreezSdk) => {
  // ANCHOR: delete-contact
  const contactId = 'contact-id'
  await sdk.deleteContact(contactId)
  console.log('Contact deleted')
  // ANCHOR_END: delete-contact
}

const exampleListContacts = async (sdk: BreezSdk) => {
  // ANCHOR: list-contacts
  // List contacts with pagination (e.g., 10 contacts starting from offset 0)
  // Optionally filter by exact name match
  const contacts = await sdk.listContacts({
    name: undefined, // Set to "Alice" to filter by name
    offset: 0,
    limit: 10
  })
  for (const contact of contacts) {
    console.log(`Contact: id=${contact.id}, name=${contact.name}, identifier=${contact.paymentIdentifier}`)
  }
  // ANCHOR_END: list-contacts
}
