package com.example.kotlinmpplib

import breez_sdk_spark.*

class Contacts {
    suspend fun addContact(sdk: BreezSdk) {
        // ANCHOR: add-contact
        val contact = sdk.addContact(AddContactRequest(
            name = "Alice",
            paymentIdentifier = "alice@example.com"
        ))
        // Log.v("Breez", "Contact added: $contact")
        // ANCHOR_END: add-contact
    }

    suspend fun updateContact(sdk: BreezSdk) {
        // ANCHOR: update-contact
        val contactId = "contact-id"
        val contact = sdk.updateContact(UpdateContactRequest(
            id = contactId,
            name = "Alice Smith",
            paymentIdentifier = "alice.smith@example.com"
        ))
        // Log.v("Breez", "Contact updated: $contact")
        // ANCHOR_END: update-contact
    }

    suspend fun deleteContact(sdk: BreezSdk) {
        // ANCHOR: delete-contact
        val contactId = "contact-id"
        sdk.deleteContact(contactId)
        // Log.v("Breez", "Contact deleted")
        // ANCHOR_END: delete-contact
    }

    suspend fun listContacts(sdk: BreezSdk) {
        // ANCHOR: list-contacts
        // List contacts with pagination (e.g., 10 contacts starting from offset 0)
        // Optionally filter by exact name match
        val contacts = sdk.listContacts(ListContactsRequest(
            name = null, // Set to Some("Alice") to filter by name
            offset = 0u,
            limit = 10u
        ))
        for (contact in contacts) {
            // Log.v("Breez", "Contact: id=${contact.id}, name=${contact.name}, identifier=${contact.paymentIdentifier}")
        }
        // ANCHOR_END: list-contacts
    }
}
