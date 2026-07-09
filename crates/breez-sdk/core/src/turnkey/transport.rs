//! Minimal cross-platform Turnkey API client on [`platform_utils::HttpClient`]
//! (native and wasm).
//!
//! Mirrors `turnkey_client`'s request flow: serialize, secp256k1-stamp into the
//! `X-Stamp` header, POST to `{base_url}{path}`, and for activities poll until
//! the activity reaches a terminal status.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use platform_utils::time::Instant;
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

/// Default cap on requests per second when `TurnkeyConfig::max_rps` is unset:
/// Turnkey's documented per-suborganization limit of 10 RPS.
const DEFAULT_MAX_RPS: u32 = 10;
/// Absolute floor on the inter-request gap. A mechanism guard, not the operating
/// rate: the gap must stay non-zero so multiplicative back-off can double it, and
/// a misconfigured rate cannot busy-fire.
const PACE_FLOOR: Duration = Duration::from_millis(1);
/// Ceiling on the inter-request gap, so a sustained 429 streak cannot stall the
/// client indefinitely.
const PACE_MAX_GAP: Duration = Duration::from_secs(5);
/// Additive gap reduction applied once per `PACE_RECOVER_STREAK` successes.
const PACE_RECOVER_STEP: Duration = Duration::from_millis(25);
/// Successes between gap reductions. Additive-increase of the rate: recover
/// gently so one lucky window does not immediately re-provoke the limit.
const PACE_RECOVER_STREAK: u32 = 4;

/// Adaptive request pacer shared across all concurrent callers of one Turnkey
/// suborganization. The steady-state rate is the configured `max_rps`, which the
/// pacer never exceeds; a 429 backs it off further (double the gap) and success
/// recovers it (additive) back to that rate. Turnkey returns no `Retry-After`,
/// so a 429 is the only throttle signal. All callers reserve emission slots from
/// one shared cursor, so firing N requests concurrently drains them at the paced
/// rate instead of bursting.
struct AdaptivePacer {
    inner: Mutex<PacerInner>,
}

struct PacerInner {
    /// Earliest time the next request may be sent.
    next_slot: Instant,
    /// Current spacing between successive requests.
    gap: Duration,
    /// Steady-state floor on the gap, derived from the configured max rate.
    min_gap: Duration,
    /// Successes since the last gap reduction or throttle.
    successes: u32,
}

impl AdaptivePacer {
    fn new(max_rps: u32) -> Self {
        // `max_rps` of 0 is rejected at config; `max(1)` plus `checked_div` is a
        // defensive guard so the division can never divide by zero.
        let min_gap = Duration::from_secs(1)
            .checked_div(max_rps.max(1))
            .unwrap_or(PACE_FLOOR)
            .max(PACE_FLOOR);
        Self {
            inner: Mutex::new(PacerInner {
                next_slot: Instant::now(),
                gap: min_gap,
                min_gap,
                successes: 0,
            }),
        }
    }

