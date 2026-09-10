# Managing contacts

Contacts allow you to save Lightning addresses for quick access. Each contact stores a name and a Lightning address, making it easy to send payments to frequently used recipients. Contacts are synced across all instances of the SDK.

## Adding a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_contact

To add a new contact, provide a name and a Lightning address.

```typescript
const contact = await sdk.addContact({
  name: 'Alice',
  paymentIdentifier: 'alice@example.com'
})
console.log(`Contact added: ${JSON.stringify(contact)}`)
```



## Updating a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.update_contact

To update an existing contact, provide the contact ID along with the new name and Lightning address.

```typescript
const contactId = 'contact-id'
const contact = await sdk.updateContact({
  id: contactId,
  name: 'Alice Smith',
  paymentIdentifier: 'alice.smith@example.com'
})
console.log(`Contact updated: ${JSON.stringify(contact)}`)
```



## Deleting a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.delete_contact

To remove a contact, pass its ID to the delete method.

```typescript
const contactId = 'contact-id'
await sdk.deleteContact(contactId)
console.log('Contact deleted')
```



## Listing contacts

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_contacts

To retrieve your saved contacts, use the list method. The results support pagination through offset and limit parameters.

```typescript
// List contacts with pagination (e.g., 10 contacts starting from offset 0)
const contacts = await sdk.listContacts({
  offset: 0,
  limit: 10
})
for (const contact of contacts) {
  console.log(
    `Contact: id=${contact.id}, name=${contact.name}, ` +
    `identifier=${contact.paymentIdentifier}`
  )
}
```
