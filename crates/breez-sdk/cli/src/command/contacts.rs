use breez_sdk_spark::{AddContactRequest, BreezSdk, ListContactsRequest, UpdateContactRequest};
use clap::Subcommand;

use crate::command::print_value;

#[derive(Clone, Debug, Subcommand)]
pub enum ContactCommand {
    /// Add a new contact
    Add {
        /// Name of the contact
        name: String,
        /// Lightning address of the contact
        lightning_address: String,
    },
    /// Update an existing contact
    Update {
        /// ID of the contact to update
        id: String,
        /// New name for the contact
        name: String,
        /// New lightning address for the contact
        lightning_address: String,
    },
    /// Delete a contact
    Delete {
        /// ID of the contact to delete
        id: String,
    },
    /// List contacts
    List {
        /// Number of contacts to skip
        offset: Option<u32>,
        /// Maximum number of contacts to return
        limit: Option<u32>,
    },
}

pub async fn handle_command(
    sdk: &BreezSdk,
    command: ContactCommand,
) -> Result<bool, anyhow::Error> {
    match command {
        ContactCommand::Add {
            name,
            lightning_address,
        } => {
            let contact = sdk
                .add_contact(AddContactRequest {
                    name,
                    lightning_address,
                })
                .await?;
            print_value(&contact)?;
            Ok(true)
        }
        ContactCommand::Update {
            id,
            name,
            lightning_address,
        } => {
            let contact = sdk
                .update_contact(UpdateContactRequest {
                    id,
                    name,
                    lightning_address,
                })
                .await?;
            print_value(&contact)?;
            Ok(true)
        }
        ContactCommand::Delete { id } => {
            sdk.delete_contact(id).await?;
            println!("Contact deleted successfully");
            Ok(true)
        }
        ContactCommand::List { offset, limit } => {
            let contacts = sdk
                .list_contacts(ListContactsRequest { offset, limit })
                .await?;
            print_value(&contacts)?;
            Ok(true)
        }
    }
}
