//! Client side of `exact` on `lnbtc`: validating a challenge before paying it,
//! and turning the payer's result into a payment payload.

use std::str::FromStr;

use bitcoin::{
    hashes::{Hash, sha256},
    secp256k1::PublicKey,
};
use serde_json::{Value, json};

use super::{
    binding::{
        BindingParams, HTTP_PROFILE, HttpRequest, MCP_PROFILE, McpToolCall, http_request_hash,
        mcp_request_hash,
    },
    error::{X402Error, X402ErrorReason, X402Result},
    fields::{decode_lower_hex_32, is_lower_hex, parse_positive_decimal, str_field},
    invoice::decode_invoice,
    network::LnbtcNetwork,
    types::{PaymentPayload, PaymentRequired, ResourceInfo, X402_VERSION},
};

pub const DEFAULT_CLOCK_SKEW_SECS: u64 = 60;
const SCHEME_EXACT: &str = "exact";
const ASSET_BTC: &str = "BTC";
const TRANSFER_METHOD_BOLT11: &str = "bolt11";
const PAYMENT_FLOW_UPFRONT: &str = "upfront";

/// The request the client is paying for. Its binding is recomputed from this,
/// never taken from the challenge.
pub enum IntendedRequest<'a> {
    Http(HttpRequest<'a>),
    Mcp(McpToolCall<'a>),
}

/// A challenge entry that passed every pre-payment check.
#[derive(Clone, Debug)]
pub struct PreparedPayment {
    /// The accepted requirement exactly as the server sent it.
    pub accepted: Value,
    pub resource: ResourceInfo,
    pub extensions: Option<Value>,
    pub invoice: String,
    /// Lowercase hex.
    pub payment_hash: String,
    pub amount_msat: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayerStatus {
    Paid,
    Unpaid,
    InFlight,
}

/// What the payer reported after being asked to pay `PreparedPayment::invoice`.
#[derive(Clone, Debug)]
pub struct PayerResult {
    pub status: PayerStatus,
    pub invoice: String,
    pub payment_hash: String,
    /// Invoice amount, excluding routing fees.
    pub amount_msat: u64,
    pub preimage: Option<String>,
}

/// Selects the first `lnbtc` entry for `network` that passes every check the
/// client must make before paying. When none passes, the first entry's error
/// is returned.
pub fn prepare_payment(
    payment_required: &PaymentRequired,
    request: &IntendedRequest,
    network: LnbtcNetwork,
    now_secs: u64,
    skew_secs: u64,
) -> X402Result<PreparedPayment> {
    if payment_required.x402_version != X402_VERSION {
        return Err(X402Error::new(
            X402ErrorReason::InvalidX402Version,
            format!("unsupported version {}", payment_required.x402_version),
        ));
    }
    let candidates: Vec<&Value> = payment_required
        .accepts
        .iter()
        .filter(|entry| {
            entry.get("scheme").and_then(Value::as_str) == Some(SCHEME_EXACT)
                && entry.get("network").and_then(Value::as_str) == Some(network.id())
        })
        .collect();
    if candidates.is_empty() {
        let any_lnbtc = payment_required.accepts.iter().any(|entry| {
            entry
                .get("network")
                .and_then(Value::as_str)
                .is_some_and(|n| n.starts_with("lnbtc:"))
        });
        return Err(if any_lnbtc {
            X402Error::new(
                X402ErrorReason::UnsupportedNetwork,
                format!("no exact entry for {}", network.id()),
            )
        } else {
            X402Error::new(
                X402ErrorReason::UnsupportedScheme,
                "no exact lnbtc entry offered",
            )
        });
    }
    let mut first_error = None;
    for entry in candidates {
        match prepare_entry(
            payment_required,
            entry,
            request,
            network,
            now_secs,
            skew_secs,
        ) {
            Ok(prepared) => return Ok(prepared),
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    Err(first_error.expect("at least one candidate was checked"))
}

/// Checks the payer's result and builds the payload carrying the preimage.
pub fn complete_payment(
    prepared: &PreparedPayment,
    result: &PayerResult,
) -> X402Result<PaymentPayload> {
    match result.status {
        PayerStatus::Paid => {}
        PayerStatus::InFlight => {
            return Err(X402Error::new(
                X402ErrorReason::PaymentInFlight,
                "payment is still in flight",
            ));
        }
        PayerStatus::Unpaid => {
            return Err(X402Error::new(
                X402ErrorReason::PaymentNotPaid,
                "payer did not report the invoice as paid",
            ));
        }
    }
    if result.invoice != prepared.invoice {
        return Err(X402Error::new(
            X402ErrorReason::PayerInvoiceMismatch,
            "payer paid a different invoice",
        ));
    }
    if result.payment_hash != prepared.payment_hash {
        return Err(X402Error::new(
            X402ErrorReason::PayerPaymentHashMismatch,
            "payer reported a different payment hash",
        ));
    }
    if result.amount_msat != prepared.amount_msat {
        return Err(X402Error::new(
            X402ErrorReason::PayerAmountMismatch,
            format!(
                "payer paid {} msat, invoice is for {}",
                result.amount_msat, prepared.amount_msat
            ),
        ));
    }
    let preimage = result.preimage.as_deref().ok_or_else(|| {
        X402Error::new(
            X402ErrorReason::PayerPreimageRequired,
            "paid result has no preimage",
        )
    })?;
    let preimage_bytes = decode_lower_hex_32(preimage).ok_or_else(|| {
        X402Error::new(
            X402ErrorReason::PayerPreimageMalformed,
            "preimage is not 64 lowercase hex characters",
        )
    })?;
    if hex::encode(sha256::Hash::hash(&preimage_bytes).to_byte_array()) != prepared.payment_hash {
        return Err(X402Error::new(
            X402ErrorReason::PayerPreimageHashMismatch,
            "preimage does not hash to the payment hash",
        ));
    }
    Ok(PaymentPayload {
        x402_version: X402_VERSION,
        resource: Some(prepared.resource.clone()),
        accepted: prepared.accepted.clone(),
        payload: json!({ "preimage": preimage }),
        extensions: prepared.extensions.clone(),
    })
}

/// The requirement's own terms, checked before its invoice is decoded.
struct EntryTerms<'a> {
    amount_msat: u64,
    max_timeout_secs: u64,
    pay_to: PublicKey,
    extra: &'a Value,
    invoice: &'a str,
}

/// Steps 1 and 2: core fields, payment flow, transfer method and invoice presence.
fn check_entry_terms(entry: &Value) -> X402Result<EntryTerms<'_>> {
    if str_field(entry, "asset") != Some(ASSET_BTC) {
        return Err(X402Error::new(X402ErrorReason::Asset, "asset is not BTC"));
    }
    let amount_msat = str_field(entry, "amount")
        .and_then(parse_positive_decimal)
        .ok_or_else(|| {
            X402Error::new(
                X402ErrorReason::Amount,
                "amount is not a positive integer string",
            )
        })?;
    let max_timeout_secs = entry
        .get("maxTimeoutSeconds")
        .and_then(Value::as_u64)
        .filter(|t| *t > 0)
        .ok_or_else(|| {
            X402Error::new(
                X402ErrorReason::MaxTimeout,
                "maxTimeoutSeconds is not a positive integer",
            )
        })?;
    let pay_to = str_field(entry, "payTo")
        .filter(|p| is_lower_hex(p, 66))
        .and_then(|p| PublicKey::from_str(p).ok())
        .ok_or_else(|| {
            X402Error::new(
                X402ErrorReason::PayToMalformed,
                "payTo is not a compressed public key in lowercase hex",
            )
        })?;

    let extra = entry.get("extra").unwrap_or(&Value::Null);
    if str_field(extra, "paymentFlow") != Some(PAYMENT_FLOW_UPFRONT) {
        return Err(X402Error::new(
            X402ErrorReason::PaymentFlow,
            "paymentFlow is not upfront",
        ));
    }
    match extra.get("assetTransferMethod") {
        None => {}
        Some(Value::String(m)) if m == TRANSFER_METHOD_BOLT11 => {}
        Some(_) => {
            return Err(X402Error::new(
                X402ErrorReason::AssetTransferMethod,
                "assetTransferMethod is not bolt11",
            ));
        }
    }
    let invoice = str_field(extra, "invoice")
        .filter(|i| !i.is_empty())
        .ok_or_else(|| X402Error::new(X402ErrorReason::InvoiceMissing, "no invoice"))?;
    Ok(EntryTerms {
        amount_msat,
        max_timeout_secs,
        pay_to,
        extra,
        invoice,
    })
}

fn prepare_entry(
    payment_required: &PaymentRequired,
    entry: &Value,
    request: &IntendedRequest,
    network: LnbtcNetwork,
    now_secs: u64,
    skew_secs: u64,
) -> X402Result<PreparedPayment> {
    let EntryTerms {
        amount_msat,
        max_timeout_secs,
        pay_to,
        extra,
        invoice: invoice_str,
    } = check_entry_terms(entry)?;

    // Step 3: strict decode, including the signature.
    let invoice = decode_invoice(invoice_str)?;

    // Step 4: request binding.
    let expected_hash = request_binding_hash(payment_required, extra, request)?;
    let Some(description_hash) = invoice.description_hash else {
        return Err(X402Error::new(
            X402ErrorReason::InvoiceDescription,
            "invoice must have exactly one description hash and no inline description",
        ));
    };
    if description_hash != expected_hash {
        return Err(X402Error::new(
            X402ErrorReason::InvoiceRequestMismatch,
            "invoice commits to a different request",
        ));
    }

    // Steps 5 to 7: payee, currency, amount.
    if invoice.payee != pay_to {
        return Err(X402Error::new(
            X402ErrorReason::InvoicePayeeMismatch,
            "invoice is not signed by payTo",
        ));
    }
    if invoice.currency != network.currency() {
        return Err(X402Error::new(
            X402ErrorReason::InvoiceCurrencyMismatch,
            "invoice currency does not match the network",
        ));
    }
    if invoice.amount_msat != Some(amount_msat) {
        return Err(X402Error::new(
            X402ErrorReason::InvoiceAmountMismatch,
            "invoice amount does not match",
        ));
    }

    // Steps 8 and 9: expiry and timestamps.
    let expiry_secs = invoice.expiry_secs;
    if expiry_secs != max_timeout_secs {
        return Err(X402Error::new(
            X402ErrorReason::InvoiceExpiryMismatch,
            format!("invoice expiry {expiry_secs}s is not maxTimeoutSeconds {max_timeout_secs}"),
        ));
    }
    let created_secs = invoice.created_secs;
    if created_secs > now_secs.saturating_add(skew_secs) {
        return Err(X402Error::new(
            X402ErrorReason::InvoiceCreatedInFuture,
            "invoice was created in the future",
        ));
    }
    // An invoice is not paid in its final second: payment needs time to land.
    if now_secs >= created_secs.saturating_add(expiry_secs) {
        return Err(X402Error::new(
            X402ErrorReason::InvoiceExpired,
            "invoice has expired",
        ));
    }

    Ok(PreparedPayment {
        accepted: entry.clone(),
        resource: payment_required.resource.clone(),
        extensions: payment_required.extensions.clone(),
        invoice: invoice_str.to_string(),
        payment_hash: hex::encode(invoice.payment_hash),
        amount_msat,
    })
}

/// Recomputes the request hash from the intended request and checks the
/// challenge's binding fields against it.
fn request_binding_hash(
    payment_required: &PaymentRequired,
    extra: &Value,
    request: &IntendedRequest,
) -> X402Result<[u8; 32]> {
    let binding_error = |detail: &str| X402Error::new(X402ErrorReason::RequestBinding, detail);
    let mismatch = |detail: &str| X402Error::new(X402ErrorReason::RequestMismatch, detail);

    let profile = str_field(extra, "requestBindingProfile")
        .ok_or_else(|| binding_error("requestBindingProfile is missing"))?;
    let advertised_hash = str_field(extra, "requestHash")
        .filter(|h| is_lower_hex(h, 64))
        .ok_or_else(|| binding_error("requestHash is not 64 lowercase hex characters"))?;
    let params = extra
        .get("requestBindingParams")
        .ok_or_else(|| binding_error("requestBindingParams is missing"))?;
    if profile != HTTP_PROFILE && profile != MCP_PROFILE {
        return Err(binding_error("unsupported requestBindingProfile"));
    }
    let params = BindingParams::parse(profile, params)?;

    let hash = match (request, params) {
        (IntendedRequest::Http(http), BindingParams::Http { headers }) => {
            if payment_required.resource.url != http.url {
                return Err(mismatch("resource URL is not the requested URL"));
            }
            http_request_hash(http, &headers)?
        }
        (IntendedRequest::Mcp(call), BindingParams::Mcp { server, metadata }) => {
            if server != call.server {
                return Err(mismatch("binding names a different MCP server"));
            }
            mcp_request_hash(call, &metadata)?
        }
        _ => {
            return Err(mismatch(
                "binding profile does not match the request transport",
            ));
        }
    };
    if hex::encode(hash) != advertised_hash {
        return Err(mismatch("requestHash does not match the request"));
    }
    Ok(hash)
}
