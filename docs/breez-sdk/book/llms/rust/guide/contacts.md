# Managing contacts

Contacts allow you to save Lightning addresses for quick access. Each contact stores a name and a Lightning address, making it easy to send payments to frequently used recipients. Contacts are synced across all instances of the SDK.

## Adding a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_contact

To add a new contact, provide a name and a Lightning address.

```rust
let contact = sdk
    .add_contact(AddContactRequest {
        name: "Alice".to_string(),
        payment_identifier: "alice@example.com".to_string(),
    })
    .await?;
info!("Contact added: {:?}", contact);
```



## Updating a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.update_contact

To update an existing contact, provide the contact ID along with the new name and Lightning address.

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



## Deleting a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.delete_contact

To remove a contact, pass its ID to the delete method.

```rust
let contact_id = "contact-id".to_string();
sdk.delete_contact(contact_id).await?;
info!("Contact deleted");
```



## Listing contacts

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_contacts

To retrieve your saved contacts, use the list method. The results support pagination through offset and limit parameters.

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
