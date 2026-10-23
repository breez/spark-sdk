//! Server side of `exact` on `lnbtc`: binding a request from the server's own
//! configuration, building the payment requirement, and validating a payment
//! as the facilitator's `/settle` does.

use std::str::FromStr;

use bitcoin::{
    hashes::{Hash, sha256},
    secp256k1::PublicKey,
};
use serde_json::{Map, Value, json};

use super::{
    binding::{BindingParams, HTTP_PROFILE, MCP_PROFILE, http_request_hash, mcp_request_hash},
    client::IntendedRequest,
    error::{X402Error, X402ErrorReason, X402Result},
    fields::{is_lower_hex, parse_positive_decimal, str_field},
    invoice::decode_invoice,
    jcs,
    network::LnbtcNetwork,
    types::{PaymentPayload, SettlementResponse},
};

/// How long a consumed payment hash stays recorded past the end of its
/// invoice's settlement window.
pub const REPLAY_RETENTION_SECS: u64 = 3600;
const BOLT11: &str = "bolt11";
const UPFRONT: &str = "upfront";
/// `extra` fields checked on their own, so not compared as plain server-declared fields.
const SPECIAL_EXTRA_FIELDS: [&str; 6] = [
    "invoice",
    "requestHash",
    "requestBindingProfile",
    "requestBindingParams",
    "assetTransferMethod",
    "paymentFlow",
];

/// A request's binding under the server's configured bound names: header
/// names for HTTP, `_meta` member names for MCP.
#[derive(Clone, Debug)]
pub struct RequestBinding {
    pub profile: &'static str,
    pub params: Value,
    pub hash: [u8; 32],
}

/// Binds `request` with the server's configuration. The MCP server URI is the
/// request's own, which the server sets from its configuration.
pub fn bind_request(
    request: &IntendedRequest,
    bound_names: &[String],
) -> X402Result<RequestBinding> {
    match request {
        IntendedRequest::Http(http) => Ok(RequestBinding {
            profile: HTTP_PROFILE,
            params: json!({ "headers": bound_names }),
            hash: http_request_hash(http, bound_names)?,
        }),
        IntendedRequest::Mcp(call) => Ok(RequestBinding {
            profile: MCP_PROFILE,
            params: json!({ "server": call.server, "metadata": bound_names }),
            hash: mcp_request_hash(call, bound_names)?,
        }),
    }
}

/// The fields of a strictly decoded invoice a server needs.
#[derive(Clone, Debug)]
pub struct InvoiceTerms {
    /// Lowercase hex.
    pub payment_hash: String,
    /// The invoice's signer.
    pub payee: PublicKey,
    pub amount_msat: Option<u64>,
    pub created_secs: u64,
    pub expiry_secs: u64,
}

/// Strictly decodes `invoice`, including its signature.
pub fn invoice_terms(invoice: &str) -> X402Result<InvoiceTerms> {
    let decoded = decode_invoice(invoice)?;
    Ok(InvoiceTerms {
        payment_hash: hex::encode(decoded.payment_hash),
        payee: decoded.payee,
        amount_msat: decoded.amount_msat,
        created_secs: decoded.created_secs,
        expiry_secs: decoded.expiry_secs,
    })
}

/// The terms of an `exact` `lnbtc` payment requirement.
#[derive(Clone, Debug)]
pub struct RequirementTerms<'a> {
    pub network: LnbtcNetwork,
    pub amount_msat: u64,
    pub pay_to: &'a PublicKey,
    pub max_timeout_secs: u64,
    pub binding: &'a RequestBinding,
    pub invoice: &'a str,
    /// The server's quote for these terms, carried in `extra.quote`.
    pub quote: Option<&'a str>,
}

