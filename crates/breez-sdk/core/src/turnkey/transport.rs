//! Minimal cross-platform Turnkey API client on [`platform_utils::HttpClient`]
//! (native and wasm).
//!
//! Mirrors `turnkey_client`'s request flow: serialize, secp256k1-stamp into the
//! `X-Stamp` header, POST to `{base_url}{path}`, and for activities poll until
//! the activity reaches a terminal status.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use platform_utils::tokio::time::sleep;
use platform_utils::{HttpClient, HttpResponse};
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

/// Query path for fetching a single activity by id, used to poll a pending
/// activity without resubmitting it.
const GET_ACTIVITY_PATH: &str = "/public/v1/query/get_activity";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GetActivityRequest<'a> {
    organization_id: &'a str,
    activity_id: &'a str,
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

/// Whether a retry whose wait takes `delay` still fits the request's time
/// budget after `elapsed` has already been spent. A wait that would end past
/// the deadline is pointless: fail with the error at hand instead.
fn retry_within_budget(elapsed: Duration, delay: Duration, timeout: Duration) -> bool {
    elapsed.saturating_add(delay) < timeout
}

/// Whether an HTTP status is worth retrying: transient server responses (408,
/// 429 rate limit, 5xx). Other statuses (400/401/403/404, and the 409
/// already-exists used for idempotent account creation) won't change on retry.
/// Transport (network) failures are retried separately.
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500..=599)
}

