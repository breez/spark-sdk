use std::collections::HashSet;

use async_trait::async_trait;
use domain_validator::{DomainValidator, DomainValidatorError};
use reqwest::Client;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FlyApiError {
    #[error("Failed to fetch certificates: {0}")]
    FetchError(String),
}

pub struct FlyDomainValidator {
    app_name: String,
    api_token: String,
    client: Client,
}

impl FlyDomainValidator {
    pub fn new(app_name: String, api_token: String) -> Self {
        Self {
            app_name,
            api_token,
            client: Client::new(),
        }
    }

    async fn get_certificate_domains(&self) -> Result<HashSet<String>, FlyApiError> {
        let graphql_query = serde_json::json!({
            "query": r#"
                query($appName: String!) {
                    app(name: $appName) {
                        certificates {
                            nodes {
                                hostname
                                clientStatus
                            }
                        }
                    }
                }
            "#,
            "variables": {
                "appName": self.app_name
            }
        });

        let response = self
            .client
            .post("https://api.fly.io/graphql")
            .header("Authorization", format!("Bearer {}", self.api_token))
            .header("Content-Type", "application/json")
            .json(&graphql_query)
            .send()
            .await
            .map_err(|e| FlyApiError::FetchError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(FlyApiError::FetchError(format!(
                "HTTP {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            )));
        }

        let response_data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| FlyApiError::FetchError(e.to_string()))?;

        let mut allowed_domains = HashSet::new();

        if let Some(certificates) = response_data
            .get("data")
            .and_then(|d| d.get("app"))
            .and_then(|a| a.get("certificates"))
            .and_then(|c| c.get("nodes"))
            .and_then(|n| n.as_array())
        {
            for cert in certificates {
                if let Some(hostname) = cert.get("hostname").and_then(|h| h.as_str()) {
                    allowed_domains.insert(hostname.to_lowercase());
                }
            }
        }

        if allowed_domains.is_empty() {
            return Err(FlyApiError::FetchError(
                "No domains found in Fly.io certificates".to_string(),
            ));
        }

        Ok(allowed_domains)
    }
}

#[async_trait]
impl DomainValidator for FlyDomainValidator {
    async fn validate_domain(&self, domain: &str) -> Result<(), DomainValidatorError> {
        let allowed_domains = self
            .get_certificate_domains()
            .await
            .map_err(|e| DomainValidatorError::DomainNotAllowed(e.to_string()))?;

        let domain_lower = domain.to_lowercase();

        if allowed_domains.contains(&domain_lower) {
            Ok(())
        } else {
            Err(DomainValidatorError::DomainNotAllowed(format!(
                "Domain {} not found in Fly.io certificates",
                domain
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fly_domain_validator_creation() {
        let fly_validator = FlyDomainValidator::new("test-app".to_string(), "test-token".to_string());
        assert_eq!(fly_validator.app_name, "test-app");
        assert_eq!(fly_validator.api_token, "test-token");
    }
}
