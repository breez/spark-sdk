mod models;
mod pay_locks;
mod quote_key;
mod server;
mod store;

use breez_sdk_common::x402::{
    binding::{
        HttpRequest, McpToolCall, normalize_bound_header_names, normalize_bound_metadata_names,
        parse_tool_call_params,
    },
    client::{
        DEFAULT_CLOCK_SKEW_SECS, IntendedRequest, PayerResult, PayerStatus, PreparedPayment,
        complete_payment, prepare_paid_payments, prepare_payments,
    },
    error::{X402Error, X402ErrorReason, X402Result},
    network::LnbtcNetwork,
    types::{PaymentRequired, Transport},
};
use platform_utils::{
    time::{Duration, Instant},
    tokio,
};
use spark_wallet::TransferId;

use crate::{
    BreezSdk, Payment, PaymentDetails, PaymentRequest, PaymentStatus, PaymentType,
    PrepareSendPaymentRequest, SdkError, SendPaymentMethod, SendPaymentOptions, SendPaymentRequest,
    X402ErrorKind, utils::payments::fetch_and_process_payment,
};

pub use models::*;
pub(crate) use pay_locks::X402PayLocks;
pub(crate) use quote_key::X402QuoteKey;
#[cfg(any(test, feature = "test-utils"))]
pub use store::x402_replay_store_tests;
pub use store::{X402ReplayStore, X402StoreError};

const DEFAULT_COMPLETION_TIMEOUT_SECS: u32 = 30;
const PENDING_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// The challenge costs more than the caller's `max_amount`. Not an x402
/// specification reason: the limit is the caller's own.
const MAX_AMOUNT_EXCEEDED: &str = "max_amount_exceeded";

/// Pays for, and charges for, HTTP resources and MCP tool calls with the x402
/// protocol over Lightning (the `exact` scheme on `lnbtc`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct X402 {
    sdk: BreezSdk,
}

