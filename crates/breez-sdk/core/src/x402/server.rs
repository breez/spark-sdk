use std::{str::FromStr, sync::Arc};

use bitcoin::hashes::sha256;
use breez_sdk_common::x402::{
    client::{DEFAULT_CLOCK_SKEW_SECS, prepare_payment},
    error::{X402Error, X402ErrorReason},
    network::LnbtcNetwork,
    quote::{QuoteTerms, quote_mac, verify_quote},
    server::{
        RequestBinding, RequirementTerms, VerifiedPayment, bind_request, failure_response,
        invoice_terms, payment_requirement, verify_settlement,
    },
    types::{PaymentPayload, PaymentRequired, ResourceInfo, X402_VERSION},
};
use platform_utils::{time::Duration, tokio};
use serde_json::{Map, Value, json};
use spark_wallet::InvoiceDescription;

use super::{lnbtc_network, transport, with_intended_request};
use crate::{
    CreateX402ChallengeRequest, CreateX402ChallengeResponse, SdkError, SettleX402Request,
    SettleX402Response, X402, X402PaymentAmount, X402ReplayStore, X402Request,
};

const DEFAULT_MAX_TIMEOUT_SECS: u32 = 300;
const RECEIPT_ATTEMPTS: u32 = 10;
const RECEIPT_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Why a settlement didn't succeed: a refused payment, reported to the payer,
/// or a failure of the SDK itself.
enum SettleFailure {
    Refused(X402Error),
    Sdk(SdkError),
}

impl From<SdkError> for SettleFailure {
    fn from(e: SdkError) -> Self {
        Self::Sdk(e)
    }
}

