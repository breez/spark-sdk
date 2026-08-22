# Managing contacts

Contacts allow you to save Lightning addresses for quick access. Each contact stores a name and a Lightning address, making it easy to send payments to frequently used recipients. Contacts are synced across all instances of the SDK.

## Adding a contact

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_contact

To add a new contact, provide a name and a Lightning address.

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
