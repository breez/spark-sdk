import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<Contact> addContact(BreezSdk sdk) async {
  // ANCHOR: add-contact
  AddContactRequest request = AddContactRequest(
    name: "Alice",
    paymentIdentifier: "alice@example.com",
  );
  Contact contact = await sdk.addContact(request: request);
  print("Contact added: $contact");
  // ANCHOR_END: add-contact
  return contact;
}

Future<Contact> updateContact(BreezSdk sdk) async {
  // ANCHOR: update-contact
  String contactId = "contact-id";
  UpdateContactRequest request = UpdateContactRequest(
    id: contactId,
    name: "Alice Smith",
    paymentIdentifier: "alice.smith@example.com",
  );
  Contact contact = await sdk.updateContact(request: request);
  print("Contact updated: $contact");
  // ANCHOR_END: update-contact
  return contact;
}

Future<void> deleteContact(BreezSdk sdk) async {
  // ANCHOR: delete-contact
  String contactId = "contact-id";
  await sdk.deleteContact(id: contactId);
  print("Contact deleted");
  // ANCHOR_END: delete-contact
}

Future<List<Contact>> listContacts(BreezSdk sdk) async {
  // ANCHOR: list-contacts
  // List contacts with pagination (e.g., 10 contacts starting from offset 0)
  // Optionally filter by exact name match
  ListContactsRequest request = ListContactsRequest(
    name: null, // Set to "Alice" to filter by name
    offset: 0,
    limit: 10,
  );
  List<Contact> contacts = await sdk.listContacts(request: request);
  for (Contact contact in contacts) {
    print("Contact: id=${contact.id}, name=${contact.name}, identifier=${contact.paymentIdentifier}");
  }
  // ANCHOR_END: list-contacts
  return contacts;
}