impl X402 {
    pub(crate) fn new(sdk: BreezSdk) -> Self {
        Self { sdk }
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl X402 {
    /// Pays an x402 challenge and returns the payment to send the request again with.
    ///
    /// Before paying, the challenge is checked against the request: its invoice
    /// must commit to this exact request, be signed by the advertised payee and
    /// cost no more than `max_amount`. A challenge that fails a check is
    /// refused and nothing is paid. Paying the same challenge again returns the
    /// existing payment instead of paying twice, waiting for it to complete if
    /// it is still pending. That works after the invoice expires too, but the
    /// server accepts the payment only until shortly past its expiry (60
    /// seconds by default).
    pub async fn pay(&self, request: PayX402Request) -> Result<PayX402Response, SdkError> {
        let payment_required = PaymentRequired::from_header_or_json(&request.payment_required)?;
        let network = lnbtc_network(&self.sdk)?;
        let timeout_secs = request
            .completion_timeout_secs
            .unwrap_or(DEFAULT_COMPLETION_TIMEOUT_SECS);
        let started = Instant::now();
        let paid = with_intended_request(&request.request, |intended| {
            prepare_paid_payments(&payment_required, intended, network)
        })?;
        let _locks = self
            .sdk
            .x402_pay_locks
            .lock(paid.iter().map(|p| p.payment_hash.clone()).collect())
            .await;
        let mut existing = None;
        let mut failed = Vec::new();
        let mut unpaid = Vec::new();
        for candidate in paid {
            match self.existing_payment(&candidate.invoice).await? {
                Some(payment) if payment.status == PaymentStatus::Failed => {
                    failed.push((candidate, payment));
                }
                Some(payment) => {
                    existing = Some((candidate, payment));
                    break;
                }
                None => unpaid.push(candidate.invoice),
            }
        }
        let (prepared, payment) = if let Some(existing) = existing {
            existing
        } else {
            self.ensure_not_sending(&unpaid).await?;
            let failed_invoices: Vec<&str> =
                failed.iter().map(|(p, _)| p.invoice.as_str()).collect();
            match payable(&payment_required, &request, network, &failed_invoices) {
                Ok(prepared) => {
                    let payment = self.send(&prepared, &request, timeout_secs).await?;
                    (prepared, payment)
                }
                // The failed payment says more than why nothing else is payable.
                Err(e) => failed.into_iter().next().ok_or(e)?,
            }
        };
        let payment = self
            .wait_for_proof(payment, started, Duration::from_secs(timeout_secs.into()))
            .await?;
        respond(&prepared, payment, transport(&request.request))
    }
}

impl X402 {
    async fn existing_payment(&self, invoice: &str) -> Result<Option<Payment>, SdkError> {
        let payment = self
            .sdk
            .storage
            .get_payment_by_invoice(invoice.to_string())
            .await?;
        Ok(payment.filter(|p| p.payment_type == PaymentType::Send))
    }

    /// Refuses while an earlier send for one of `invoices` is still being
    /// finished: it failed after the operators committed it, so it has no
    /// payment record to find yet.
    async fn ensure_not_sending(&self, invoices: &[String]) -> Result<(), SdkError> {
        for invoice in invoices {
            if self.sdk.lightning_sender.has_pending_send(invoice).await {
                return Err(SdkError::Generic(
                    "x402 payment for this challenge is still being sent: pay it again later"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Polls storage until the payment has its proof or fails, or `timeout`
    /// has passed since `started`.
    async fn wait_for_proof(
        &self,
        mut payment: Payment,
        started: Instant,
        timeout: Duration,
    ) -> Result<Payment, SdkError> {
        while awaiting_proof(&payment) {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break;
            }
            tokio::time::sleep(PENDING_POLL_INTERVAL.min(remaining)).await;
            payment = self.refreshed(&payment.id).await?;
        }
        Ok(payment)
    }

    /// The stored payment, with the status and preimage the operators report
    /// when storage lacks the proof and background sync won't bring it:
    /// background tasks are off, or the send completed without its preimage.
    async fn refreshed(&self, payment_id: &str) -> Result<Payment, SdkError> {
        let mut payment = self
            .sdk
            .storage
            .get_payment_by_id(payment_id.to_string())
            .await?;
        let polled =
            self.sdk.config.background_tasks_enabled && payment.status == PaymentStatus::Pending;
        if !awaiting_proof(&payment) || polled {
            return Ok(payment);
        }
        let Ok(Some(reported)) = fetch_and_process_payment(
            &self.sdk.spark_wallet,
            self.sdk.storage.clone(),
            payment_id,
            false,
        )
        .await
        else {
            return Ok(payment);
        };
        if let (
            Some(PaymentDetails::Lightning { htlc_details, .. }),
            Some(PaymentDetails::Lightning {
                htlc_details: reported_htlc,
                ..
            }),
        ) = (&mut payment.details, &reported.details)
            && reported_htlc.payment_hash == htlc_details.payment_hash
        {
            payment.status = reported.status;
            if htlc_details.preimage.is_none() {
                htlc_details.preimage.clone_from(&reported_htlc.preimage);
            }
        }
        Ok(payment)
    }

    async fn send(
        &self,
        prepared: &PreparedPayment,
        request: &PayX402Request,
        completion_timeout_secs: u32,
    ) -> Result<Payment, SdkError> {
        let prepare_response = self
            .sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: PaymentRequest::Input {
                    input: prepared.invoice.clone(),
                },
                amount: None,
                token_identifier: None,
                conversion_options: request.conversion_options.clone(),
                fee_policy: None,
            })
            .await?;
        let SendPaymentMethod::Bolt11Invoice {
            lightning_fee_sats, ..
        } = &prepare_response.payment_method
        else {
            return Err(SdkError::Generic(
                "x402 invoice did not prepare as a Lightning payment".into(),
            ));
        };
        let X402PaymentAmount::Lightning {
            amount_sats: max_sats,
        } = request.max_amount;
        let total_sats = prepare_response
            .amount
            .saturating_add(u128::from(*lightning_fee_sats));
        if total_sats > u128::from(max_sats) {
            return Err(SdkError::x402(
                X402ErrorKind::Refused,
                MAX_AMOUNT_EXCEEDED,
                format!("the payment costs {total_sats} sats, more than the maximum of {max_sats}"),
            ));
        }
        // A token leg (an explicit or stable balance conversion) rejects
        // idempotency keys, so only a plain sats payment gets one. Transfer ids
        // are unique across Spark, so the key also names the sender.
        let identity = self.sdk.spark_wallet.get_identity_public_key();
        let idempotency_key = prepare_response.conversion_estimate.is_none().then(|| {
            TransferId::from_name(&format!("x402:{identity}:{}", prepared.payment_hash)).to_string()
        });
        let response = self
            .sdk
            .send_payment(SendPaymentRequest {
                prepare_response,
                // The preimage is the proof of payment, and a Spark-routed
                // payment does not reveal one.
                options: Some(SendPaymentOptions::Bolt11Invoice {
                    prefer_spark: false,
                    completion_timeout_secs: Some(completion_timeout_secs),
                }),
                idempotency_key,
            })
            .await?;
        Ok(response.payment)
    }
}

/// The first entry that passes every check and whose invoice amount fits
/// `max_amount`. The routing fee is checked once it is known.
/// The first entry, other than those for `excluded` invoices, that passes every
/// check and costs no more than the caller's maximum.
fn payable(
    payment_required: &PaymentRequired,
    request: &PayX402Request,
    network: LnbtcNetwork,
    excluded: &[&str],
) -> Result<PreparedPayment, SdkError> {
    let now = u64::from(breez_sdk_common::utils::now());
    let payable = with_intended_request(&request.request, |intended| {
        prepare_payments(
            payment_required,
            intended,
            network,
            now,
            DEFAULT_CLOCK_SKEW_SECS,
        )
    })?;
    let X402PaymentAmount::Lightning {
        amount_sats: max_sats,
    } = request.max_amount;
    payable
        .into_iter()
        .filter(|prepared| !excluded.contains(&prepared.invoice.as_str()))
        .find(|prepared| prepared.amount_msat.div_ceil(1000) <= max_sats)
        .ok_or_else(|| {
            SdkError::x402(
                X402ErrorKind::Refused,
                MAX_AMOUNT_EXCEEDED,
                format!("every payable offer costs more than the maximum of {max_sats} sats"),
            )
        })
}

/// The bound names as the binding takes them. A name that can't be bound is
/// the server's own mistake, not the request's.
fn normalized_bound_names(
    request: &X402Request,
    names: &[String],
) -> Result<Vec<String>, SdkError> {
    match request {
        X402Request::Http { .. } => normalize_bound_header_names(names),
        X402Request::Mcp { .. } => normalize_bound_metadata_names(names),
    }
    .map_err(|e| SdkError::InvalidInput(format!("x402 bound names: {}", e.detail)))
}

/// Runs `f` on the protocol's view of `request`. MCP params are parsed
/// strictly, so a call the request hash can't represent is refused.
fn with_intended_request<T>(
    request: &X402Request,
    f: impl FnOnce(&IntendedRequest) -> X402Result<T>,
) -> X402Result<T> {
    match request {
        X402Request::Http {
            method,
            url,
            headers,
            body,
        } => {
            let headers: Vec<(String, String)> = headers
                .iter()
                .map(|h| (h.name.clone(), h.value.clone()))
                .collect();
            f(&IntendedRequest::Http(HttpRequest {
                method,
                url,
                body: body.as_deref().unwrap_or_default(),
                headers: &headers,
            }))
        }
        X402Request::Mcp { server, params } => {
            let params = parse_tool_call_params(params)?;
            f(&IntendedRequest::Mcp(McpToolCall {
                server,
                params: &params,
            }))
        }
    }
}

/// How x402 messages travel with `request`.
fn transport(request: &X402Request) -> Transport {
    match request {
        X402Request::Http { .. } => Transport::Http,
        X402Request::Mcp { .. } => Transport::Mcp,
    }
}

/// The SDK's network in the x402 `lnbtc` namespace.
fn lnbtc_network(sdk: &BreezSdk) -> Result<LnbtcNetwork, SdkError> {
    LnbtcNetwork::from_bitcoin_network(sdk.config.network.into()).ok_or_else(|| {
        SdkError::x402(
            X402ErrorKind::Refused,
            X402ErrorReason::UnsupportedNetwork.as_str(),
            "x402 is not supported on this network".to_string(),
        )
    })
}

/// A failure in the SDK's own x402 data rather than in what it received.
fn internal(X402Error { reason, detail }: X402Error) -> SdkError {
    SdkError::Generic(format!("x402: {}: {detail}", reason.as_str()))
}

/// Whether `payment` may still produce its proof: it is in flight, or it
/// completed before its preimage was recorded.
fn awaiting_proof(payment: &Payment) -> bool {
    match payment.status {
        PaymentStatus::Pending => true,
        PaymentStatus::Completed => !matches!(
            &payment.details,
            Some(PaymentDetails::Lightning { htlc_details, .. }) if htlc_details.preimage.is_some()
        ),
        PaymentStatus::Failed => false,
    }
}

fn respond(
    prepared: &PreparedPayment,
    payment: Payment,
    transport: Transport,
) -> Result<PayX402Response, SdkError> {
    let Some(PaymentDetails::Lightning {
        invoice,
        htlc_details,
        ..
    }) = &payment.details
    else {
        return Err(SdkError::Generic(
            "x402 payment has no Lightning details".into(),
        ));
    };
    if awaiting_proof(&payment) {
        return Ok(PayX402Response {
            status: X402PaymentStatus::Pending,
            payment_signature: None,
            payment,
        });
    }
    let status = if payment.status == PaymentStatus::Completed {
        PayerStatus::Paid
    } else {
        PayerStatus::Unpaid
    };
    let result = PayerResult {
        status,
        invoice: invoice.clone(),
        payment_hash: htlc_details.payment_hash.clone(),
        // A paid invoice settles exactly its own amount; `Payment::amount` is
        // rounded to sats and would misreport sub-sat invoices.
        amount_msat: prepared.amount_msat,
        preimage: htlc_details.preimage.clone(),
    };
    let payload = complete_payment(prepared, &result)?;
    Ok(PayX402Response {
        status: X402PaymentStatus::Paid,
        payment_signature: Some(payload.encode(transport).map_err(internal)?),
        payment,
    })
}

#[cfg(test)]
mod tests {
    use base64::{Engine, prelude::BASE64_STANDARD};
    use breez_sdk_common::x402::types::{PaymentPayload, ResourceInfo};
    use serde_json::{Map, json};

    use super::*;
    use crate::{PaymentMethod, SparkHtlcDetails, SparkHtlcStatus};

    // The x402 spec's lnbtc example: a 25 sat invoice and the preimage it pays out.
    const INVOICE: &str = "lnbc250n1pj48ugqpp54y3u9s8ylemsv8l3ewyzzu0klhujvuvmkl6llchq23vy8rzjsf0qsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygshp5p4nz8am4uqj4q8a87z3sk4x6yk4dv2mvel34epw68qqkwy0xcqvqxqzfvcqpjr4rx6ls6j5rpwknuea64evlk7yfx56wmqcer5eerekdsn9tlv6v4ex9mlz5dtm9qapl3svwlqcf7837dmjkru9z9w4h2rvm0md52w2sqxrwu5f";
    const PAYMENT_HASH: &str = "a923c2c0e4fe77061ff1cb882171f6fdf926719bb7f5ffe2e05458438c52825e";
    const PREIMAGE: &str = "0001020304050607080900010203040506070809000102030405060708090102";

    fn prepared() -> PreparedPayment {
        PreparedPayment {
            accepted: json!({ "scheme": "exact", "extra": { "invoice": INVOICE } }),
            resource: ResourceInfo {
                url: "https://api.example.com/article/A".to_string(),
                other: Map::new(),
            },
            extensions: None,
            invoice: INVOICE.to_string(),
            payment_hash: PAYMENT_HASH.to_string(),
            amount_msat: 25_000,
        }
    }

    fn payment(status: PaymentStatus, preimage: Option<&str>) -> Payment {
        Payment {
            id: "payment".to_string(),
            payment_type: PaymentType::Send,
            status,
            amount: 25,
            fees: 1,
            timestamp: 1_700_000_000,
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                description: None,
                invoice: INVOICE.to_string(),
                destination_pubkey: String::new(),
                htlc_details: SparkHtlcDetails {
                    payment_hash: PAYMENT_HASH.to_string(),
                    preimage: preimage.map(str::to_string),
                    expiry_time: 0,
                    status: SparkHtlcStatus::PreimageShared,
                },
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
                lnurl_receive_metadata: None,
                conversion_info: None,
            }),
            conversion_details: None,
        }
    }

    #[macros::test_all]
    fn completed_payment_yields_signature_with_preimage() {
        let response = respond(
            &prepared(),
            payment(PaymentStatus::Completed, Some(PREIMAGE)),
            Transport::Http,
        )
        .unwrap();
        assert_eq!(response.status, X402PaymentStatus::Paid);
        let header = response.payment_signature.unwrap();
        let payload: PaymentPayload =
            serde_json::from_slice(&BASE64_STANDARD.decode(header).unwrap()).unwrap();
        assert_eq!(payload.x402_version, 2);
        assert_eq!(payload.payload, json!({ "preimage": PREIMAGE }));
        assert_eq!(payload.accepted, prepared().accepted);
    }

    #[macros::test_all]
    fn mcp_payment_signature_is_json() {
        let response = respond(
            &prepared(),
            payment(PaymentStatus::Completed, Some(PREIMAGE)),
            Transport::Mcp,
        )
        .unwrap();
        let payload: PaymentPayload =
            serde_json::from_str(&response.payment_signature.unwrap()).unwrap();
        assert_eq!(payload.payload, json!({ "preimage": PREIMAGE }));
    }

    #[macros::test_all]
    fn pending_payment_has_no_signature() {
        let response = respond(
            &prepared(),
            payment(PaymentStatus::Pending, None),
            Transport::Http,
        )
        .unwrap();
        assert_eq!(response.status, X402PaymentStatus::Pending);
        assert!(response.payment_signature.is_none());
    }

    #[macros::test_all]
    fn completed_payment_without_preimage_is_pending() {
        let response = respond(
            &prepared(),
            payment(PaymentStatus::Completed, None),
            Transport::Http,
        )
        .unwrap();
        assert_eq!(response.status, X402PaymentStatus::Pending);
        assert!(response.payment_signature.is_none());
    }

    #[macros::test_all]
    fn failed_payment_is_a_failed_x402_payment() {
        let err = respond(
            &prepared(),
            payment(PaymentStatus::Failed, None),
            Transport::Http,
        )
        .unwrap_err();
        assert!(
            matches!(
                &err,
                SdkError::X402Error { kind: X402ErrorKind::PaymentFailed, reason, .. }
                    if reason == "exact_lnbtc_payment_not_paid"
            ),
            "{err:?}"
        );
    }

    #[macros::test_all]
    fn wrong_preimage_is_refused() {
        let err = respond(
            &prepared(),
            payment(PaymentStatus::Completed, Some(&"00".repeat(32))),
            Transport::Http,
        )
        .unwrap_err();
        assert!(
            matches!(&err, SdkError::Generic(m) if m.contains("payer_preimage_hash_mismatch")),
            "{err:?}"
        );
    }
}
