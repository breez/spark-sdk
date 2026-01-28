use crate::SdkError;

/// Validates contact input, returns trimmed name on success
pub fn validate_contact_input(name: &str, payment_identifier: &str) -> Result<String, SdkError> {
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
    let payment_identifier = payment_identifier.trim();
    if payment_identifier.is_empty() {
        return Err(SdkError::InvalidInput(
            "Payment identifier cannot be empty".to_string(),
        ));
    }
    if payment_identifier.len() > 1000 {
        return Err(SdkError::InvalidInput(
            "Payment identifier cannot exceed 1000 characters".to_string(),
        ));
    }
    Ok(name)
}
