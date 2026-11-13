# Managing contacts

Contacts allow you to save Lightning addresses for quick access. Each contact stores a name and a Lightning address, making it easy to send payments to frequently used recipients. Contacts are synced across all instances of the SDK.

## Adding a contact

To add a new contact, provide a name and a Lightning address.

```swift
let contact = try await sdk.addContact(
    request: AddContactRequest(
        name: "Alice",
        paymentIdentifier: "alice@example.com"
    ))
print("Contact added: \(contact)")
```



## Updating a contact

To update an existing contact, provide the contact ID along with the new name and Lightning address.

```swift
let contactId = "contact-id"
let contact = try await sdk.updateContact(
    request: UpdateContactRequest(
        id: contactId,
        name: "Alice Smith",
        paymentIdentifier: "alice.smith@example.com"
    ))
print("Contact updated: \(contact)")
```



## Deleting a contact

To remove a contact, pass its ID to the delete method.

```swift
let contactId = "contact-id"
try await sdk.deleteContact(id: contactId)
print("Contact deleted")
```



## Listing contacts

To retrieve your saved contacts, use the list method. The results support pagination through offset and limit parameters.

```swift
// List contacts with pagination (e.g., 10 contacts starting from offset 0)
let contacts = try await sdk.listContacts(
    request: ListContactsRequest(
        offset: 0,
        limit: 10
    ))
for contact in contacts {
    print(
        "Contact: id=\(contact.id), name=\(contact.name), "
            + "identifier=\(contact.paymentIdentifier)")
}
```
