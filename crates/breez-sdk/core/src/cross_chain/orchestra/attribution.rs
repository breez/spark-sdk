//! Partner attribution on Orchestra requests: affiliate ids on quotes and
//! estimates, and the order tag on quotes.

use std::sync::Mutex;

use flashnet::FlashnetError;
use tracing::{debug, info, warn};

use crate::utils::time::now_secs;

const DEFAULT_AFFILIATE_ID: &str = "breez_sdk";
/// The partner portal registers the partner's affiliate as `breez_<partner id>`.
const PARTNER_AFFILIATE_PREFIX: &str = "breez_";

/// How long a rejected partner affiliate id is left off requests before it is
/// tried again, so a partner who registers mid-session starts earning.
const PARTNER_AFFILIATE_RETRY_SECS: u64 = 60 * 60;

/// Affiliate ids and order tag derived from the partner id.
///
/// The affiliate ids are `breez_sdk`, followed by `breez_<partner id>` when
/// there is one. Orchestra rejects a whole request that names an unregistered
/// or disabled affiliate. A rejected partner affiliate id is left off for
/// [`PARTNER_AFFILIATE_RETRY_SECS`]. `breez_sdk` is always registered, so an
/// affiliate rejection of a request carrying the partner's is the partner's.
///
/// The tag is the bare partner id, sent whether or not the partner affiliate
/// id is left off.
pub(super) struct Attribution {
    partner_affiliate_id: Option<String>,
    partner_affiliate_rejected_at: Mutex<Option<u64>>,
    tag: Option<String>,
}

impl Attribution {
    pub(super) fn new(partner_id: Option<String>) -> Self {
        let partner_affiliate_id = partner_id
            .as_ref()
            .map(|id| format!("{PARTNER_AFFILIATE_PREFIX}{id}"));
        if let Some(id) = &partner_affiliate_id {
            info!("Orchestra: partner affiliate id {id}");
        } else {
            info!("Orchestra: no partner affiliate id, sending {DEFAULT_AFFILIATE_ID} only");
        }
        Self {
            partner_affiliate_id,
            partner_affiliate_rejected_at: Mutex::new(None),
            tag: partner_id,
        }
    }

    /// The tag for a quote's orders.
    pub(super) fn tag(&self) -> Option<String> {
        self.tag.clone()
    }

    fn partner_affiliate_in_use(&self, now: u64) -> Option<&str> {
        let partner = self.partner_affiliate_id.as_deref()?;
        let rejected_at = *self.partner_affiliate_rejected_at.lock().unwrap();
        match rejected_at {
            Some(at) if now.saturating_sub(at) < PARTNER_AFFILIATE_RETRY_SECS => None,
            _ => Some(partner),
        }
    }

    fn reject_partner_affiliate(&self, now: u64) {
        *self.partner_affiliate_rejected_at.lock().unwrap() = Some(now);
    }

    /// Runs `request` with the current affiliate ids. If a request carrying the
    /// partner affiliate id is rejected, runs it once more with `breez_sdk`
    /// only. The partner affiliate id is left off for
    /// [`PARTNER_AFFILIATE_RETRY_SECS`] only when the rejection is an affiliate
    /// rejection or mentions it.
    pub(super) async fn run<T, F, Fut>(&self, request: F) -> Result<T, FlashnetError>
    where
        F: Fn(Vec<String>) -> Fut,
        Fut: Future<Output = Result<T, FlashnetError>>,
    {
        self.run_at(now_secs(), request).await
    }

