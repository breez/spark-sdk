import BreezSdkSpark

func addContact(sdk: BreezSdk) async throws {
    // ANCHOR: add-contact
    let contact = try await sdk.addContact(
        request: AddContactRequest(
            name: "Alice",
            paymentIdentifier: "alice@example.com"
        ))
    print("Contact added: \(contact)")
    // ANCHOR_END: add-contact
}

func updateContact(sdk: BreezSdk) async throws {
    // ANCHOR: update-contact
    let contactId = "contact-id"
    let contact = try await sdk.updateContact(
        request: UpdateContactRequest(
            id: contactId,
            name: "Alice Smith",
            paymentIdentifier: "alice.smith@example.com"
        ))
    print("Contact updated: \(contact)")
    // ANCHOR_END: update-contact
}

func deleteContact(sdk: BreezSdk) async throws {
    // ANCHOR: delete-contact
    let contactId = "contact-id"
    try await sdk.deleteContact(id: contactId)
    print("Contact deleted")
    // ANCHOR_END: delete-contact
}

func listContacts(sdk: BreezSdk) async throws {
    // ANCHOR: list-contacts
    // List contacts with pagination (e.g., 10 contacts starting from offset 0)
    // Optionally filter by exact name match
    let contacts = try await sdk.listContacts(
        request: ListContactsRequest(
            name: nil, // Set to some("Alice") to filter by name
            offset: 0,
            limit: 10
        ))
    for contact in contacts {
        print("Contact: id=\(contact.id), name=\(contact.name), identifier=\(contact.paymentIdentifier)")
    }
    // ANCHOR_END: list-contacts
}