/// Builds the `accepts` entry for `terms`.
pub fn payment_requirement(terms: &RequirementTerms) -> Value {
    let mut requirement = json!({
        "scheme": "exact",
        "network": terms.network.id(),
        "amount": terms.amount_msat.to_string(),
        "asset": "BTC",
        "payTo": terms.pay_to.to_string(),
        "maxTimeoutSeconds": terms.max_timeout_secs,
        "extra": {
            "assetTransferMethod": BOLT11,
            "paymentFlow": UPFRONT,
            "requestHash": hex::encode(terms.binding.hash),
            "requestBindingProfile": terms.binding.profile,
            "requestBindingParams": terms.binding.params,
            "invoice": terms.invoice,
        },
    });
    if let Some(quote) = terms.quote {
        requirement["extra"]["quote"] = Value::String(quote.to_string());
    }
    requirement
}

/// A payment that passed every facilitator check. It still has to be claimed
/// under [`VerifiedPayment::consumption_key`] before the resource is served.
#[derive(Clone, Debug)]
pub struct VerifiedPayment {
    pub network: LnbtcNetwork,
    /// Lowercase hex.
    pub payment_hash: String,
    pub amount_msat: u64,
    /// The invoice's creation time plus its expiry.
    pub invoice_end_secs: u64,
}

impl VerifiedPayment {
    /// The canonical replay key, `network:payment_hash`.
    pub fn consumption_key(&self) -> String {
        format!("{}:{}", self.network.id(), self.payment_hash)
    }

    /// Until when the consumed key must stay recorded.
    pub fn retain_until_secs(&self, skew_secs: u64) -> u64 {
        self.invoice_end_secs
            .saturating_add(skew_secs)
            .saturating_add(REPLAY_RETENTION_SECS)
    }

    pub fn settlement_response(&self) -> SettlementResponse {
        SettlementResponse {
            success: true,
            error_reason: None,
            transaction: self.payment_hash.clone(),
            network: self.network.id().to_string(),
            payer: None,
        }
    }
}

/// A failed settlement's response.
pub fn failure_response(reason: X402ErrorReason, network: &str) -> SettlementResponse {
    SettlementResponse {
        success: false,
        error_reason: Some(reason.as_str().to_string()),
        transaction: String::new(),
        network: network.to_string(),
        payer: None,
    }
}