    async fn run_at<T, F, Fut>(&self, now: u64, request: F) -> Result<T, FlashnetError>
    where
        F: Fn(Vec<String>) -> Fut,
        Fut: Future<Output = Result<T, FlashnetError>>,
    {
        let without_partner = vec![DEFAULT_AFFILIATE_ID.to_string()];
        let Some(partner) = self.partner_affiliate_in_use(now) else {
            if let Some(partner) = &self.partner_affiliate_id {
                debug!(
                    "Orchestra: affiliate ids {without_partner:?} ({partner} left off after a \
                     rejection)"
                );
            } else {
                debug!("Orchestra: affiliate ids {without_partner:?}");
            }
            return request(without_partner).await;
        };
        let ids = vec![DEFAULT_AFFILIATE_ID.to_string(), partner.to_string()];
        debug!("Orchestra: affiliate ids {ids:?}");
        let err = match request(ids).await {
            Ok(value) => return Ok(value),
            Err(err) => err,
        };
        if !is_rejection(&err) {
            return Err(err);
        }
        if is_partner_rejection(&err, partner) {
            info!(
                "Orchestra: affiliate {partner} was rejected ({err}), leaving it off for \
                 {PARTNER_AFFILIATE_RETRY_SECS}s"
            );
            self.reject_partner_affiliate(now);
            let retried = request(without_partner).await;
            if let Err(e) = &retried {
                warn!("Orchestra: request without affiliate {partner} also failed: {e}");
            }
            return retried;
        }
        // The rejection doesn't mention the partner, so retry this request
        // without it in case it was the cause, but keep it for the next one.
        let retried = request(without_partner).await;
        match &retried {
            Ok(_) => warn!(
                "Orchestra: request with affiliate {partner} was rejected ({err}) but succeeded \
                 without it"
            ),
            Err(e) if mentions(&err, DEFAULT_AFFILIATE_ID) => {
                warn!("Orchestra: affiliate {DEFAULT_AFFILIATE_ID} was rejected: {e}");
            }
            Err(e) => debug!(
                "Orchestra: request with affiliate {partner} was rejected ({err}), and failed \
                 without it too: {e}"
            ),
        }
        retried
    }
}

/// Whether `err` is a rejection the partner id could have caused. Rate
/// limiting, rejected credentials, server errors and the typed amount and
/// route errors are not.
fn is_rejection(err: &FlashnetError) -> bool {
    match err {
        FlashnetError::InvalidRequest { .. } | FlashnetError::AffiliateRejected { .. } => true,
        FlashnetError::Network {
            code: Some(code), ..
        } => (400..500).contains(code) && !matches!(code, 401 | 403 | 429),
        _ => false,
    }
}

/// Whether `err` is the partner's rejection: an affiliate rejection, which can
/// only be the partner's, or one that mentions its id.
fn is_partner_rejection(err: &FlashnetError, partner: &str) -> bool {
    matches!(err, FlashnetError::AffiliateRejected { .. }) || mentions(err, partner)
}

