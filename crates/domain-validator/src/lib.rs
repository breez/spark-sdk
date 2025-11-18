use std::collections::HashSet;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainValidatorError {
    #[error("Domain {0} is not allowed")]
    DomainNotAllowed(String),
}

#[async_trait::async_trait]
pub trait DomainValidator: Send + Sync {
    async fn validate_domain(&self, domain: &str) -> Result<(), DomainValidatorError>;
}

pub struct ListDomainValidator {
    allowed_domains: HashSet<String>,
}

impl ListDomainValidator {
    pub fn new(domains: HashSet<String>) -> Self {
        Self {
            allowed_domains: domains,
        }
    }
}

#[async_trait::async_trait]
impl DomainValidator for ListDomainValidator {
    async fn validate_domain(&self, domain: &str) -> Result<(), DomainValidatorError> {
        let domain_lower = domain.to_lowercase();
        if self.allowed_domains.contains(&domain_lower) {
            Ok(())
        } else {
            Err(DomainValidatorError::DomainNotAllowed(domain.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_list_domain_validator() {
        let domains = HashSet::from(["example.com".to_string(), "test.org".to_string()]);
        let validator = ListDomainValidator::new(domains);

        assert!(validator.validate_domain("example.com").await.is_ok());
        assert!(validator.validate_domain("EXAMPLE.COM").await.is_ok());
        assert!(validator.validate_domain("test.org").await.is_ok());
        assert!(validator.validate_domain("invalid.com").await.is_err());
    }
}