    /// Reserves the next emission slot, returning a guard that carries the wait
    /// and reclaims the slot if the caller is dropped before sending. The lock is
    /// never held across the wait.
    ///
    /// A dropped-before-commit reservation (the request future cancelled during
    /// the pacing wait) would otherwise leave the cursor advanced with nothing
    /// sent, so a burst of cancellations would push the cursor far ahead and make
    /// the next real request wait needlessly. The guard rolls the cursor back on
    /// such a drop; [`SlotReservation::commit`] disarms it once the request is
    /// actually sent.
    fn reserve(&self) -> SlotReservation<'_> {
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("pacer mutex poisoned");
        let prev_slot = inner.next_slot;
        let slot = prev_slot.max(now);
        let reserved = slot.checked_add(inner.gap).unwrap_or(slot);
        inner.next_slot = reserved;
        SlotReservation {
            pacer: self,
            wait: slot.saturating_duration_since(now),
            prev_slot,
            reserved,
            committed: false,
        }
    }

    /// Records a successful request, narrowing the gap by one step every
    /// `PACE_RECOVER_STREAK` successes (down to the configured rate).
    fn on_success(&self) {
        let mut inner = self.inner.lock().expect("pacer mutex poisoned");
        inner.successes = inner.successes.saturating_add(1);
        if inner.successes >= PACE_RECOVER_STREAK {
            inner.successes = 0;
            let min_gap = inner.min_gap;
            inner.gap = inner.gap.saturating_sub(PACE_RECOVER_STEP).max(min_gap);
        }
    }

    /// Records a 429, doubling the gap (up to `PACE_MAX_GAP`) and pushing the
    /// cursor out so this retry and every future reservation back off. Callers
    /// already waiting on earlier slots keep their schedule; they widen the gap
    /// further if they also 429. Returns the new gap for the retry-budget check.
    fn on_throttle(&self) -> Duration {
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("pacer mutex poisoned");
        inner.successes = 0;
        inner.gap = inner.gap.saturating_mul(2).min(PACE_MAX_GAP);
        inner.next_slot = inner
            .next_slot
            .max(now.checked_add(inner.gap).unwrap_or(now));
        inner.gap
    }
}

/// A reserved emission slot. Holds the wait until the slot opens and, until
/// [`commit`](Self::commit) is called, rolls the pacer cursor back on drop so a
/// cancelled request does not consume its slot.
struct SlotReservation<'a> {
    pacer: &'a AdaptivePacer,
    wait: Duration,
    /// Cursor value before this reservation, restored on rollback.
    prev_slot: Instant,
    /// Cursor value this reservation set. Rollback applies only while the cursor
    /// still holds it: once a later caller has advanced past our slot, their
    /// schedule depends on it, so we leave it in place and reclaim nothing.
    reserved: Instant,
    committed: bool,
}

impl SlotReservation<'_> {
    /// How long the caller must wait before sending.
    fn wait(&self) -> Duration {
        self.wait
    }

    /// Marks the slot as used, so dropping the guard no longer rolls it back.
    /// Called once the request is actually being sent.
    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for SlotReservation<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut inner = self.pacer.inner.lock().expect("pacer mutex poisoned");
        if inner.next_slot == self.reserved {
            inner.next_slot = self.prev_slot;
        }
    }
}

pub(crate) struct TurnkeyClient {
    http: Arc<dyn HttpClient>,
    base_url: String,
    pub(crate) organization_id: String,
    pub(crate) wallet_id: String,
    stamper: ApiKeyStamper,
    retry: TurnkeyRetryConfig,
    pacer: AdaptivePacer,
}