/// Whether the rejection's message contains `id` as a whole affiliate id, not
/// as part of a longer one.
fn mentions(err: &FlashnetError, id: &str) -> bool {
    let (FlashnetError::InvalidRequest { reason, .. } | FlashnetError::Network { reason, .. }) =
        err
    else {
        return false;
    };
    reason
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-')))
        .any(|token| token == id)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use flashnet::FlashnetError;
    use macros::{async_test_all, test_all};

    use super::{Attribution, DEFAULT_AFFILIATE_ID, PARTNER_AFFILIATE_RETRY_SECS};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const PARTNER_ID: &str = "3f0f749007e24b";
    const PARTNER: &str = "breez_3f0f749007e24b";
    const NOW: u64 = 1_000_000;

    /// The rejection Orchestra returns for an unknown or disabled affiliate
    /// without specific affiliate errors.
    fn unknown_affiliate(id: &str) -> FlashnetError {
        invalid_request(&format!("Unknown affiliateId: {id}"))
    }

    fn invalid_request(reason: &str) -> FlashnetError {
        FlashnetError::InvalidRequest {
            reason: reason.to_string(),
            code: 400,
        }
    }

    /// The rejection Orchestra returns for an unusable affiliate with specific
    /// affiliate errors. `reason` doesn't have to name the affiliate.
    fn affiliate_rejected(reason: &str) -> FlashnetError {
        FlashnetError::AffiliateRejected {
            reason: reason.to_string(),
            code: 400,
        }
    }

    fn client_error(code: u16, reason: &str) -> FlashnetError {
        FlashnetError::Network {
            reason: reason.to_string(),
            code: Some(code),
        }
    }

    fn with_partner(partner: &str) -> Vec<String> {
        vec![DEFAULT_AFFILIATE_ID.to_string(), partner.to_string()]
    }

    fn both() -> Vec<String> {
        with_partner(PARTNER)
    }

    fn default_only() -> Vec<String> {
        vec![DEFAULT_AFFILIATE_ID.to_string()]
    }

    /// Runs `attribution` at `now` against scripted responses, one per call,
    /// and returns the result with the affiliate ids each call carried.
    async fn run(
        attribution: &Attribution,
        now: u64,
        responses: Vec<Result<(), FlashnetError>>,
    ) -> (Result<(), FlashnetError>, Vec<Vec<String>>) {
        let calls = Mutex::new(Vec::new());
        let responses = Mutex::new(responses.into_iter());
        let result = attribution
            .run_at(now, |ids| {
                calls.lock().unwrap().push(ids);
                let response = responses.lock().unwrap().next().expect("unexpected call");
                async move { response }
            })
            .await;
        (result, calls.into_inner().unwrap())
    }

    /// The ids the next request carries, one second after `now`.
    async fn next_call(attribution: &Attribution, now: u64) -> Vec<String> {
        let (_, calls) = run(attribution, now.saturating_add(1), vec![Ok(())]).await;
        calls.into_iter().next().expect("one call")
    }

    #[test_all]
    fn the_tag_is_the_partner_id() {
        assert_eq!(
            Attribution::new(Some(PARTNER_ID.to_string()))
                .tag()
                .as_deref(),
            Some(PARTNER_ID)
        );
        assert_eq!(Attribution::new(None).tag(), None);
    }

    #[async_test_all]
    async fn a_registered_partner_rides_along_with_breez_sdk() {
        let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
        let (result, calls) = run(&attribution, NOW, vec![Ok(())]).await;
        assert!(result.is_ok());
        assert_eq!(calls, vec![both()]);
    }

    #[async_test_all]
    async fn no_partner_sends_breez_sdk_only_and_never_retries() {
        let attribution = Attribution::new(None);
        let (result, calls) = run(&attribution, NOW, vec![Err(invalid_request("bad"))]).await;
        assert!(result.is_err());
        assert_eq!(calls, vec![default_only()]);
    }

    #[async_test_all]
    async fn an_affiliate_rejection_leaves_the_partner_off_until_the_window_passes() {
        // The messages Orchestra sent live for an unknown and a disabled affiliate.
        for reason in [
            format!("Unknown affiliateId: {PARTNER}"),
            "Affiliate is disabled".to_string(),
        ] {
            let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
            let (result, calls) = run(
                &attribution,
                NOW,
                vec![Err(affiliate_rejected(&reason)), Ok(())],
            )
            .await;
            assert!(result.is_ok(), "{reason}");
            assert_eq!(calls, vec![both(), default_only()], "{reason}");

            let (_, calls) = run(
                &attribution,
                NOW + PARTNER_AFFILIATE_RETRY_SECS - 1,
                vec![Ok(())],
            )
            .await;
            assert_eq!(calls, vec![default_only()], "{reason}");
            let (_, calls) = run(
                &attribution,
                NOW + PARTNER_AFFILIATE_RETRY_SECS,
                vec![Ok(())],
            )
            .await;
            assert_eq!(calls, vec![both()], "{reason}");
        }
    }

    #[async_test_all]
    async fn an_affiliate_rejection_is_remembered_when_the_retry_also_fails() {
        let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
        let (result, calls) = run(
            &attribution,
            NOW,
            vec![
                Err(affiliate_rejected("Affiliate is disabled")),
                Err(client_error(503, "down")),
            ],
        )
        .await;
        assert!(matches!(
            result,
            Err(FlashnetError::Network {
                code: Some(503),
                ..
            })
        ));
        assert_eq!(calls, vec![both(), default_only()]);
        assert_eq!(next_call(&attribution, NOW).await, default_only());
    }

    #[async_test_all]
    async fn an_affiliate_rejection_without_the_partner_is_returned_without_a_retry() {
        let attribution = Attribution::new(None);
        let (result, calls) = run(
            &attribution,
            NOW,
            vec![Err(affiliate_rejected("Affiliate is disabled"))],
        )
        .await;
        assert!(matches!(
            result,
            Err(FlashnetError::AffiliateRejected { .. })
        ));
        assert_eq!(calls, vec![default_only()]);

        let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
        let (setup, _) = run(
            &attribution,
            NOW,
            vec![Err(affiliate_rejected("Affiliate is disabled")), Ok(())],
        )
        .await;
        assert!(setup.is_ok());
        let (result, calls) = run(
            &attribution,
            NOW + 1,
            vec![Err(affiliate_rejected("Affiliate is disabled"))],
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls, vec![default_only()]);
    }

    #[async_test_all]
    async fn a_rejection_naming_the_partner_leaves_it_off_until_the_window_passes() {
        let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
        let (result, calls) = run(
            &attribution,
            NOW,
            vec![Err(unknown_affiliate(PARTNER)), Ok(())],
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(calls, vec![both(), default_only()]);

        let (_, calls) = run(
            &attribution,
            NOW + PARTNER_AFFILIATE_RETRY_SECS - 1,
            vec![Ok(())],
        )
        .await;
        assert_eq!(calls, vec![default_only()]);
        let (_, calls) = run(
            &attribution,
            NOW + PARTNER_AFFILIATE_RETRY_SECS,
            vec![Ok(())],
        )
        .await;
        assert_eq!(calls, vec![both()]);
    }

    #[async_test_all]
    async fn a_rejection_mentioning_the_partner_in_other_words_leaves_it_off() {
        let rejections = [
            invalid_request(&format!("Unknown affiliateId: {PARTNER}.")),
            invalid_request(&format!("Unknown affiliateId: '{PARTNER}'")),
            invalid_request(&format!("Affiliate \"{PARTNER}\" is disabled")),
            invalid_request(&format!(
                "Unknown affiliateIds: {DEFAULT_AFFILIATE_ID}, {PARTNER}"
            )),
            client_error(404, &format!("Affiliate {PARTNER} not found")),
            client_error(409, &format!("affiliate_disabled ({PARTNER})")),
            client_error(422, &format!("[{PARTNER}] has no fee plan")),
        ];
        for rejection in rejections {
            let label = format!("{rejection:?}");
            let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
            let (result, calls) = run(&attribution, NOW, vec![Err(rejection), Ok(())]).await;
            assert!(result.is_ok(), "{label}");
            assert_eq!(calls, vec![both(), default_only()], "{label}");
            assert_eq!(
                next_call(&attribution, NOW).await,
                default_only(),
                "{label}"
            );
        }
    }

    #[async_test_all]
    async fn a_rejection_naming_the_partner_is_remembered_when_the_retry_also_fails() {
        let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
        let (result, calls) = run(
            &attribution,
            NOW,
            vec![
                Err(unknown_affiliate(PARTNER)),
                Err(client_error(503, "down")),
            ],
        )
        .await;
        assert!(matches!(
            result,
            Err(FlashnetError::Network {
                code: Some(503),
                ..
            })
        ));
        assert_eq!(calls, vec![both(), default_only()]);
        assert_eq!(next_call(&attribution, NOW).await, default_only());
    }

    #[async_test_all]
    async fn a_longer_id_containing_the_partner_is_not_a_mention() {
        let partner = "breez_1";
        let attribution = Attribution::new(Some("1".to_string()));
        let (result, calls) = run(
            &attribution,
            NOW,
            vec![Err(unknown_affiliate("breez_1a2b")), Ok(())],
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(calls, vec![with_partner(partner), default_only()]);
        assert_eq!(next_call(&attribution, NOW).await, with_partner(partner));
    }

    #[async_test_all]
    async fn an_unattributed_rejection_is_retried_once_without_being_remembered() {
        let rejections = [
            invalid_request("recipientAddress is invalid"),
            unknown_affiliate("breez_someone_else"),
            client_error(400, "Bad request"),
            client_error(404, "Not found"),
            client_error(409, "Conflict"),
            client_error(422, "Unprocessable"),
        ];
        for rejection in rejections {
            let label = format!("{rejection:?}");
            let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
            let (result, calls) = run(&attribution, NOW, vec![Err(rejection), Ok(())]).await;
            assert!(result.is_ok(), "{label}");
            assert_eq!(calls, vec![both(), default_only()], "{label}");
            assert_eq!(next_call(&attribution, NOW).await, both(), "{label}");
        }
    }

    #[async_test_all]
    async fn an_unattributed_rejection_that_persists_returns_the_retry_error() {
        let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
        let (result, calls) = run(
            &attribution,
            NOW,
            vec![
                Err(invalid_request("recipientAddress is invalid")),
                Err(invalid_request("still invalid")),
            ],
        )
        .await;
        assert!(matches!(
            result,
            Err(FlashnetError::InvalidRequest { ref reason, .. }) if reason == "still invalid"
        ));
        assert_eq!(calls, vec![both(), default_only()]);
        assert_eq!(next_call(&attribution, NOW).await, both());
    }

    #[async_test_all]
    async fn a_rejection_naming_breez_sdk_surfaces_and_is_not_blamed_on_the_partner() {
        let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
        let (result, calls) = run(
            &attribution,
            NOW,
            vec![
                Err(unknown_affiliate(DEFAULT_AFFILIATE_ID)),
                Err(unknown_affiliate(DEFAULT_AFFILIATE_ID)),
            ],
        )
        .await;
        assert!(matches!(
            result,
            Err(FlashnetError::InvalidRequest { ref reason, .. })
                if *reason == format!("Unknown affiliateId: {DEFAULT_AFFILIATE_ID}")
        ));
        assert_eq!(calls, vec![both(), default_only()]);
        assert_eq!(next_call(&attribution, NOW).await, both());
    }

    #[async_test_all]
    async fn throttling_credentials_and_typed_errors_are_returned_without_a_retry() {
        let errors = [
            client_error(401, "no"),
            client_error(403, "no"),
            client_error(429, &format!("Too many requests for {PARTNER}")),
            client_error(500, &format!("Affiliate {PARTNER} lookup failed")),
            client_error(503, "busy"),
            FlashnetError::Network {
                reason: "transport".to_string(),
                code: None,
            },
            FlashnetError::AmountOutOfRange {
                reason: "too small".to_string(),
                too_small: true,
            },
            FlashnetError::RouteUnavailable {
                reason: "busy".to_string(),
                temporary: true,
            },
        ];
        for error in errors {
            let label = format!("{error:?}");
            let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
            let (result, calls) = run(&attribution, NOW, vec![Err(error)]).await;
            assert!(result.is_err(), "{label}");
            assert_eq!(calls, vec![both()], "{label}");
            assert_eq!(next_call(&attribution, NOW).await, both(), "{label}");
        }
    }

    #[async_test_all]
    async fn while_the_partner_is_left_off_a_rejection_is_returned_without_a_retry() {
        let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
        let (setup, _) = run(
            &attribution,
            NOW,
            vec![Err(unknown_affiliate(PARTNER)), Ok(())],
        )
        .await;
        assert!(setup.is_ok());
        let (result, calls) = run(
            &attribution,
            NOW + 1,
            vec![Err(invalid_request("recipientAddress is invalid"))],
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls, vec![default_only()]);
    }

    #[async_test_all]
    async fn a_clock_moving_back_keeps_the_partner_left_off() {
        let attribution = Attribution::new(Some(PARTNER_ID.to_string()));
        let (setup, _) = run(
            &attribution,
            NOW,
            vec![Err(unknown_affiliate(PARTNER)), Ok(())],
        )
        .await;
        assert!(setup.is_ok());
        let (_, calls) = run(&attribution, NOW - 600, vec![Ok(())]).await;
        assert_eq!(calls, vec![default_only()]);
    }
}
