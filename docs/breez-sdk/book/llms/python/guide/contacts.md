# Managing contacts

Contacts allow you to save Lightning addresses for quick access. Each contact stores a name and a Lightning address, making it easy to send payments to frequently used recipients. Contacts are synced across all instances of the SDK.

## Adding a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_contact

To add a new contact, provide a name and a Lightning address.

```python
contact = await sdk.add_contact(
    request=AddContactRequest(
        name="Alice",
        payment_identifier="alice@example.com",
    )
)
logging.debug(f"Contact added: {contact}")
```



## Updating a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.update_contact

To update an existing contact, provide the contact ID along with the new name and Lightning address.

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



## Deleting a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.delete_contact

To remove a contact, pass its ID to the delete method.

```python
contact_id = "contact-id"
await sdk.delete_contact(id=contact_id)
logging.debug("Contact deleted")
```



## Listing contacts

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_contacts

To retrieve your saved contacts, use the list method. The results support pagination through offset and limit parameters.

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
