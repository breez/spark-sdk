# Managing contacts

Contacts allow you to save Lightning addresses for quick access. Each contact stores a name and a Lightning address, making it easy to send payments to frequently used recipients. Contacts are synced across all instances of the SDK.

## Adding a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_contact

To add a new contact, provide a name and a Lightning address.

### Rust

```rust
let contact = sdk
    .add_contact(AddContactRequest {
        name: "Alice".to_string(),
        payment_identifier: "alice@example.com".to_string(),
    })
    .await?;
info!("Contact added: {:?}", contact);
```

### Swift

```swift
let contact = try await sdk.addContact(
    request: AddContactRequest(
        name: "Alice",
        paymentIdentifier: "alice@example.com"
    ))
print("Contact added: \(contact)")
```

### Kotlin

```kotlin
val contact = sdk.addContact(AddContactRequest(
    name = "Alice",
    paymentIdentifier = "alice@example.com"
))
// Log.v("Breez", "Contact added: $contact")
```

### C#

```csharp
var contact = await sdk.AddContact(request: new AddContactRequest(
    name: "Alice",
    paymentIdentifier: "alice@example.com"
));
Console.WriteLine($"Contact added: {contact}");
```

### Javascript (Wasm)

```typescript
const contact = await sdk.addContact({
  name: 'Alice',
  paymentIdentifier: 'alice@example.com'
})
console.log(`Contact added: ${JSON.stringify(contact)}`)
```

### React Native

```typescript
const contact = await sdk.addContact({
  name: 'Alice',
  paymentIdentifier: 'alice@example.com'
})
console.log(`Contact added: ${JSON.stringify(contact)}`)
```

### Flutter

```dart
AddContactRequest request = AddContactRequest(
  name: "Alice",
  paymentIdentifier: "alice@example.com",
);
Contact contact = await sdk.addContact(request: request);
print("Contact added: $contact");
```

### Python

```python
contact = await sdk.add_contact(
    request=AddContactRequest(
        name="Alice",
        payment_identifier="alice@example.com",
    )
)
logging.debug(f"Contact added: {contact}")
```

### Go

```go
contact, err := sdk.AddContact(breez_sdk_spark.AddContactRequest{
	Name:              "Alice",
	PaymentIdentifier: "alice@example.com",
})
if err != nil {
	return nil, err
}

log.Printf("Contact added: %v", contact)
```



## Updating a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.update_contact

To update an existing contact, provide the contact ID along with the new name and Lightning address.

### Rust

```rust
let contact_id = "contact-id".to_string();
let contact = sdk
    .update_contact(UpdateContactRequest {
        id: contact_id,
        name: "Alice Smith".to_string(),
        payment_identifier: "alice.smith@example.com".to_string(),
    })
    .await?;
info!("Contact updated: {:?}", contact);
```

### Swift

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

### Kotlin

```kotlin
val contactId = "contact-id"
val contact = sdk.updateContact(UpdateContactRequest(
    id = contactId,
    name = "Alice Smith",
    paymentIdentifier = "alice.smith@example.com"
))
// Log.v("Breez", "Contact updated: $contact")
```

### C#

```csharp
var contactId = "contact-id";
var contact = await sdk.UpdateContact(request: new UpdateContactRequest(
    id: contactId,
    name: "Alice Smith",
    paymentIdentifier: "alice.smith@example.com"
));
Console.WriteLine($"Contact updated: {contact}");
```

### Javascript (Wasm)

```typescript
const contactId = 'contact-id'
const contact = await sdk.updateContact({
  id: contactId,
  name: 'Alice Smith',
  paymentIdentifier: 'alice.smith@example.com'
})
console.log(`Contact updated: ${JSON.stringify(contact)}`)
```

### React Native

```typescript
const contactId = 'contact-id'
const contact = await sdk.updateContact({
  id: contactId,
  name: 'Alice Smith',
  paymentIdentifier: 'alice.smith@example.com'
})
console.log(`Contact updated: ${JSON.stringify(contact)}`)
```

### Flutter

