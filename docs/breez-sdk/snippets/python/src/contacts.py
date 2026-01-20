import logging
from breez_sdk_spark import (
    BreezSdk,
    AddContactRequest,
    ListContactsRequest,
    UpdateContactRequest,
)


async def add_contact(sdk: BreezSdk):
    # ANCHOR: add-contact
    try:
        contact = await sdk.add_contact(
            request=AddContactRequest(
                name="Alice",
                lightning_address="alice@example.com",
            )
        )
        logging.debug(f"Contact added: {contact}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: add-contact


async def update_contact(sdk: BreezSdk):
    # ANCHOR: update-contact
    try:
        contact_id = "contact-id"
        contact = await sdk.update_contact(
            request=UpdateContactRequest(
                id=contact_id,
                name="Alice Smith",
                lightning_address="alice.smith@example.com",
            )
        )
        logging.debug(f"Contact updated: {contact}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: update-contact


async def delete_contact(sdk: BreezSdk):
    # ANCHOR: delete-contact
    try:
        contact_id = "contact-id"
        await sdk.delete_contact(id=contact_id)
        logging.debug("Contact deleted")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: delete-contact


async def list_contacts(sdk: BreezSdk):
    # ANCHOR: list-contacts
    # List contacts with pagination (e.g., 10 contacts starting from offset 0)
    try:
        contacts = await sdk.list_contacts(
            request=ListContactsRequest(
                offset=0,
                limit=10,
            )
        )
        for contact in contacts:
            logging.debug(
                f"Contact: id={contact.id}, name={contact.name}, "
                f"address={contact.lightning_address}"
            )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: list-contacts
