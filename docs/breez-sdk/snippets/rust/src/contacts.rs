use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

pub(crate) async fn add_contact(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: add-contact
    let contact = sdk
        .add_contact(AddContactRequest {
            name: "Alice".to_string(),
            payment_identifier: "alice@example.com".to_string(),
        })
        .await?;
    info!("Contact added: {:?}", contact);
    // ANCHOR_END: add-contact
    Ok(())
}

pub(crate) async fn update_contact(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: update-contact
    let contact_id = "contact-id".to_string();
    let contact = sdk
        .update_contact(UpdateContactRequest {
            id: contact_id,
            name: "Alice Smith".to_string(),
            payment_identifier: "alice.smith@example.com".to_string(),
        })
        .await?;
    info!("Contact updated: {:?}", contact);
    // ANCHOR_END: update-contact
    Ok(())
}

pub(crate) async fn delete_contact(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: delete-contact
    let contact_id = "contact-id".to_string();
    sdk.delete_contact(contact_id).await?;
    info!("Contact deleted");
    // ANCHOR_END: delete-contact
    Ok(())
}

pub(crate) async fn list_contacts(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: list-contacts
    // List contacts with pagination (e.g., 10 contacts starting from offset 0)
    // Optionally filter by exact name match
    let contacts = sdk
        .list_contacts(ListContactsRequest {
            name: None, // Set to Some("Alice") to filter by name
            offset: Some(0),
            limit: Some(10),
        })
        .await?;
    for contact in contacts {
        info!(
            "Contact: id={}, name={}, identifier={}",
            contact.id, contact.name, contact.payment_identifier
        );
    }
    // ANCHOR_END: list-contacts
    Ok(())
}
