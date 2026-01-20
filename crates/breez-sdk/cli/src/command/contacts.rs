use breez_sdk_spark::{AddContactRequest, BreezSdk, ListContactsRequest, UpdateContactRequest};
use clap::Subcommand;

use crate::command::print_value;

#[derive(Clone, Debug, Subcommand)]
pub enum ContactCommand {
    /// Add a new contact
    Add {
        /// Name of the contact
        name: String,
        /// Payment identifier (Lightning address, BOLT12 offer, BIP353 address, etc.)
        payment_identifier: String,
    },
    /// Update an existing contact
    Update {
        /// ID of the contact to update
        id: String,
        /// New name for the contact
        name: String,
        /// New payment identifier (Lightning address, BOLT12 offer, BIP353 address, etc.)
        payment_identifier: String,
    },
    /// Delete a contact
    Delete {
        /// ID of the contact to delete
        id: String,
    },
    /// List contacts
    List {
        /// Filter by exact name match
        #[clap(long)]
        name: Option<String>,
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
            payment_identifier,
        } => {
            let contact = sdk
                .add_contact(AddContactRequest {
                    name,
                    payment_identifier,
                })
                .await?;
            print_value(&contact)?;
            Ok(true)
        }
        ContactCommand::Update {
            id,
            name,
            payment_identifier,
        } => {
            let contact = sdk
                .update_contact(UpdateContactRequest {
                    id,
                    name,
                    payment_identifier,
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
        ContactCommand::List {
            name,
            offset,
            limit,
        } => {
            let contacts = sdk
                .list_contacts(ListContactsRequest {
                    name,
                    offset,
                    limit,
                })
                .await?;
            print_value(&contacts)?;
            Ok(true)
        }
    }
}
