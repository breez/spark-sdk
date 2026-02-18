use crate::{
    AddContactRequest, Contact, ListContactsRequest, UpdateContactRequest, error::SdkError,
    utils::contacts_validation::validate_contact_input,
};

use super::BreezSdk;

impl BreezSdk {
    /// Checks that no other contact (excluding `exclude_id`) shares the same
    /// `(name, payment_identifier)` pair.
    async fn check_contact_duplicate(
        &self,
        name: &str,
        payment_identifier: &str,
        exclude_id: Option<&str>,
    ) -> Result<(), SdkError> {
        let contacts = self
            .storage
            .list_contacts(ListContactsRequest {
                name: Some(name.to_string()),
                ..Default::default()
            })
            .await?;
        let duplicate = contacts.iter().any(|c| {
            c.payment_identifier == payment_identifier && exclude_id.is_none_or(|id| c.id != id)
        });
        if duplicate {
            return Err(SdkError::InvalidInput("Duplicate entry".to_string()));
        }
        Ok(())
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Adds a new contact.
    ///
    /// # Arguments
    ///
    /// * `request` - The request containing the contact details
    ///
    /// # Returns
    ///
    /// The created contact or an error
    pub async fn add_contact(&self, request: AddContactRequest) -> Result<Contact, SdkError> {
        let name = validate_contact_input(&request.name, &request.payment_identifier)?;
        let payment_identifier = request.payment_identifier.trim().to_string();

        self.check_contact_duplicate(&name, &payment_identifier, None)
            .await?;

        let now = web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .map_err(|_| SdkError::Generic("Failed to get current time".to_string()))?;

        let contact = Contact {
            id: uuid::Uuid::now_v7().to_string(),
            name,
            payment_identifier,
            created_at: now,
            updated_at: now,
        };

        self.storage.insert_contact(contact.clone()).await?;
        Ok(contact)
    }

    /// Updates an existing contact.
    ///
    /// # Arguments
    ///
    /// * `request` - The request containing the updated contact details
    ///
    /// # Returns
    ///
    /// The updated contact or an error
    pub async fn update_contact(&self, request: UpdateContactRequest) -> Result<Contact, SdkError> {
        let name = validate_contact_input(&request.name, &request.payment_identifier)?;
        let payment_identifier = request.payment_identifier.trim().to_string();

        let existing = self.storage.get_contact(request.id.clone()).await?;

        self.check_contact_duplicate(&name, &payment_identifier, Some(&request.id))
            .await?;

        let now = web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .map_err(|_| SdkError::Generic("Failed to get current time".to_string()))?;

        let contact = Contact {
            id: request.id,
            name,
            payment_identifier,
            created_at: existing.created_at,
            updated_at: now,
        };

        self.storage.insert_contact(contact.clone()).await?;
        Ok(contact)
    }

    /// Deletes a contact by its ID.
    ///
    /// # Arguments
    ///
    /// * `id` - The ID of the contact to delete
    ///
    /// # Returns
    ///
    /// Success or an error
    pub async fn delete_contact(&self, id: String) -> Result<(), SdkError> {
        self.storage.delete_contact(id).await?;
        Ok(())
    }

    /// Lists contacts with optional pagination.
    ///
    /// # Arguments
    ///
    /// * `request` - The request containing optional pagination parameters
    ///
    /// # Returns
    ///
    /// A list of contacts or an error
    pub async fn list_contacts(
        &self,
        request: ListContactsRequest,
    ) -> Result<Vec<Contact>, SdkError> {
        let contacts = self.storage.list_contacts(request).await?;
        Ok(contacts)
    }
}