/// Validates `payload` against the server's `requirements` as the
/// facilitator's `/settle` does, in the specification's step order. The
/// requirements must carry the request hash recomputed from the request
/// being served, never the client's echo. Replay is the caller's to enforce.
pub fn verify_settlement(
    requirements: &Value,
    payload: &PaymentPayload,
    now_secs: u64,
    skew_secs: u64,
) -> X402Result<VerifiedPayment> {
    let accepted = &payload.accepted;
    check_core_fields_match(requirements, accepted)?;
    let terms = check_core_fields(requirements)?;
    let empty = Value::Object(Map::new());
    let required_extra = requirements.get("extra").unwrap_or(&empty);
    let accepted_extra = accepted.get("extra").unwrap_or(&empty);
    let request_hash = check_extra(required_extra, accepted_extra)?;

    // Step 4: both invoices present. The accepted one is the one paid.
    let (Some(_), Some(accepted_invoice)) =
        (invoice_field(required_extra), invoice_field(accepted_extra))
    else {
        return Err(X402Error::new(
            X402ErrorReason::InvoiceMissing,
            "an invoice is missing",
        ));
    };

    // Step 5: the accepted invoice.
    let invoice = decode_invoice(accepted_invoice)?;
    let Some(description_hash) = invoice.description_hash else {
        return Err(X402Error::new(
            X402ErrorReason::InvoiceDescription,
            "invoice must have exactly one description hash and no inline description",
        ));
    };
    let checks = [
        (
            description_hash == request_hash,
            X402ErrorReason::InvoiceRequestMismatch,
            "invoice commits to a different request",
        ),
        (
            invoice.payee == terms.pay_to,
            X402ErrorReason::InvoicePayeeMismatch,
            "invoice is not signed by payTo",
        ),
        (
            invoice.currency == terms.network.currency(),
            X402ErrorReason::InvoiceCurrencyMismatch,
            "invoice currency does not match the network",
        ),
        (
            invoice.amount_msat == Some(terms.amount_msat),
            X402ErrorReason::InvoiceAmountMismatch,
            "invoice amount does not match",
        ),
        (
            invoice.expiry_secs == terms.max_timeout_secs,
            X402ErrorReason::InvoiceExpiryMismatch,
            "invoice expiry is not maxTimeoutSeconds",
        ),
        (
            invoice.created_secs <= now_secs.saturating_add(skew_secs),
            X402ErrorReason::InvoiceCreatedInFuture,
            "invoice was created in the future",
        ),
    ];
    if let Some((_, reason, detail)) = checks.into_iter().find(|(ok, _, _)| !ok) {
        return Err(X402Error::new(reason, detail));
    }

    // Step 6: the preimage.
    let preimage = payload
        .payload
        .get("preimage")
        .and_then(Value::as_str)
        .ok_or_else(|| X402Error::new(X402ErrorReason::PreimageMissing, "no preimage"))?;
    if !preimage
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(X402Error::new(
            X402ErrorReason::PreimageMalformed,
            "preimage is not lowercase hex",
        ));
    }
    if preimage.len() != 64 {
        return Err(X402Error::new(
            X402ErrorReason::PreimageLength,
            "preimage is not 32 bytes",
        ));
    }
    let preimage_bytes = hex::decode(preimage)
        .map_err(|e| X402Error::new(X402ErrorReason::PreimageMalformed, e.to_string()))?;
    if sha256::Hash::hash(&preimage_bytes).to_byte_array() != invoice.payment_hash {
        return Err(X402Error::new(
            X402ErrorReason::PreimageHashMismatch,
            "preimage does not hash to the payment hash",
        ));
    }

    // Step 7: a payment completed shortly before expiry may still be presented
    // until the end of the invoice plus the clock skew.
    let invoice_end_secs = invoice.created_secs.saturating_add(invoice.expiry_secs);
    if now_secs > invoice_end_secs.saturating_add(skew_secs) {
        return Err(X402Error::new(
            X402ErrorReason::InvoiceExpired,
            "invoice settlement window has passed",
        ));
    }

    Ok(VerifiedPayment {
        network: terms.network,
        payment_hash: hex::encode(invoice.payment_hash),
        amount_msat: terms.amount_msat,
        invoice_end_secs,
    })
}

struct CoreTerms {
    network: LnbtcNetwork,
    amount_msat: u64,
    pay_to: PublicKey,
    max_timeout_secs: u64,
}

/// Step 1: the accepted terms are the required ones.
fn check_core_fields_match(requirements: &Value, accepted: &Value) -> X402Result<()> {
    let fields = [
        ("scheme", X402ErrorReason::UnsupportedScheme),
        ("network", X402ErrorReason::NetworkMismatch),
        ("amount", X402ErrorReason::AmountMismatch),
        ("asset", X402ErrorReason::Asset),
        ("payTo", X402ErrorReason::PayToMismatch),
        ("maxTimeoutSeconds", X402ErrorReason::MaxTimeoutMismatch),
    ];
    for (field, reason) in fields {
        if accepted.get(field) != requirements.get(field) {
            return Err(X402Error::new(reason, format!("accepted {field} differs")));
        }
    }
    Ok(())
}

