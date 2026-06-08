//! Minimal cross-platform Turnkey API client on [`platform_utils::HttpClient`]
//! (native and wasm).
//!
//! Mirrors `turnkey_client`'s request flow: serialize, secp256k1-stamp into the
//! `X-Stamp` header, POST to `{base_url}{path}`, and for activities poll until
//! the activity reaches a terminal status.

use std::collections::HashMap;
use std::sync::Arc;

use platform_utils::HttpClient;
use platform_utils::tokio::time::sleep;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::config::{TurnkeyConfig, TurnkeyRetryConfig};
use super::error::TurnkeyError;
use super::stamp::ApiKeyStamper;

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub(crate) enum ActivityStatus {
    #[serde(rename = "ACTIVITY_STATUS_CREATED")]
    Created,
    #[serde(rename = "ACTIVITY_STATUS_PENDING")]
    Pending,
    #[serde(rename = "ACTIVITY_STATUS_CONSENSUS_NEEDED")]
    ConsensusNeeded,
    #[serde(rename = "ACTIVITY_STATUS_COMPLETED")]
    Completed,
    #[serde(rename = "ACTIVITY_STATUS_FAILED")]
    Failed,
    #[serde(rename = "ACTIVITY_STATUS_REJECTED")]
    Rejected,
    #[serde(other)]
    Unknown,
}

/// A Turnkey activity as returned by the API. `result` is kept as raw JSON; each
/// typed activity call extracts the specific `*Result` field it expects.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Activity {
    pub id: String,
    pub status: ActivityStatus,
    #[serde(default)]
    pub result: serde_json::Value,
    #[serde(default)]
    pub failure: serde_json::Value,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivityResponse {
    activity: Option<Activity>,
}

pub(crate) struct TurnkeyClient {
    http: Arc<dyn HttpClient>,
    base_url: String,
    pub(crate) organization_id: String,
    pub(crate) wallet_id: String,
    stamper: ApiKeyStamper,
    retry: TurnkeyRetryConfig,
}

impl TurnkeyClient {
    pub(crate) fn new(
        config: &TurnkeyConfig,
        http: Arc<dyn HttpClient>,
    ) -> Result<Self, TurnkeyError> {
        Ok(Self {
            http,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            organization_id: config.organization_id.clone(),
            wallet_id: config.wallet_id.clone(),
            stamper: ApiKeyStamper::from_hex(&config.api_private_key, &config.api_public_key)?,
            retry: config.retry.clone(),
        })
    }

    /// Serializes `request`, stamps it, POSTs to `{base_url}{path}`, and
    /// deserializes the JSON response into `Resp`. Used for both queries and the
    /// activity-submit endpoint.
    pub(crate) async fn process_request<Req, Resp>(
        &self,
        path: &str,
        request: &Req,
    ) -> Result<Resp, TurnkeyError>
    where
        Req: Serialize + ?Sized,
        Resp: DeserializeOwned,
    {
        let url = format!("{}{}", self.base_url, path);
        let body =
            serde_json::to_string(request).map_err(|e| TurnkeyError::Serialize(e.to_string()))?;
        let (stamp_name, stamp_value) = self.stamper.stamp(body.as_bytes())?;

        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        headers.insert(stamp_name, stamp_value);

        let resp = self
            .http
            .post(url, Some(headers), Some(body))
            .await
            .map_err(|e| TurnkeyError::Transport(e.to_string()))?;

        if !resp.is_success() {
            return Err(TurnkeyError::Http {
                status: resp.status,
                body: resp.body,
            });
        }
        resp.json::<Resp>()
            .map_err(|e| TurnkeyError::Deserialize(e.to_string()))
    }

    /// Submits an activity and polls until it reaches a terminal status,
    /// returning the completed [`Activity`]. Pending activities are retried with
    /// exponential backoff per the configured [`TurnkeyRetryConfig`].
    pub(crate) async fn process_activity<Req>(
        &self,
        path: &str,
        request: &Req,
    ) -> Result<Activity, TurnkeyError>
    where
        Req: Serialize + ?Sized,
    {
        let mut attempt: u32 = 0;
        loop {
            let response: ActivityResponse = self.process_request(path, request).await?;
            let activity = response.activity.ok_or(TurnkeyError::MissingActivity)?;
            match activity.status {
                ActivityStatus::Completed => return Ok(activity),
                ActivityStatus::Pending | ActivityStatus::Created => {
                    if attempt >= self.retry.max_retries {
                        return Err(TurnkeyError::ExceededRetries(attempt));
                    }
                    attempt += 1;
                    sleep(self.retry.delay_for_attempt(attempt)).await;
                }
                ActivityStatus::Failed => {
                    return Err(TurnkeyError::ActivityFailed(activity.failure.to_string()));
                }
                ActivityStatus::ConsensusNeeded => {
                    return Err(TurnkeyError::ConsensusNeeded(activity.id));
                }
                ActivityStatus::Rejected | ActivityStatus::Unknown => {
                    return Err(TurnkeyError::UnexpectedStatus(format!(
                        "{:?}",
                        activity.status
                    )));
                }
            }
        }
    }
}
