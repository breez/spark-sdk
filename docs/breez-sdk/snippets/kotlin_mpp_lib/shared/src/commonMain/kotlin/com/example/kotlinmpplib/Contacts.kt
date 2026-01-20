package com.example.kotlinmpplib

import breez_sdk_spark.*

class Contacts {
    suspend fun addContact(sdk: BreezSdk) {
        // ANCHOR: add-contact
        try {
            val contact = sdk.addContact(AddContactRequest(
                name = "Alice",
                lightningAddress = "alice@example.com"
            ))
            // Log.v("Breez", "Contact added: $contact")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: add-contact
    }

    suspend fun updateContact(sdk: BreezSdk) {
        // ANCHOR: update-contact
        try {
            val contactId = "contact-id"
            val contact = sdk.updateContact(UpdateContactRequest(
                id = contactId,
                name = "Alice Smith",
                lightningAddress = "alice.smith@example.com"
            ))
            // Log.v("Breez", "Contact updated: $contact")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: update-contact
    }

    suspend fun deleteContact(sdk: BreezSdk) {
        // ANCHOR: delete-contact
        try {
            val contactId = "contact-id"
            sdk.deleteContact(contactId)
            // Log.v("Breez", "Contact deleted")
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: delete-contact
    }

    suspend fun listContacts(sdk: BreezSdk) {
        // ANCHOR: list-contacts
        // List contacts with pagination (e.g., 10 contacts starting from offset 0)
        try {
            val contacts = sdk.listContacts(ListContactsRequest(
                offset = 0u,
                limit = 10u
            ))
            for (contact in contacts) {
                // Log.v("Breez", "Contact: id=${contact.id}, name=${contact.name}, address=${contact.lightningAddress}")
            }
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: list-contacts
    }
}