impl TurnkeyClient {
    pub(crate) fn new(
        config: &TurnkeyConfig,
        http: Arc<dyn HttpClient>,
    ) -> Result<Self, TurnkeyError> {
        if config.max_rps == Some(0) {
            return Err(TurnkeyError::InvalidConfig(
                "max_rps must be greater than 0".to_string(),
            ));
        }
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
            pacer: AdaptivePacer::new(config.max_rps.unwrap_or(DEFAULT_MAX_RPS)),
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
    /// request with the error at hand instead of stalling. The timeout is
    /// measured from the first attempt (after the initial paced slot), so time
    /// spent waiting behind other concurrent callers does not eat the retry
    /// budget.
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

        let timeout = self.retry.request_timeout();
        let mut attempt: u32 = 0;
        // Set on the first attempt, after the initial paced slot. Queue wait
        // (waiting our turn behind other concurrent callers) is deliberately
        // excluded from the timeout: it must not consume the request's retry
        // budget, or a deeply-queued request would arrive with none left.
        let mut started: Option<Instant> = None;
        loop {
            // Pace emission to the adaptive rate before every attempt, so
            // concurrent callers self-throttle to Turnkey's per-suborg RPS limit
            // rather than bursting into 429s.
            let reservation = self.pacer.reserve();
            let wait = reservation.wait();
            if !wait.is_zero() {
                sleep(wait).await;
            }
            // Past the pacing wait: the request is about to go out, so keep the
            // slot even if the POST below is later cancelled.
            reservation.commit();
            let started = *started.get_or_insert_with(Instant::now);
            let outcome = self
                .http
                .post(url.clone(), Some(headers.clone()), Some(body.clone()))
                .await;
            match outcome {
                Ok(resp) if resp.is_success() => {
                    self.pacer.on_success();
                    return resp
                        .json::<Resp>()
                        .map_err(|e| TurnkeyError::Deserialize(e.to_string()));
                }
                // A 429 is a rate signal: widen the pacer (Turnkey sends no
                // Retry-After) and let the next reserve() impose the back-off.
                Ok(resp) if resp.status == 429 && attempt < self.retry.max_retries => {
                    attempt = attempt.saturating_add(1);
                    let backoff = self.pacer.on_throttle();
                    if !retry_within_budget(started.elapsed(), backoff, timeout) {
                        return Err(TurnkeyError::Http {
                            status: resp.status,
                            body: resp.body,
                        });
                    }
                }
                // Other transient failures (408, 5xx, conflict-retry) are not
                // rate signals, so back off on the exponential schedule and leave
                // the pacer untouched. 429 is excluded explicitly so this arm's
                // behavior does not depend on the 429 arm above coming first.
                Ok(resp)
                    if attempt < self.retry.max_retries
                        && resp.status != 429
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

    #[test]
    fn pacer_backs_off_on_throttle_and_recovers_on_success() {
        let pacer = AdaptivePacer::new(10);
        // 1s / 10 RPS.
        let min_gap = Duration::from_millis(100);

        // Fresh pacer: the first slot is immediate, the next is spaced by the
        // configured-rate gap. Commit each so the cursor advances.
        let first = pacer.reserve();
        assert_eq!(first.wait(), Duration::ZERO);
        first.commit();
        let second = pacer.reserve();
        assert!(second.wait() > Duration::from_millis(50) && second.wait() <= min_gap);
        second.commit();

        // Each 429 doubles the gap (multiplicative decrease of the rate).
        let g1 = pacer.on_throttle();
        let g2 = pacer.on_throttle();
        assert_eq!(g1, min_gap.saturating_mul(2));
        assert_eq!(g2, g1.saturating_mul(2));

        // Sustained throttling saturates at the ceiling, never beyond.
        for _ in 0..10 {
            assert!(pacer.on_throttle() <= PACE_MAX_GAP);
        }
        assert_eq!(pacer.on_throttle(), PACE_MAX_GAP);

        // Sustained success walks the gap back down to the configured rate
        // (additive increase), and stops there.
        for _ in 0..10_000 {
            pacer.on_success();
        }
        assert_eq!(pacer.on_throttle(), min_gap.saturating_mul(2));
    }

    #[test]
    fn pacer_reclaims_slot_when_reservation_dropped_uncommitted() {
        let pacer = AdaptivePacer::new(10);
        let min_gap = Duration::from_millis(100);

        // Commit one slot so the cursor sits one gap in the future.
        pacer.reserve().commit();
        // Reserve then drop without committing: models a caller cancelled during
        // the pacing wait. The tail slot must be reclaimed.
        drop(pacer.reserve());
        // The next caller reuses the reclaimed slot: one gap out, not two.
        let next = pacer.reserve();
        assert!(next.wait() <= min_gap);
    }

    #[test]
    fn pacer_steady_state_gap_tracks_configured_rate() {
        // A faster configured rate settles at a smaller gap.
        let fast = AdaptivePacer::new(100);
        for _ in 0..10_000 {
            fast.on_success();
        }
        assert_eq!(
            fast.on_throttle(),
            Duration::from_millis(10).saturating_mul(2)
        );

        // Defensive: config rejects 0, but the pacer still never divides by
        // zero, clamping to 1 RPS (a 1s gap).
        let guarded = AdaptivePacer::new(0);
        for _ in 0..10_000 {
            guarded.on_success();
        }
        assert_eq!(
            guarded.on_throttle(),
            Duration::from_secs(1).saturating_mul(2)
        );
    }
}
