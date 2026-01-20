use crate::SdkError;

/// Validates contact input, returns trimmed name on success
pub fn validate_contact_input(name: &str, address: &str) -> Result<String, SdkError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(SdkError::InvalidInput(
            "Contact name cannot be empty".to_string(),
        ));
    }
    if name.len() > 100 {
        return Err(SdkError::InvalidInput(
            "Contact name cannot exceed 100 characters".to_string(),
        ));
    }
    if address.len() > 320 {
        return Err(SdkError::InvalidInput(
            "Lightning address cannot exceed 320 characters".to_string(),
        ));
    }
    if !breez_sdk_common::input::validate_lightning_address_format(address) {
        return Err(SdkError::InvalidInput(
            "Invalid lightning address format".to_string(),
        ));
    }
    Ok(name)
}