/// Step 2: the terms themselves are valid.
fn check_core_fields(requirements: &Value) -> X402Result<CoreTerms> {
    if str_field(requirements, "scheme") != Some("exact") {
        return Err(X402Error::new(
            X402ErrorReason::UnsupportedScheme,
            "scheme is not exact",
        ));
    }
    let network = str_field(requirements, "network")
        .and_then(LnbtcNetwork::from_id)
        .ok_or_else(|| X402Error::new(X402ErrorReason::UnsupportedNetwork, "unknown network"))?;
    if str_field(requirements, "asset") != Some("BTC") {
        return Err(X402Error::new(X402ErrorReason::Asset, "asset is not BTC"));
    }
    let amount_msat = str_field(requirements, "amount")
        .and_then(parse_positive_decimal)
        .ok_or_else(|| {
            X402Error::new(
                X402ErrorReason::Amount,
                "amount is not a positive integer string",
            )
        })?;
    let max_timeout_secs = requirements
        .get("maxTimeoutSeconds")
        .and_then(Value::as_u64)
        .filter(|t| *t > 0)
        .ok_or_else(|| {
            X402Error::new(
                X402ErrorReason::MaxTimeout,
                "maxTimeoutSeconds is not a positive integer",
            )
        })?;
    let pay_to = str_field(requirements, "payTo")
        .filter(|p| is_lower_hex(p, 66))
        .and_then(|p| PublicKey::from_str(p).ok())
        .ok_or_else(|| {
            X402Error::new(
                X402ErrorReason::PayToMalformed,
                "payTo is not a compressed public key in lowercase hex",
            )
        })?;
    Ok(CoreTerms {
        network,
        amount_msat,
        pay_to,
        max_timeout_secs,
    })
}

/// Step 3: transfer method, flow, request binding and server-declared
/// `extra` fields. Returns the required request hash.
fn check_extra(required: &Value, accepted: &Value) -> X402Result<[u8; 32]> {
    if transfer_method(required) != Some(BOLT11) || transfer_method(accepted) != Some(BOLT11) {
        return Err(X402Error::new(
            X402ErrorReason::AssetTransferMethod,
            "assetTransferMethod is not bolt11",
        ));
    }
    if str_field(required, "paymentFlow") != Some(UPFRONT)
        || str_field(accepted, "paymentFlow") != Some(UPFRONT)
    {
        return Err(X402Error::new(
            X402ErrorReason::PaymentFlow,
            "paymentFlow is not upfront",
        ));
    }

    let required_binding = parse_binding(required)?;
    let accepted_binding = parse_binding(accepted)?;
    if required_binding != accepted_binding {
        return Err(X402Error::new(
            X402ErrorReason::RequestMismatch,
            "accepted request binding differs",
        ));
    }

    if let Value::Object(fields) = required {
        for (name, value) in fields {
            if !SPECIAL_EXTRA_FIELDS.contains(&name.as_str()) && accepted.get(name) != Some(value) {
                return Err(X402Error::new(
                    X402ErrorReason::ExtraMismatch,
                    format!("accepted extra.{name} differs"),
                ));
            }
        }
    }
    Ok(required_binding.0)
}

fn invoice_field(extra: &Value) -> Option<&str> {
    str_field(extra, "invoice").filter(|i| !i.is_empty())
}

/// A missing `assetTransferMethod` means the scheme default, `bolt11`.
fn transfer_method(extra: &Value) -> Option<&str> {
    match extra.get("assetTransferMethod") {
        None => Some(BOLT11),
        Some(v) => v.as_str(),
    }
}

/// A side's request hash, profile and canonical parameters, validated.
fn parse_binding(extra: &Value) -> X402Result<([u8; 32], String, String)> {
    let binding_error = |detail: &str| X402Error::new(X402ErrorReason::RequestBinding, detail);
    let hash = str_field(extra, "requestHash")
        .filter(|h| is_lower_hex(h, 64))
        .and_then(|h| hex::decode(h).ok())
        .and_then(|h| <[u8; 32]>::try_from(h).ok())
        .ok_or_else(|| binding_error("requestHash is not 64 lowercase hex characters"))?;
    let profile = str_field(extra, "requestBindingProfile")
        .ok_or_else(|| binding_error("requestBindingProfile is missing"))?;
    let params = extra
        .get("requestBindingParams")
        .ok_or_else(|| binding_error("requestBindingParams is missing"))?;
    BindingParams::parse(profile, params)?;
    let canonical = jcs::canonicalize(params).map_err(|e| binding_error(&e.0))?;
    Ok((hash, profile.to_string(), canonical))
}