/// The delay a `Retry-After` header requests, in its delta-seconds form. The
/// HTTP-date form is not parsed, so the caller falls back to its backoff
/// schedule for that.
fn retry_after_delay(resp: &HttpResponse) -> Option<Duration> {
    let secs: u64 = resp.header("retry-after")?.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

/// Whether to retry or surface an HTTP 409 (conflict) response.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnConflict {
    /// Retry the request within the retry budget.
    Retry,
    /// Return the 409 to the caller.
    Surface,
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
            base_url: config
                .base_url
                .as_deref()
                .unwrap_or(super::config::DEFAULT_BASE_URL)
                .trim_end_matches('/')
                .to_string(),
            organization_id: config.organization_id.clone(),
            wallet_id: config.wallet_id.clone(),
            stamper: ApiKeyStamper::from_hex(&config.api_private_key, &config.api_public_key)?,
            retry: config.retry.clone().unwrap_or_default(),
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
    /// so a retried submit never double-executes (it returns the original
    /// activity). With [`OnConflict::Retry`], a 409 from such a resubmit (the
    /// original still in flight) is also retried until it converges; with
    /// [`OnConflict::Surface`] the 409 is returned. A server-provided
    /// `Retry-After` overrides the backoff delay. Every retry is bounded by the
    /// configured request timeout: a wait that would end past it (a long
    /// `Retry-After`, or a late attempt with little budget left) fails the
    /// request with the error at hand instead of stalling.
    pub(crate) async fn process_request<Req, Resp>(
        &self,
        path: &str,
        request: &Req,
        on_conflict: OnConflict,
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

        let started = platform_utils::time::Instant::now();
        let timeout = self.retry.request_timeout();
        let mut attempt: u32 = 0;
        loop {
            let outcome = self
                .http
                .post(url.clone(), Some(headers.clone()), Some(body.clone()))
                .await;
            match outcome {
                Ok(resp) if resp.is_success() => {
                    return resp
                        .json::<Resp>()
                        .map_err(|e| TurnkeyError::Deserialize(e.to_string()));
                }
                Ok(resp)
                    if attempt < self.retry.max_retries
                        && (is_retryable_status(resp.status)
                            || (resp.status == 409 && on_conflict == OnConflict::Retry)) =>
                {
                    attempt = attempt.saturating_add(1);
                    let delay = retry_after_delay(&resp)
                        .unwrap_or_else(|| self.retry.delay_for_attempt(attempt));
                    if !retry_within_budget(started.elapsed(), delay, timeout) {
                        return Err(TurnkeyError::Http {
                            status: resp.status,
                            body: resp.body,
                        });
                    }
                    sleep(delay).await;
                }
                Ok(resp) => {
                    return Err(TurnkeyError::Http {
                        status: resp.status,
                        body: resp.body,
                    });
                }
                Err(e) if attempt < self.retry.max_retries => {
                    attempt = attempt.saturating_add(1);
                    let delay = self.retry.delay_for_attempt(attempt);
                    if !retry_within_budget(started.elapsed(), delay, timeout) {
                        return Err(TurnkeyError::Transport(e.to_string()));
                    }
                    sleep(delay).await;
                }
                Err(e) => return Err(TurnkeyError::Transport(e.to_string())),
            }
        }
    }

    /// Submits an activity once, then polls it by id until it reaches a terminal
    /// status, returning the completed [`Activity`].
    ///
    /// Polling fetches the activity by id rather than resubmitting it: Turnkey
    /// records the activity on the first submit and rejects an identical
    /// resubmit with a fingerprint 409, so a pending activity must be polled via
    /// [`Self::get_activity`], not re-sent. Backoff between polls follows the
    /// configured [`TurnkeyRetryConfig`].
    pub(crate) async fn process_activity<Req>(
        &self,
        path: &str,
        request: &Req,
        on_conflict: OnConflict,
    ) -> Result<Activity, TurnkeyError>
    where
        Req: Serialize + ?Sized,
    {
        let response: ActivityResponse = self.process_request(path, request, on_conflict).await?;
        let mut activity = response.activity.ok_or(TurnkeyError::MissingActivity)?;
        let mut attempt: u32 = 0;
        loop {
            match activity.status {
                ActivityStatus::Completed => return Ok(activity),
                ActivityStatus::Pending | ActivityStatus::Created => {
                    if attempt >= self.retry.max_retries {
                        return Err(TurnkeyError::ExceededRetries(attempt));
                    }
                    attempt = attempt.saturating_add(1);
                    sleep(self.retry.delay_for_attempt(attempt)).await;
                    activity = self.get_activity(&activity.id).await?;
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

    /// Fetches an activity by id. An idempotent query (safe to retry), used to
    /// poll a pending activity without resubmitting it.
    async fn get_activity(&self, activity_id: &str) -> Result<Activity, TurnkeyError> {
        let request = GetActivityRequest {
            organization_id: &self.organization_id,
            activity_id,
        };
        // A poll is a read; it never hits the duplicate-submit 409.
        let response: ActivityResponse = self
            .process_request(GET_ACTIVITY_PATH, &request, OnConflict::Surface)
            .await?;
        response.activity.ok_or(TurnkeyError::MissingActivity)
    }

    /// Submits `parameters` as `activity_type` to `path`, polls to completion,
    /// and deserializes `activity.result.{result_field}` into `R`.
    pub(crate) async fn submit_activity<P, R>(
        &self,
        path: &str,
        activity_type: &str,
        parameters: P,
        result_field: &str,
        on_conflict: OnConflict,
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
        let activity = self.process_activity(path, &envelope, on_conflict).await?;
        let value = activity.result.get(result_field).ok_or_else(|| {
            TurnkeyError::UnexpectedResponse(format!("missing {result_field} in activity result"))
        })?;
        serde_json::from_value(value.clone()).map_err(|e| TurnkeyError::Deserialize(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_with_retry_after(value: &str) -> HttpResponse {
        HttpResponse {
            status: 429,
            body: String::new(),
            headers: std::iter::once(("retry-after".to_string(), value.to_string())).collect(),
        }
    }

    #[test]
    fn classifies_retryable_statuses() {
        for status in [408u16, 429, 500, 502, 503, 504] {
            assert!(is_retryable_status(status), "{status} should be retryable");
        }
        for status in [400u16, 401, 403, 404, 409] {
            assert!(
                !is_retryable_status(status),
                "{status} should not be retryable"
            );
        }
    }

    #[test]
    fn parses_retry_after_seconds() {
        assert_eq!(
            retry_after_delay(&response_with_retry_after("5")),
            Some(Duration::from_secs(5))
        );
        // Parsed as-is; the retry loop refuses waits that fall outside the
        // request's time budget rather than capping them.
        assert_eq!(
            retry_after_delay(&response_with_retry_after("99999")),
            Some(Duration::from_secs(99999))
        );
        // The HTTP-date form is not parsed; the caller falls back to backoff.
        assert_eq!(
            retry_after_delay(&response_with_retry_after("Wed, 21 Oct 2015 07:28:00 GMT")),
            None
        );
        let no_header = HttpResponse {
            status: 429,
            body: String::new(),
            headers: HashMap::new(),
        };
        assert_eq!(retry_after_delay(&no_header), None);
    }

    #[test]
    fn refuses_retries_past_the_request_timeout() {
        let timeout = Duration::from_mins(1);
        // Fresh request, short wait: retry.
        assert!(retry_within_budget(
            Duration::ZERO,
            Duration::from_secs(5),
            timeout
        ));
        // A Retry-After longer than the whole budget: never retry.
        assert!(!retry_within_budget(
            Duration::ZERO,
            Duration::from_mins(10),
            timeout
        ));
        // Late attempt: a wait that fit earlier no longer does.
        assert!(!retry_within_budget(
            Duration::from_secs(58),
            Duration::from_secs(5),
            timeout
        ));
        // Budget already exhausted.
        assert!(!retry_within_budget(
            Duration::from_mins(1),
            Duration::ZERO,
            timeout
        ));
    }
}
