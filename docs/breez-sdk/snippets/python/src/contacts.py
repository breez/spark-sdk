import logging
from breez_sdk_spark import (
    BreezSdk,
    AddContactRequest,
    ListContactsRequest,
    UpdateContactRequest,
)


async def add_contact(sdk: BreezSdk):
    # ANCHOR: add-contact
    contact = await sdk.add_contact(
        request=AddContactRequest(
            name="Alice",
            payment_identifier="alice@example.com",
        )
    )
    logging.debug(f"Contact added: {contact}")
    # ANCHOR_END: add-contact


async def update_contact(sdk: BreezSdk):
    # ANCHOR: update-contact
    contact_id = "contact-id"
    contact = await sdk.update_contact(
        request=UpdateContactRequest(
            id=contact_id,
            name="Alice Smith",
            payment_identifier="alice.smith@example.com",
        )
    )
    logging.debug(f"Contact updated: {contact}")
    # ANCHOR_END: update-contact


async def delete_contact(sdk: BreezSdk):
    # ANCHOR: delete-contact
    contact_id = "contact-id"
    await sdk.delete_contact(id=contact_id)
    logging.debug("Contact deleted")
    # ANCHOR_END: delete-contact


async def list_contacts(sdk: BreezSdk):
    # ANCHOR: list-contacts
    # List contacts with pagination (e.g., 10 contacts starting from offset 0)
    # Optionally filter by exact name match
    contacts = await sdk.list_contacts(
        request=ListContactsRequest(
            name=None,  # Set to Some("Alice") to filter by name
            offset=0,
            limit=10,
        )
    )
    for contact in contacts:
        logging.debug(
            f"Contact: id={contact.id}, name={contact.name}, "
            f"identifier={contact.payment_identifier}"
        )
    # ANCHOR_END: list-contacts