```dart
String contactId = "contact-id";
UpdateContactRequest request = UpdateContactRequest(
  id: contactId,
  name: "Alice Smith",
  paymentIdentifier: "alice.smith@example.com",
);
Contact contact = await sdk.updateContact(request: request);
print("Contact updated: $contact");
```

### Python

```python
contact_id = "contact-id"
contact = await sdk.update_contact(
    request=UpdateContactRequest(
        id=contact_id,
        name="Alice Smith",
        payment_identifier="alice.smith@example.com",
    )
)
logging.debug(f"Contact updated: {contact}")
```

### Go

```go
contactId := "contact-id"
contact, err := sdk.UpdateContact(breez_sdk_spark.UpdateContactRequest{
	Id:                contactId,
	Name:              "Alice Smith",
	PaymentIdentifier: "alice.smith@example.com",
})
if err != nil {
	return nil, err
}

log.Printf("Contact updated: %v", contact)
```



## Deleting a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.delete_contact

To remove a contact, pass its ID to the delete method.

### Rust

```rust
let contact_id = "contact-id".to_string();
sdk.delete_contact(contact_id).await?;
info!("Contact deleted");
```

### Swift

```swift
let contactId = "contact-id"
try await sdk.deleteContact(id: contactId)
print("Contact deleted")
```

### Kotlin

```kotlin
val contactId = "contact-id"
sdk.deleteContact(contactId)
// Log.v("Breez", "Contact deleted")
```

### C#

```csharp
var contactId = "contact-id";
await sdk.DeleteContact(id: contactId);
Console.WriteLine("Contact deleted");
```

### Javascript (Wasm)

```typescript
const contactId = 'contact-id'
await sdk.deleteContact(contactId)
console.log('Contact deleted')
```

### React Native

```typescript
const contactId = 'contact-id'
await sdk.deleteContact(contactId)
console.log('Contact deleted')
```

### Flutter

```dart
String contactId = "contact-id";
await sdk.deleteContact(id: contactId);
print("Contact deleted");
```

### Python

```python
contact_id = "contact-id"
await sdk.delete_contact(id=contact_id)
logging.debug("Contact deleted")
```

### Go

```go
contactId := "contact-id"
err := sdk.DeleteContact(contactId)
if err != nil {
	return err
}

log.Printf("Contact deleted")
```



## Listing contacts

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_contacts

To retrieve your saved contacts, use the list method. The results support pagination through offset and limit parameters.

### Rust

```rust
// List contacts with pagination (e.g., 10 contacts starting from offset 0)
let contacts = sdk
    .list_contacts(ListContactsRequest {
        offset: Some(0),
        limit: Some(10),
    })
    .await?;
for contact in contacts {
    info!(
        "Contact: id={}, name={}, identifier={}",
        contact.id, contact.name, contact.payment_identifier
    );
}
```

### Swift

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

### Kotlin

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

### C#

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

### Javascript (Wasm)

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

### React Native

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

### Flutter

```dart
// List contacts with pagination (e.g., 10 contacts starting from offset 0)
ListContactsRequest request = ListContactsRequest(
  offset: 0,
  limit: 10,
);
List<Contact> contacts = await sdk.listContacts(request: request);
for (Contact contact in contacts) {
  print("Contact: id=${contact.id}, name=${contact.name}, identifier=${contact.paymentIdentifier}");
}
```

### Python

```python
# List contacts with pagination (e.g., 10 contacts starting from offset 0)
contacts = await sdk.list_contacts(
    request=ListContactsRequest(
        offset=0,
        limit=10,
    )
)
for contact in contacts:
    logging.debug(
        f"Contact: id={contact.id}, name={contact.name}, "
        f"identifier={contact.payment_identifier}"
    )
```

### Go

```go
// List contacts with pagination (e.g., 10 contacts starting from offset 0)
offset := uint32(0)
limit := uint32(10)
contacts, err := sdk.ListContacts(breez_sdk_spark.ListContactsRequest{
	Offset: &offset,
	Limit:  &limit,
})
if err != nil {
	return nil, err
}

for _, contact := range contacts {
	log.Printf(
		"Contact: id=%v, name=%v, identifier=%v",
		contact.Id,
		contact.Name,
		contact.PaymentIdentifier,
	)
}
```

