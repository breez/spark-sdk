using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class Contacts
    {
        async Task AddContact(BreezSdk sdk)
        {
            // ANCHOR: add-contact
            var contact = await sdk.AddContact(request: new AddContactRequest(
                name: "Alice",
                paymentIdentifier: "alice@example.com"
            ));
            Console.WriteLine($"Contact added: {contact}");
            // ANCHOR_END: add-contact
        }

        async Task UpdateContact(BreezSdk sdk)
        {
            // ANCHOR: update-contact
            var contactId = "contact-id";
            var contact = await sdk.UpdateContact(request: new UpdateContactRequest(
                id: contactId,
                name: "Alice Smith",
                paymentIdentifier: "alice.smith@example.com"
            ));
            Console.WriteLine($"Contact updated: {contact}");
            // ANCHOR_END: update-contact
        }

        async Task DeleteContact(BreezSdk sdk)
        {
            // ANCHOR: delete-contact
            var contactId = "contact-id";
            await sdk.DeleteContact(id: contactId);
            Console.WriteLine("Contact deleted");
            // ANCHOR_END: delete-contact
        }

        async Task ListContacts(BreezSdk sdk)
        {
            // ANCHOR: list-contacts
            // List contacts with pagination (e.g., 10 contacts starting from offset 0)
            // Optionally filter by exact name match
            var contacts = await sdk.ListContacts(request: new ListContactsRequest(
                name: null, // Set to "Alice" to filter by name
                offset: 0,
                limit: 10
            ));
            foreach (var contact in contacts)
            {
                Console.WriteLine($"Contact: id={contact.id}, name={contact.name}, identifier={contact.paymentIdentifier}");
            }
            // ANCHOR_END: list-contacts
        }
    }
}
