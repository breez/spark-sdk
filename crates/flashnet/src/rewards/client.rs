use std::collections::HashMap;
use std::sync::Arc;

use platform_utils::{ContentType, HttpClient, HttpResponse, add_content_type_header};
use spark_wallet::{PublicKey, SparkWallet};
use tracing::debug;

use super::models::{Endorsements, UpsertEndorsementRequest};
use crate::error::FlashnetError;

pub struct RewardsClient {
    base_url: String,
    spark_wallet: Arc<SparkWallet>,
    http_client: Arc<dyn HttpClient>,
}

impl RewardsClient {
    pub fn new(
        base_url: String,
        spark_wallet: Arc<SparkWallet>,
        http_client: Arc<dyn HttpClient>,
    ) -> Self {
        Self {
            base_url,
            spark_wallet,
            http_client,
        }
    }

    /// The endorsements this wallet has set.
    pub async fn get_endorsements(&self) -> Result<Endorsements, FlashnetError> {
        let endorser = self.spark_wallet.get_identity_public_key();
        debug!("Rewards: GET /v1/rewards/{endorser}/endorsements");
        let url = format!("{}/v1/rewards/{endorser}/endorsements", self.base_url);
        let response = self.http_client.get(url, Some(json_headers())).await?;
        parse_response(response)
    }

    /// Creates or updates this wallet's endorsement of `to`.
    pub async fn upsert_endorsement(
        &self,
        to: &PublicKey,
        ratio_bps: u16,
    ) -> Result<Endorsements, FlashnetError> {
        let endorser = self.spark_wallet.get_identity_public_key();
        let timestamp_nonce = uuid::Uuid::now_v7().to_string();
        let payload = upsert_payload(&endorser, to, ratio_bps, &timestamp_nonce);
        let signature = self.spark_wallet.sign_message(&payload).await?;
        let request = UpsertEndorsementRequest {
            endorser: endorser.to_string(),
            to: to.to_string(),
            ratio_bps,
            timestamp_nonce,
            signature: hex::encode(signature.serialize_compact()),
        };
        let body = serde_json::to_string(&request)
            .map_err(|e| FlashnetError::Generic(format!("Failed to serialize endorsement: {e}")))?;

        debug!(
            "Rewards: POST /v1/rewards/{endorser}/endorsements (to={to}, ratio_bps={ratio_bps})"
        );
        let url = format!("{}/v1/rewards/{endorser}/endorsements", self.base_url);
        let response = self
            .http_client
            .post(url, Some(json_headers()), Some(body))
            .await?;
        parse_response(response)
    }
}

/// The canonical YAML the API verifies the signature against.
fn upsert_payload(endorser: &PublicKey, to: &PublicKey, ratio_bps: u16, nonce: &str) -> String {
    format!(
        "kind: rewards_endorsement\n\
         version: 1\n\
         action: upsert\n\
         endorser: \"{endorser}\"\n\
         to: \"{to}\"\n\
         ratioBps: {ratio_bps}\n\
         timestampNonce: \"{nonce}\""
    )
}

fn json_headers() -> HashMap<String, String> {
    let mut headers = HashMap::new();
    add_content_type_header(&mut headers, ContentType::Json);
    headers
}

/// Error bodies have the shape `{"error":"..."}`.
fn parse_response<D: serde::de::DeserializeOwned>(
    response: HttpResponse,
) -> Result<D, FlashnetError> {
    if !response.is_success() {
        let reason = serde_json::from_str::<serde_json::Value>(&response.body)
            .ok()
            .and_then(|v| v.get("error")?.as_str().map(String::from))
            .unwrap_or(response.body);
        return Err(FlashnetError::Network {
            reason,
            code: Some(response.status),
        });
    }
    response
        .json::<D>()
        .map_err(|e| FlashnetError::Generic(format!("Failed to parse rewards response: {e}")))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    const ENDORSER: &str = "02c137ce8f8344421652b48f327de036f41da00003912992dfd5d61b7d7c6d1976";
    const TO: &str = "0257c86ea9ba12b11b24003aff7b546516dc31861b5b7b9ce5f96c9e3226d15e6d";

    #[test]
    fn upsert_payload_matches_the_canonical_template() {
        let payload = upsert_payload(
            &PublicKey::from_str(ENDORSER).unwrap(),
            &PublicKey::from_str(TO).unwrap(),
            2500,
            "01958d67-2d1a-7a8a-bf8a-5b0a947f8f11",
        );
        let expected = format!(
            "kind: rewards_endorsement\nversion: 1\naction: upsert\nendorser: \"{ENDORSER}\"\nto: \"{TO}\"\nratioBps: 2500\ntimestampNonce: \"01958d67-2d1a-7a8a-bf8a-5b0a947f8f11\""
        );
        assert_eq!(payload, expected);
    }

    #[test]
    fn an_error_body_becomes_its_message_and_status() {
        let response = HttpResponse {
            status: 400,
            body: r#"{"error":"Nonce has already been used"}"#.to_string(),
            headers: HashMap::new(),
        };
        let err = parse_response::<Endorsements>(response).unwrap_err();
        assert!(matches!(
            err,
            FlashnetError::Network { reason, code: Some(400) }
                if reason == "Nonce has already been used"
        ));
    }
}
