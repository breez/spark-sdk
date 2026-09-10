# Managing contacts

Contacts allow you to save Lightning addresses for quick access. Each contact stores a name and a Lightning address, making it easy to send payments to frequently used recipients. Contacts are synced across all instances of the SDK.

## Adding a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_contact

To add a new contact, provide a name and a Lightning address.

```kotlin
val contact = sdk.addContact(AddContactRequest(
    name = "Alice",
    paymentIdentifier = "alice@example.com"
))
// Log.v("Breez", "Contact added: $contact")
```



## Updating a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.update_contact

To update an existing contact, provide the contact ID along with the new name and Lightning address.

```kotlin
val contactId = "contact-id"
val contact = sdk.updateContact(UpdateContactRequest(
    id = contactId,
    name = "Alice Smith",
    paymentIdentifier = "alice.smith@example.com"
))
// Log.v("Breez", "Contact updated: $contact")
```



## Deleting a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.delete_contact

To remove a contact, pass its ID to the delete method.

```kotlin
val contactId = "contact-id"
sdk.deleteContact(contactId)
// Log.v("Breez", "Contact deleted")
```



## Listing contacts

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_contacts

To retrieve your saved contacts, use the list method. The results support pagination through offset and limit parameters.

```kotlin
// List contacts with pagination (e.g., 10 contacts starting from offset 0)
val contacts = sdk.listContacts(ListContactsRequest(
    offset = 0u,
    limit = 10u
))
for (contact in contacts) {
    // Log.v("Breez", "Contact: id=${contact.id}, name=${contact.name},
    // identifier=${contact.paymentIdentifier}")
}
```
