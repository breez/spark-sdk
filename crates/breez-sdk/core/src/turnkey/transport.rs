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

/// Wraps an activity intent in the `{type, timestampMs, organizationId,
/// parameters}` envelope Turnkey's submit endpoints expect.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityEnvelope<'a, P> {
    #[serde(rename = "type")]
    activity_type: &'a str,
    timestamp_ms: String,
    organization_id: &'a str,
    parameters: P,
}

fn current_timestamp_ms() -> String {
    platform_utils::time::SystemTime::now()
        .duration_since(platform_utils::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
        .to_string()
}

/// Whether a failed request is worth retrying: network/transport failures and
/// transient server responses (408, 429 rate limit, 5xx). Other 4xx (bad
/// request, auth, 409 already-exists) won't change on retry, so they propagate.
fn is_retryable(error: &TurnkeyError) -> bool {
    match error {
        TurnkeyError::Transport(_) => true,
        TurnkeyError::Http { status, .. } => matches!(*status, 408 | 429 | 500..=599),
        _ => false,
    }
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
    ///
    /// Transient failures (network errors, 429 rate limits, 5xx) are retried
    /// with exponential backoff per [`TurnkeyRetryConfig`]. The body is stamped
    /// once and replayed verbatim: the signature covers only the body, and
    /// activity submits carry a timestamp that Turnkey fingerprints for dedup,
    /// so a retried submit never double-executes. Without response-header access
    /// we can't honor `Retry-After`, so 429s fall back to the backoff schedule.
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

        let mut attempt: u32 = 0;
        loop {
            match self.send_once(&url, &headers, &body).await {
                Ok(resp) => return Ok(resp),
                Err(e) if attempt < self.retry.max_retries && is_retryable(&e) => {
                    attempt = attempt.saturating_add(1);
                    sleep(self.retry.delay_for_attempt(attempt)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// A single HTTP attempt: POST the already-stamped body and deserialize the
    /// success response, mapping transport and non-2xx outcomes to errors.
    async fn send_once<Resp>(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        body: &str,
    ) -> Result<Resp, TurnkeyError>
    where
        Resp: DeserializeOwned,
    {
        let resp = self
            .http
            .post(
                url.to_string(),
                Some(headers.clone()),
                Some(body.to_string()),
            )
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
                    attempt = attempt.saturating_add(1);
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

    /// Submits `parameters` as `activity_type` to `path`, polls to completion,
    /// and deserializes `activity.result.{result_field}` into `R`.
    pub(crate) async fn submit_activity<P, R>(
        &self,
        path: &str,
        activity_type: &str,
        parameters: P,
        result_field: &str,
    ) -> Result<R, TurnkeyError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let envelope = ActivityEnvelope {
            activity_type,
            timestamp_ms: current_timestamp_ms(),
            organization_id: &self.organization_id,
            parameters,
        };
        let activity = self.process_activity(path, &envelope).await?;
        let value = activity.result.get(result_field).ok_or_else(|| {
            TurnkeyError::UnexpectedResponse(format!("missing {result_field} in activity result"))
        })?;
        serde_json::from_value(value.clone()).map_err(|e| TurnkeyError::Deserialize(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_retryable_errors() {
        assert!(is_retryable(&TurnkeyError::Transport(
            "connection reset".into()
        )));
        for status in [408u16, 429, 500, 502, 503, 504] {
            assert!(
                is_retryable(&TurnkeyError::Http {
                    status,
                    body: String::new()
                }),
                "{status} should be retryable"
            );
        }
        for status in [400u16, 401, 403, 404, 409] {
            assert!(
                !is_retryable(&TurnkeyError::Http {
                    status,
                    body: String::new()
                }),
                "{status} should not be retryable"
            );
        }
        assert!(!is_retryable(&TurnkeyError::Deserialize("bad json".into())));
    }
}
