# Managing contacts

Contacts allow you to save Lightning addresses for quick access. Each contact stores a name and a Lightning address, making it easy to send payments to frequently used recipients. Contacts are synced across all instances of the SDK.

## Adding a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_contact

To add a new contact, provide a name and a Lightning address.

```csharp
var contact = await sdk.AddContact(request: new AddContactRequest(
    name: "Alice",
    paymentIdentifier: "alice@example.com"
));
Console.WriteLine($"Contact added: {contact}");
```



## Updating a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.update_contact

To update an existing contact, provide the contact ID along with the new name and Lightning address.

```csharp
var contactId = "contact-id";
var contact = await sdk.UpdateContact(request: new UpdateContactRequest(
    id: contactId,
    name: "Alice Smith",
    paymentIdentifier: "alice.smith@example.com"
));
Console.WriteLine($"Contact updated: {contact}");
```



## Deleting a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.delete_contact

To remove a contact, pass its ID to the delete method.

```csharp
var contactId = "contact-id";
await sdk.DeleteContact(id: contactId);
Console.WriteLine("Contact deleted");
```



## Listing contacts

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_contacts

To retrieve your saved contacts, use the list method. The results support pagination through offset and limit parameters.

```csharp
// List contacts with pagination (e.g., 10 contacts starting from offset 0)
var contacts = await sdk.ListContacts(request: new ListContactsRequest(
    offset: 0,
    limit: 10
));
foreach (var contact in contacts)
{
    Console.WriteLine($"Contact: id={contact.id}, name={contact.name}, " +
                    $"identifier={contact.paymentIdentifier}");
}
```