impl From<X402Error> for SettleFailure {
    fn from(e: X402Error) -> Self {
        Self::Refused(e)
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl X402 {
    /// Creates the x402 challenge for a request that hasn't been paid for.
    ///
    /// The challenge's invoice commits to the request, so its payment can only
    /// be used for this request. Send the challenge in a 402 response, then
    /// call [`X402::settle`] when the request comes back with a payment.
    pub async fn create_challenge(
        &self,
        request: CreateX402ChallengeRequest,
    ) -> Result<CreateX402ChallengeResponse, SdkError> {
        // Refused up front: a payment taken now could never be settled.
        self.replay_store()?;
        let network = lnbtc_network(&self.sdk)?;
        let max_timeout_secs = request.max_timeout_secs.unwrap_or(DEFAULT_MAX_TIMEOUT_SECS);
        let amount_sats = lightning_price(&request.amounts)?;
        let amount_msat = amount_sats.saturating_mul(1000);
        let binding = with_intended_request(&request.request, |intended| {
            bind_request(intended, &request.bound_names)
        })?;

        // The invoice carries no Spark address: a payer routing over Spark
        // would get no preimage to prove payment with.
        let receive = self
            .sdk
            .spark_wallet
            .create_lightning_invoice(
                amount_sats,
                Some(InvoiceDescription::DescriptionHash(binding.hash)),
                None,
                Some(max_timeout_secs),
                false,
            )
            .await?;
        let invoice = receive.invoice;
        let terms = invoice_terms(&invoice)?;
        let pay_to = terms.payee.to_string();

        // Every Spark user's invoices are signed by the same service provider
        // key, so the quote is what shows these terms and this invoice are ours.
        let key = self.sdk.x402_quote_key.get().await?;
        let quote = quote_mac(
            &key,
            &QuoteTerms {
                network,
                amount_msat,
                pay_to: &pay_to,
                max_timeout_secs: u64::from(max_timeout_secs),
                request_hash: binding.hash,
                payment_hash: &terms.payment_hash,
            },
        )?;
        let requirement = payment_requirement(&RequirementTerms {
            network,
            amount_msat,
            pay_to: &terms.payee,
            max_timeout_secs: u64::from(max_timeout_secs),
            binding: &binding,
            invoice: &invoice,
            quote: Some(&quote),
        });
        let payment_required = PaymentRequired {
            x402_version: X402_VERSION,
            error: Some("Payment required".to_string()),
            resource: resource_info(&request),
            accepts: vec![requirement],
            extensions: None,
        };

        // The invoice comes from the service provider, so check the challenge
        // as a payer would before offering it: a wrong expiry, for one, is
        // otherwise only logged.
        let now = u64::from(breez_sdk_common::utils::now());
        with_intended_request(&request.request, |intended| {
            prepare_payment(
                &payment_required,
                intended,
                network,
                now,
                DEFAULT_CLOCK_SKEW_SECS,
            )
        })
        .map_err(|e| SdkError::Generic(format!("issued x402 challenge is invalid: {e}")))?;

        Ok(CreateX402ChallengeResponse {
            payment_required: payment_required.encode(transport(&request.request))?,
            payment_hash: terms.payment_hash,
        })
    }

    /// Settles the payment sent with a paid-for request.
    ///
    /// The payment must be for a challenge this SDK created for this same
    /// request, carry its invoice's preimage, and be paid to this SDK. It is
    /// then recorded as used, so it can't be used again. Serve the request
    /// only when the response reports success, and send `payment_response`
    /// with it either way.
    pub async fn settle(&self, request: SettleX402Request) -> Result<SettleX402Response, SdkError> {
        let network = lnbtc_network(&self.sdk)?;
        let transport = transport(&request.request);
        match self.try_settle(&request, network).await {
            Ok(verified) => {
                let response = verified.settlement_response();
                Ok(SettleX402Response {
                    success: true,
                    error_reason: None,
                    payment_response: response.encode(transport)?,
                    payment_hash: Some(verified.payment_hash),
                    // Challenges are priced in whole sats.
                    amount: Some(X402PaymentAmount::Lightning {
                        amount_sats: verified.amount_msat / 1000,
                    }),
                })
            }
            Err(SettleFailure::Refused(error)) => {
                let response = failure_response(error.reason, network.id());
                Ok(SettleX402Response {
                    success: false,
                    error_reason: Some(error.reason.as_str().to_string()),
                    payment_response: response.encode(transport)?,
                    payment_hash: accepted_payment_hash(&request.payment),
                    amount: None,
                })
            }
            Err(SettleFailure::Sdk(e)) => Err(e),
        }
    }
}

impl X402 {
    fn replay_store(&self) -> Result<&Arc<dyn X402ReplayStore>, SdkError> {
        self.sdk.x402_replay_store.as_ref().ok_or_else(|| {
            SdkError::Generic(
                "x402 charging needs a replay store: use a storage backend that provides one, \
                 or set one with SdkBuilder::with_x402_replay_store"
                    .to_string(),
            )
        })
    }

    async fn try_settle(
        &self,
        request: &SettleX402Request,
        network: LnbtcNetwork,
    ) -> Result<VerifiedPayment, SettleFailure> {
        let store = self.replay_store()?;
        let payload = PaymentPayload::from_header_or_json(&request.payment)?;
        let binding = with_intended_request(&request.request, |intended| {
            bind_request(intended, &request.bound_names)
        })
        .map_err(|e| X402Error::new(X402ErrorReason::RequestBinding, e.to_string()))?;

        // The terms come from the client's echo, so they are only trusted
        // once the quote over them verifies.
        let requirements = requirements_from_accepted(&payload.accepted, network, &binding);
        let now = u64::from(breez_sdk_common::utils::now());
        let verified = verify_settlement(&requirements, &payload, now, DEFAULT_CLOCK_SKEW_SECS)?;
        let key = self.sdk.x402_quote_key.get().await?;
        verify_quote(
            &key,
            &QuoteTerms {
                network,
                amount_msat: verified.amount_msat,
                pay_to: requirements["payTo"].as_str().unwrap_or_default(),
                max_timeout_secs: requirements["maxTimeoutSeconds"]
                    .as_u64()
                    .unwrap_or_default(),
                request_hash: binding.hash,
                payment_hash: &verified.payment_hash,
            },
            requirements["extra"]["quote"].as_str(),
        )?;

        // A valid preimage for an invoice we quoted means the operators
        // released it with our transfer; confirming it with them still
        // guards against a preimage that leaked another way.
        let payment_hash = sha256::Hash::from_str(&verified.payment_hash)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        let received = self
            .wait_until_received(&payment_hash)
            .await
            .map_err(|e| X402Error::new(X402ErrorReason::ReceiverUnavailable, e.to_string()))?;
        if !received {
            return Err(X402Error::new(
                X402ErrorReason::InvoiceNotSettled,
                "invoice is not settled to this SDK",
            )
            .into());
        }

        let claimed = store
            .consume(
                verified.consumption_key(),
                verified.retain_until_secs(DEFAULT_CLOCK_SKEW_SECS),
            )
            .await
            .map_err(SdkError::from)?;
        if !claimed {
            return Err(X402Error::new(
                X402ErrorReason::DuplicateSettlement,
                "payment was already used",
            )
            .into());
        }
        Ok(verified)
    }

    /// Whether the operators show the payment as received, asking a few times:
    /// a payer holding the preimage has been paid already, but the record can
    /// lag. Answering a paid request with a new challenge would make the payer
    /// pay again.
    async fn wait_until_received(&self, payment_hash: &sha256::Hash) -> Result<bool, SdkError> {
        for attempt in 1..=RECEIPT_ATTEMPTS {
            if self
                .sdk
                .spark_wallet
                .is_lightning_receive_settled(payment_hash)
                .await?
            {
                return Ok(true);
            }
            if attempt < RECEIPT_ATTEMPTS {
                tokio::time::sleep(RECEIPT_RETRY_DELAY).await;
            }
        }
        Ok(false)
    }
}

/// The Lightning price among `amounts`, which is the only rail charged on.
fn lightning_price(amounts: &[X402PaymentAmount]) -> Result<u64, SdkError> {
    match amounts {
        [] => Err(SdkError::InvalidInput(
            "an x402 challenge needs an amount".to_string(),
        )),
        [X402PaymentAmount::Lightning { amount_sats: 0 }] => Err(SdkError::InvalidInput(
            "an x402 challenge can't be free".to_string(),
        )),
        [X402PaymentAmount::Lightning { amount_sats }] => Ok(*amount_sats),
        _ => Err(SdkError::InvalidInput(
            "an x402 challenge takes one amount per rail".to_string(),
        )),
    }
}

/// The requirements to settle against: the accepted amount, `payTo`, timeout,
/// invoice and quote, under this server's network and the binding of the
/// request being served. Anything else the client changed fails the checks.
fn requirements_from_accepted(
    accepted: &Value,
    network: LnbtcNetwork,
    binding: &RequestBinding,
) -> Value {
    let accepted_extra = &accepted["extra"];
    let mut extra = json!({
        "assetTransferMethod": "bolt11",
        "paymentFlow": "upfront",
        "requestHash": hex::encode(binding.hash),
        "requestBindingProfile": binding.profile,
        "requestBindingParams": binding.params,
    });
    for field in ["invoice", "quote"] {
        if let Some(value) = accepted_extra.get(field) {
            extra[field] = value.clone();
        }
    }
    let mut requirements = json!({
        "scheme": "exact",
        "network": network.id(),
        "asset": "BTC",
        "extra": extra,
    });
    for field in ["amount", "payTo", "maxTimeoutSeconds"] {
        if let Some(value) = accepted.get(field) {
            requirements[field] = value.clone();
        }
    }
    requirements
}

/// The payment hash of the accepted invoice, when the payment is decodable.
fn accepted_payment_hash(payment: &str) -> Option<String> {
    let payload = PaymentPayload::from_header_or_json(payment).ok()?;
    let invoice = payload.accepted["extra"]["invoice"].as_str()?;
    Some(invoice_terms(invoice).ok()?.payment_hash)
}

/// The challenge's resource: the URL requested over HTTP, or the tool for MCP.
/// The MCP params were already parsed strictly when the request was bound.
fn resource_info(request: &CreateX402ChallengeRequest) -> ResourceInfo {
    let url = match &request.request {
        X402Request::Http { url, .. } => url.clone(),
        X402Request::Mcp { params, .. } => {
            let params: Value = serde_json::from_str(params).unwrap_or_default();
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            format!("mcp://tool/{name}")
        }
    };
    let mut other = Map::new();
    if let Some(description) = &request.description {
        other.insert(
            "description".to_string(),
            Value::String(description.clone()),
        );
    }
    if let Some(mime_type) = &request.mime_type {
        other.insert("mimeType".to_string(), Value::String(mime_type.clone()));
    }
    ResourceInfo { url, other }
}

#[cfg(test)]
mod tests {
    use breez_sdk_common::x402::binding::{HTTP_PROFILE, MCP_PROFILE};

    use super::*;

    fn binding() -> RequestBinding {
        RequestBinding {
            profile: HTTP_PROFILE,
            params: json!({ "headers": [] }),
            hash: [3; 32],
        }
    }

    #[macros::test_all]
    fn price_is_one_positive_lightning_amount() {
        let sats = |amount_sats| X402PaymentAmount::Lightning { amount_sats };
        assert_eq!(lightning_price(&[sats(21)]).unwrap(), 21);
        assert!(lightning_price(&[]).is_err());
        assert!(lightning_price(&[sats(0)]).is_err());
        assert!(lightning_price(&[sats(21), sats(42)]).is_err());
    }

    #[macros::test_all]
    fn requirements_take_terms_but_not_binding_or_network_from_the_echo() {
        let accepted = json!({
            "scheme": "other",
            "network": "lnbtc:000000000019d6689c085ae165831e93",
            "amount": "21000",
            "asset": "USD",
            "payTo": "02aa",
            "maxTimeoutSeconds": 300,
            "extra": {
                "invoice": "lnbcrt1",
                "quote": "mac",
                "requestHash": "ff",
                "requestBindingProfile": MCP_PROFILE,
                "clientNote": "ignored",
            },
        });
        let requirements = requirements_from_accepted(&accepted, LnbtcNetwork::Regtest, &binding());
        assert_eq!(requirements["scheme"], "exact");
        assert_eq!(requirements["network"], LnbtcNetwork::Regtest.id());
        assert_eq!(requirements["asset"], "BTC");
        assert_eq!(requirements["amount"], "21000");
        assert_eq!(requirements["payTo"], "02aa");
        assert_eq!(requirements["maxTimeoutSeconds"], 300);
        assert_eq!(requirements["extra"]["invoice"], "lnbcrt1");
        assert_eq!(requirements["extra"]["quote"], "mac");
        assert_eq!(requirements["extra"]["requestHash"], hex::encode([3; 32]));
        assert_eq!(requirements["extra"]["requestBindingProfile"], HTTP_PROFILE);
        assert!(requirements["extra"].get("clientNote").is_none());
    }
}
