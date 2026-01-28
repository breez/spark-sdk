package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func AddContact(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.Contact, error) {
	// ANCHOR: add-contact
	contact, err := sdk.AddContact(breez_sdk_spark.AddContactRequest{
		Name:              "Alice",
		PaymentIdentifier: "alice@example.com",
	})
	if err != nil {
		return nil, err
	}

	log.Printf("Contact added: %v", contact)
	// ANCHOR_END: add-contact
	return &contact, nil
}

func UpdateContact(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.Contact, error) {
	// ANCHOR: update-contact
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
	// ANCHOR_END: update-contact
	return &contact, nil
}

func DeleteContact(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: delete-contact
	contactId := "contact-id"
	err := sdk.DeleteContact(contactId)
	if err != nil {
		return err
	}

	log.Printf("Contact deleted")
	// ANCHOR_END: delete-contact
	return nil
}

func ListContacts(sdk *breez_sdk_spark.BreezSdk) ([]breez_sdk_spark.Contact, error) {
	// ANCHOR: list-contacts
	// List contacts with pagination (e.g., 10 contacts starting from offset 0)
	// Optionally filter by exact name match
	offset := uint32(0)
	limit := uint32(10)
	contacts, err := sdk.ListContacts(breez_sdk_spark.ListContactsRequest{
		Name:   nil, // Set to &"Alice" to filter by name
		Offset: &offset,
		Limit:  &limit,
	})
	if err != nil {
		return nil, err
	}

	for _, contact := range contacts {
		log.Printf("Contact: id=%v, name=%v, identifier=%v", contact.Id, contact.Name, contact.PaymentIdentifier)
	}
	// ANCHOR_END: list-contacts
	return contacts, nil
}
