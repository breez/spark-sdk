use breez_sdk_common::input::validate_lightning_address_format;

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
    if payment_identifier.len() > 2000 {
        return Err(SdkError::InvalidInput(
            "Payment identifier cannot exceed 2000 characters".to_string(),
        ));
    }

    if !validate_lightning_address_format(payment_identifier) {
        return Err(SdkError::InvalidInput(
            "Payment identifier must be a valid lightning address (user@domain)".to_string(),
        ));
    }

    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_NAME: &str = "Alice";

    #[test]
    fn test_accepts_lightning_address() {
        assert!(validate_contact_input(VALID_NAME, "user@domain.com").is_ok());
    }

    #[test]
    fn test_rejects_non_lightning_address() {
        assert!(validate_contact_input(VALID_NAME, "not_a_lightning_address").is_err());
        assert!(validate_contact_input(VALID_NAME, "1andreas3batLhQa2FawWjeyjCqyBzypd").is_err());
    }

    #[test]
    fn test_rejects_oversized_payment_identifier() {
        let long_id = "a".repeat(2001);
        let result = validate_contact_input(VALID_NAME, &long_id);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cannot exceed 2000")
        );
    }

    #[test]
    fn test_rejects_empty_name() {
        assert!(validate_contact_input("", "user@domain.com").is_err());
    }

    #[test]
    fn test_rejects_empty_payment_identifier() {
        assert!(validate_contact_input(VALID_NAME, "").is_err());
    }
}
