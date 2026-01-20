package example

import (
	"errors"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func AddContact(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.Contact, error) {
	// ANCHOR: add-contact
	contact, err := sdk.AddContact(breez_sdk_spark.AddContactRequest{
		Name:             "Alice",
		LightningAddress: "alice@example.com",
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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
		Id:               contactId,
		Name:             "Alice Smith",
		LightningAddress: "alice.smith@example.com",
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}

	log.Printf("Contact deleted")
	// ANCHOR_END: delete-contact
	return nil
}

func ListContacts(sdk *breez_sdk_spark.BreezSdk) ([]breez_sdk_spark.Contact, error) {
	// ANCHOR: list-contacts
	// List contacts with pagination (e.g., 10 contacts starting from offset 0)
	offset := uint32(0)
	limit := uint32(10)
	contacts, err := sdk.ListContacts(breez_sdk_spark.ListContactsRequest{
		Offset: &offset,
		Limit:  &limit,
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	for _, contact := range contacts {
		log.Printf("Contact: id=%v, name=%v, address=%v", contact.Id, contact.Name, contact.LightningAddress)
	}
	// ANCHOR_END: list-contacts
	return contacts, nil
}
