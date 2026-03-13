use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use axum_extra::extract::Host;
use bitcoin::{
    hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256},
    secp256k1::{PublicKey, XOnlyPublicKey, ecdsa::Signature},
};
use lightning_invoice::Bolt11Invoice;
use lnurl_models::{
    CheckUsernameAvailableResponse, InvoicePaidRequest, InvoicesPaidRequest, ListMetadataRequest,
    ListMetadataResponse, PublishZapReceiptRequest, PublishZapReceiptResponse,
    RecoverLnurlPayRequest, RecoverLnurlPayResponse, RegisterLnurlPayRequest,
    RegisterLnurlPayResponse, UnregisterLnurlPayRequest, sanitize_username,
};
use nostr::{Alphabet, Event, EventBuilder, JsonUtil, Kind, TagStandard, key::Keys};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::marker::PhantomData;
use std::str::FromStr;
use tracing::{debug, error, trace, warn};

use crate::{
    invoice_paid::{
        HandleInvoicePaidError, create_invoice, handle_invoice_paid, handle_invoices_paid,
    },
    repository::LnurlSenderComment,
    time::{now_millis, now_u64},
    zap::Zap,
};
use crate::{
    repository::{LnurlRepository, LnurlRepositoryError},
    state::State,
    user::{USERNAME_VALIDATION_REGEX, User},
};

const ACCEPTABLE_TIME_DIFF_SECS: u64 = 600;
const DEFAULT_METADATA_OFFSET: u32 = 0;
const DEFAULT_METADATA_LIMIT: u32 = 100;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LnurlPayCallbackParams {
    pub amount: Option<u64>,
    pub comment: Option<String>,
    pub nostr: Option<String>,
    pub expiry: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tag {
    #[serde(rename = "payRequest")]
    Pay,
    #[serde(rename = "withdrawRequest")]
    Withdraw,
    #[serde(rename = "channelRequest")]
    Channel,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PayResponse {
    /// a second-level url which give you an invoice with a GET request
    /// and an amount
    pub callback: String,
    /// max sendable amount for a given user on a given service
    #[serde(rename = "maxSendable")]
    pub max_sendable: u64,
    /// min sendable amount for a given user on a given service,
    /// can not be less than 1 or more than `max_sendable`
    #[serde(rename = "minSendable")]
    pub min_sendable: u64,
    /// tag of the request
    pub tag: Tag,
    /// Metadata json which must be presented as raw string here,
    /// this is required to pass signature verification at a later step
    pub metadata: String,

    /// Optional, if true, the service allows comments
    /// the number is the max length of the comment
    #[serde(rename = "commentAllowed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_allowed: Option<u32>,

    /// Optional, if true, the service allows nostr zaps
    #[serde(rename = "allowsNostr")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allows_nostr: Option<bool>,

    /// Optional, if true, the nostr pubkey that will be used to sign zap events
    #[serde(rename = "nostrPubkey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nostr_pubkey: Option<XOnlyPublicKey>,
}

pub struct LnurlServer<DB> {
    db: PhantomData<DB>,
}

impl<DB> LnurlServer<DB>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    pub async fn available(
        Host(host): Host,
        Path(identifier): Path<String>,
        Extension(state): Extension<State<DB>>,
    ) -> Result<Json<CheckUsernameAvailableResponse>, (StatusCode, Json<Value>)> {
        let username = sanitize_username(&identifier);
        validate_username(&username)?;
        let user = state
            .db
            .get_user_by_name(&sanitize_domain(&state, &host)?, &username)
            .await
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?;

        Ok(Json(CheckUsernameAvailableResponse {
            available: user.is_none(),
        }))
    }

    pub async fn register(
        Host(host): Host,
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<RegisterLnurlPayRequest>,
    ) -> Result<Json<RegisterLnurlPayResponse>, (StatusCode, Json<Value>)> {
        let username = sanitize_username(&payload.username);
        validate_username(&username)?;
        let pubkey = validate(
            &pubkey,
            &payload.signature,
            &username,
            payload.timestamp,
            &state,
        )
        .await?;
        if payload.description.chars().take(256).count() > 255 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(Value::String("description too long".into())),
            ));
        }

        let user = User {
            domain: sanitize_domain(&state, &host)?,
            pubkey: pubkey.to_string(),
            name: username,
            description: payload.description,
            lnurl_private_mode_enabled: payload.lnurl_private_mode_enabled,
        };

        if let Err(e) = state.db.upsert_user(&user).await {
            if let LnurlRepositoryError::NameTaken = e {
                trace!("name already taken: {}", user.name);
                return Err((
                    StatusCode::CONFLICT,
                    Json(Value::String("name already taken".into())),
                ));
            }

            error!("failed to execute query: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Value::String("internal server error".into())),
            ));
        }

        debug!("registered user '{}' for pubkey {}", user.name, pubkey);
        let lnurl = format!("lnurlp://{}/lnurlp/{}", user.domain, user.name);
        Ok(Json(RegisterLnurlPayResponse {
            lnurl,
            lightning_address: format!("{}@{}", user.name, user.domain),
        }))
    }

    pub async fn unregister(
        Host(host): Host,
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<UnregisterLnurlPayRequest>,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        let username = sanitize_username(&payload.username);
        let pubkey = validate(
            &pubkey,
            &payload.signature,
            &username,
            payload.timestamp,
            &state,
        )
        .await?;

        state
            .db
            .delete_user(&sanitize_domain(&state, &host)?, &pubkey.to_string())
            .await
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?;
        debug!("unregistered user for pubkey {}", pubkey);
        Ok(())
    }

    pub async fn recover(
        Host(host): Host,
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<RecoverLnurlPayRequest>,
    ) -> Result<Json<RecoverLnurlPayResponse>, (StatusCode, Json<Value>)> {
        let pubkey = validate(
            &pubkey,
            &payload.signature,
            &pubkey,
            payload.timestamp,
            &state,
        )
        .await?;

        let user = state
            .db
            .get_user_by_pubkey(&sanitize_domain(&state, &host)?, &pubkey.to_string())
            .await
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?;

        match user {
            Some(user) => {
                let lnurl = format!("lnurlp://{}/lnurlp/{}", &user.domain, user.name);
                Ok(Json(RecoverLnurlPayResponse {
                    lnurl,
                    lightning_address: format!("{}@{}", user.name, &user.domain),
                    username: user.name,
                    description: user.description,
                }))
            }
            None => Err((
                StatusCode::NOT_FOUND,
                Json(Value::String("user not found".into())),
            )),
        }
    }

    pub async fn list_metadata(
        Path(pubkey): Path<String>,
        Query(params): Query<ListMetadataRequest>,
        Extension(state): Extension<State<DB>>,
    ) -> Result<Json<ListMetadataResponse>, (StatusCode, Json<Value>)> {
        let pubkey = validate(
            &pubkey,
            &params.signature,
            &pubkey,
            params.timestamp,
            &state,
        )
        .await?;
        let offset = params.offset.unwrap_or(DEFAULT_METADATA_OFFSET);
        let limit = params.limit.unwrap_or(DEFAULT_METADATA_LIMIT);
        let metadata = state
            .db
            .get_metadata_by_pubkey(&pubkey.to_string(), offset, limit, params.updated_after)
            .await
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?;
        Ok(Json(ListMetadataResponse { metadata }))
    }

    #[allow(clippy::too_many_lines)]
    pub async fn publish_zap_receipt(
        Path((pubkey, payment_hash)): Path<(String, String)>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<PublishZapReceiptRequest>,
    ) -> Result<Json<PublishZapReceiptResponse>, (StatusCode, Json<Value>)> {
        let pubkey = validate(
            &pubkey,
            &payload.signature,
            &payload.zap_receipt,
            payload.timestamp,
            &state,
        )
        .await?;

        // Parse and validate the zap receipt
        let zap_receipt = Event::from_json(&payload.zap_receipt).map_err(|e| {
            trace!("invalid zap receipt, could not parse: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid zap receipt"})),
            )
        })?;

        // Validate it's a zap receipt (kind 9735)
        if zap_receipt.kind != Kind::ZapReceipt {
            trace!(
                "event is not a zap receipt, got kind: {:?}",
                zap_receipt.kind
            );
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "event is not a zap receipt"})),
            ));
        }

        // Verify the zap receipt signature
        if zap_receipt.verify().is_err() {
            trace!("invalid zap receipt signature");
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid zap receipt signature"})),
            ));
        }

        // Extract preimage from zap receipt for LUD-21 backward compatibility
        // This allows old clients using publish_zap_receipt to still populate
        // the invoice's preimage for the verify endpoint
        let preimage_from_receipt = zap_receipt.tags.iter().find_map(|t| {
            if let Some(TagStandard::Preimage(p)) = t.as_standardized() {
                Some(p.clone())
            } else {
                None
            }
        });

        // Get the existing zap record
        let mut zap = state
            .db
            .get_zap_by_payment_hash(&payment_hash)
            .await
            .map_err(|e| {
                error!("failed to query zap: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "internal server error"})),
                )
            })?
            .ok_or_else(|| {
                trace!("zap not found for payment hash: {}", payment_hash);
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": "zap not found"})),
                )
            })?;

        // Verify the zap belongs to this user
        if zap.user_pubkey != pubkey.to_string() {
            trace!("zap does not belong to this user");
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({"error": "unauthorized"})),
            ));
        }

        // If we have a preimage, call the invoice paid handler for LUD-21 compatibility
        // This ensures the preimage is stored in the invoices table
        if let Some(preimage) = &preimage_from_receipt {
            match handle_invoice_paid(
                &state.db,
                &payment_hash,
                preimage,
                &state.invoice_paid_trigger,
            )
            .await
            {
                Err(HandleInvoicePaidError::InvalidPreimage(_)) => {
                    trace!("invalid preimage in zap receipt for {}", payment_hash);
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "invalid preimage"})),
                    ));
                }
                Err(e) => {
                    // Log but don't fail - this is for backward compatibility
                    debug!(
                        "Failed to handle invoice paid from zap receipt for {}: {}",
                        payment_hash, e
                    );
                }
                Ok(()) => {}
            }
        }

        // Check if zap receipt already exists
        let mut published = false;
        if let Some(zap_receipt) = &zap.zap_event {
            debug!(
                "Zap receipt already exists for payment hash {}",
                payment_hash
            );
            return Ok(Json(PublishZapReceiptResponse {
                published,
                zap_receipt: zap_receipt.clone(),
            }));
        }

        // Parse the zap request to get relay info
        let zap_request = Event::from_json(&zap.zap_request).map_err(|e| {
            error!("failed to parse stored zap request: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "internal server error"})),
            )
        })?;

        // Determine if we need to recreate the zap receipt with server nostr key
        let zap_receipt = match (zap.is_user_nostr_key, &state.nostr_keys) {
            (true, _) => zap_receipt,
            (false, None) => {
                warn!("server nostr keys not configured, but should publish zap receipt.");
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        json!({"error": "zap receipt should be server-published, but server does not support nostr (anymore)"}),
                    ),
                ));
            }
            (false, Some(signing_keys)) => {
                // Recreate zap receipt signed by server nostr key
                let preimage = zap_receipt.tags.iter().find_map(|t| {
                    if let Some(TagStandard::Preimage(p)) = t.as_standardized() {
                        Some(p.clone())
                    } else {
                        None
                    }
                });

                let invoice = zap_receipt
                    .tags
                    .iter()
                    .find_map(|t| {
                        if let Some(TagStandard::Bolt11(b)) = t.as_standardized() {
                            Some(b)
                        } else {
                            None
                        }
                    })
                    .ok_or_else(|| {
                        warn!("zap receipt missing bolt11 tag");
                        (
                            StatusCode::BAD_REQUEST,
                            Json(json!({"error": "zap receipt missing bolt11 tag"})),
                        )
                    })?;

                let zap_request_event = Event::from_json(&zap.zap_request).map_err(|e| {
                    error!("failed to parse zap request: {}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": "internal server error"})),
                    )
                })?;
                let builder = EventBuilder::zap_receipt(invoice, preimage, &zap_request_event);

                builder.sign_with_keys(signing_keys).map_err(|e| {
                    error!("failed to sign zap receipt: {}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": "internal server error"})),
                    )
                })?
            }
        };

        let relays = zap_request
            .tags
            .iter()
            .filter_map(|t| {
                if let Some(TagStandard::Relays(r)) = t.as_standardized() {
                    Some(r.clone())
                } else {
                    None
                }
            })
            .flatten()
            .collect::<Vec<_>>();

        if !relays.is_empty() {
            // The nostr keys are not really needed here, but we use them to create the client
            let publish_nostr_keys = match &state.nostr_keys {
                Some(keys) => keys.clone(),
                None => Keys::generate(),
            };
            let nostr_client = nostr_sdk::Client::new(publish_nostr_keys);
            for r in &relays {
                if let Err(e) = nostr_client.add_relay(r).await {
                    warn!("Failed to add relay {r}: {e}");
                }
            }

            nostr_client.connect().await;

            if let Err(e) = nostr_client.send_event(&zap_receipt).await {
                error!("Failed to publish zap receipt to relays: {e}");
            } else {
                debug!("Published zap receipt to {} relays", relays.len());
                published = true;
            }

            nostr_client.disconnect().await;
        }

        let zap_receipt_json = zap_receipt.try_as_json().map_err(|e| {
            error!("failed to serialize zap receipt: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "internal server error"})),
            )
        })?;
        zap.zap_event = Some(zap_receipt_json.clone());
        zap.updated_at = now_millis();
        state.db.upsert_zap(&zap).await.map_err(|e| {
            error!("failed to save zap receipt: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "internal server error"})),
            )
        })?;

        Ok(Json(PublishZapReceiptResponse {
            published,
            zap_receipt: zap_receipt_json,
        }))
    }

    pub async fn handle_lnurl_pay(
        Host(host): Host,
        Path(identifier): Path<String>,
        Extension(state): Extension<State<DB>>,
    ) -> Result<Json<PayResponse>, (StatusCode, Json<Value>)> {
        if identifier.is_empty() {
            return Err((StatusCode::NOT_FOUND, Json(Value::String(String::new()))));
        }

        let username = sanitize_username(&identifier);
        let user = state
            .db
            .get_user_by_name(&sanitize_domain(&state, &host)?, &username)
            .await
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                lnurl_error("internal server error")
            })?;

        let Some(user) = user else {
            return Err((StatusCode::NOT_FOUND, Json(Value::String(String::new()))));
        };

        let (allows_nostr, nostr_pubkey) = if let Some(nostr_keys) = state.nostr_keys.as_ref() {
            let xonly_pubkey = nostr_keys.public_key.xonly().map_err(|e| {
                error!(
                    "invalid nostr pubkey in server keys, could not parse: {:?}",
                    e
                );
                lnurl_error("internal server error")
            })?;
            (Some(true), Some(xonly_pubkey))
        } else {
            (None, None)
        };
        Ok(Json(PayResponse {
            callback: format!(
                "{}://{}/lnurlp/{}/invoice",
                state.scheme, &user.domain, user.name
            ),
            max_sendable: state.max_sendable,
            min_sendable: state.min_sendable,
            tag: Tag::Pay,
            metadata: get_metadata(&user.domain, &user),
            comment_allowed: Some(255),
            allows_nostr,
            nostr_pubkey,
        }))
    }

    #[allow(clippy::too_many_lines)]
    pub async fn handle_invoice(
        Host(host): Host,
        Path(identifier): Path<String>,
        Query(params): Query<LnurlPayCallbackParams>,
        Extension(state): Extension<State<DB>>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        if identifier.is_empty() {
            return Err((StatusCode::NOT_FOUND, Json(Value::String(String::new()))));
        }

        let username = sanitize_username(&identifier);
        let domain = sanitize_domain(&state, &host)?;
        let user = state
            .db
            .get_user_by_name(&domain, &username)
            .await
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                lnurl_error("internal server error")
            })?;
        let Some(user) = user else {
            return Err((StatusCode::NOT_FOUND, Json(Value::String(String::new()))));
        };

        let Some(amount_msat) = params.amount else {
            trace!("missing amount");
            return Err(lnurl_error("missing amount"));
        };

        if amount_msat % 1000 != 0 {
            trace!("not a full sat amount");
            return Err(lnurl_error("amount must be a whole sat amount"));
        }

        let nostr_pubkey = state
            .nostr_keys
            .as_ref()
            .map(|nostr_keys| {
                nostr_keys.public_key.xonly().map_err(|e| {
                    error!(
                        "invalid nostr pubkey in server keys, could not parse: {:?}",
                        e
                    );
                    lnurl_error("internal server error")
                })
            })
            .transpose()?;

        let desc_hash = if let Some(event) = &params.nostr {
            if nostr_pubkey.is_none() {
                trace!("nostr zap not supported");
                return Err(lnurl_error("nostr zap not supported"));
            }

            let event = Event::from_json(event).map_err(|e| {
                trace!("invalid nostr event, could not parse: {}", e);
                lnurl_error("invalid nostr event")
            })?;
            validate_nostr_zap_request(amount_msat, &event)?;
            sha256::Hash::hash(event.as_json().as_bytes())
        } else {
            let metadata = get_metadata(&user.domain, &user);
            sha256::Hash::hash(metadata.as_bytes())
        };

        let pubkey = parse_pubkey(&user.pubkey)?;
        let res = state
            .wallet
            .create_lightning_invoice(
                amount_msat / 1000,
                Some(spark_wallet::InvoiceDescription::DescriptionHash(
                    desc_hash.to_byte_array(),
                )),
                Some(pubkey),
                params.expiry,
                state.include_spark_address,
            )
            .await
            .map_err(|e| {
                error!("failed to create lightning invoice: {}", e);
                lnurl_error("failed to create invoice")
            })?;

        debug!("Created lightning invoice: {:?}", res);

        let invoice = Bolt11Invoice::from_str(&res.invoice).map_err(|e| {
            error!("failed to parse invoice: {}", e);
            lnurl_error("internal server error")
        })?;

        // Calculate expiry timestamp: current time + expiry duration from invoice
        let expiry_timestamp = invoice.expires_at().ok_or_else(|| {
            error!(
                "invoice has invalid expiry: duration since epoch {}s, expiry time: {}s",
                invoice.duration_since_epoch().as_secs(),
                invoice.expiry_time().as_secs()
            );
            lnurl_error("internal server error")
        })?;

        let updated_at = now_millis();
        let payment_hash = invoice.payment_hash().to_string();
        let invoice_expiry: i64 = i64::try_from(expiry_timestamp.as_secs()).map_err(|e| {
            error!(
                "invoice has invalid expiry for i64: duration since epoch {}s, expiry time: {}s: {e}",
                invoice.duration_since_epoch().as_secs(),
                invoice.expiry_time().as_secs(),
            );
            lnurl_error("internal server error")
        })?;

        // save to zap event to db
        if let Some(zap_request) = params.nostr {
            let zap = Zap {
                payment_hash: payment_hash.clone(),
                zap_request,
                zap_event: None,
                user_pubkey: user.pubkey.clone(),
                invoice_expiry,
                updated_at,
                is_user_nostr_key: false,
            };
            if let Err(e) = state.db.upsert_zap(&zap).await {
                error!("failed to save zap event: {}", e);
                return Err(lnurl_error("internal server error"));
            }
        }

        if let Some(comment) = params.comment {
            let comment = comment.trim();
            if !comment.is_empty()
                && let Err(e) = state
                    .db
                    .insert_lnurl_sender_comment(&LnurlSenderComment {
                        comment: comment.to_string(),
                        payment_hash: payment_hash.clone(),
                        user_pubkey: user.pubkey.clone(),
                        updated_at,
                    })
                    .await
            {
                error!("Failed to insert lnurl sender comment: {:?}", e);
                return Err(lnurl_error("internal server error"));
            }
        }

        // Store invoice for LUD-21 verify support (webhook provides payment updates)
        if let Err(e) = create_invoice(
            &state.db,
            &payment_hash,
            &user.pubkey,
            &res.invoice,
            invoice_expiry,
        )
        .await
        {
            error!("Failed to create invoice record: {}", e);
            return Err(lnurl_error("internal server error"));
        }

        let verify_url = format!("{}://{}/verify/{}", state.scheme, domain, payment_hash);

        Ok(Json(json!({
            "pr": res.invoice,
            "routes": Vec::<String>::new(),
            "verify": verify_url,
        })))
    }

    /// LUD-21 verify endpoint
    pub async fn verify(
        Path(payment_hash): Path<String>,
        Extension(state): Extension<State<DB>>,
    ) -> impl IntoResponse {
        let invoice = match state.db.get_invoice_by_payment_hash(&payment_hash).await {
            Ok(Some(invoice)) => invoice,
            Ok(None) => {
                return Json(json!({
                    "status": "ERROR",
                    "reason": "Not found"
                }));
            }
            Err(e) => {
                error!("Failed to get invoice by payment hash: {}", e);
                return Json(json!({
                    "status": "ERROR",
                    "reason": "Internal server error"
                }));
            }
        };

        let settled = invoice.preimage.is_some();
        Json(json!({
            "status": "OK",
            "settled": settled,
            "preimage": invoice.preimage,
            "pr": invoice.invoice
        }))
    }

    /// Invoice-paid notification endpoint (single invoice).
    /// Deprecated: use `invoices_paid` instead, which supports batch notifications.
    /// TODO: Remove this endpoint after all clients have migrated to `invoices_paid`.
    pub async fn invoice_paid(
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<InvoicePaidRequest>,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        let pubkey = validate(
            &pubkey,
            &payload.signature,
            &payload.preimage,
            payload.timestamp,
            &state,
        )
        .await?;

        let preimage_bytes = hex::decode(&payload.preimage).map_err(|e| {
            trace!("invalid preimage, could not decode: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(Value::String("invalid preimage".into())),
            )
        })?;
        let payment_hash = bitcoin::hashes::sha256::Hash::hash(&preimage_bytes);
        let payment_hash_hex = payment_hash.to_string();

        // Verify the invoice belongs to this user
        let invoice = state
            .db
            .get_invoice_by_payment_hash(&payment_hash_hex)
            .await
            .map_err(|e| {
                error!("Failed to get invoice: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?
            .ok_or_else(|| {
                trace!("invoice not found for payment hash: {}", payment_hash_hex);
                (
                    StatusCode::NOT_FOUND,
                    Json(Value::String("invoice not found".into())),
                )
            })?;

        if invoice.user_pubkey != pubkey.to_string() {
            trace!("invoice does not belong to this user");
            return Err((
                StatusCode::NOT_FOUND,
                Json(Value::String("invoice not found".into())),
            ));
        }

        // Use the central invoice paid handler
        handle_invoice_paid(
            &state.db,
            &payment_hash_hex,
            &payload.preimage,
            &state.invoice_paid_trigger,
        )
        .await
        .map_err(|e| {
            error!("Failed to handle invoice paid: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Value::String("internal server error".into())),
            )
        })?;

        debug!(
            "Invoice paid notification received for payment hash {}",
            payment_hash_hex
        );
        Ok(())
    }

    /// Batch invoices-paid notification endpoint.
    /// Client notifies server that multiple invoices were paid with their preimages.
    pub async fn invoices_paid(
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<InvoicesPaidRequest>,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        const MAX_PREIMAGES: usize = 100;

        if payload.invoices.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(Value::String("invoices must not be empty".into())),
            ));
        }

        if payload.invoices.len() > MAX_PREIMAGES {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(Value::String(format!(
                    "too many invoices, max is {MAX_PREIMAGES}"
                ))),
            ));
        }

        let pubkey = validate(
            &pubkey,
            &payload.signature,
            &pubkey,
            payload.timestamp,
            &state,
        )
        .await?;

        handle_invoices_paid(
            &state.db,
            &payload.invoices,
            &pubkey.to_string(),
            &state.invoice_paid_trigger,
        )
        .await
        .map_err(|e| match &e {
            HandleInvoicePaidError::InvalidInvoice(msg)
            | HandleInvoicePaidError::InvalidPreimage(msg) => {
                trace!("Invalid input in invoices-paid: {}", msg);
                (StatusCode::BAD_REQUEST, Json(Value::String(msg.clone())))
            }
            HandleInvoicePaidError::Repository(_) => {
                error!("Failed to handle invoices paid: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            }
        })?;

        debug!(
            "Invoices paid notification received for {} invoices",
            payload.invoices.len()
        );
        Ok(())
    }

    /// Webhook endpoint for SSP payment notifications.
    /// Verifies HMAC-SHA256 signature and processes payment preimages.
    pub async fn webhook(
        Extension(state): Extension<State<DB>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        process_webhook(
            &state.db,
            &state.webhook_secret,
            &state.invoice_paid_trigger,
            &headers,
            &body,
        )
        .await
    }
}

#[allow(clippy::too_many_lines)]
async fn process_webhook<DB>(
    db: &DB,
    webhook_secret: &str,
    invoice_paid_trigger: &tokio::sync::watch::Sender<()>,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<(), (StatusCode, Json<Value>)>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    // Verify HMAC-SHA256 signature
    let signature_header = headers
        .get("X-Spark-Signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            trace!("missing X-Spark-Signature header");
            (
                StatusCode::UNAUTHORIZED,
                Json(Value::String("missing signature".into())),
            )
        })?;

    let signature_bytes = hex::decode(signature_header).map_err(|_| {
        trace!("invalid signature hex encoding");
        (
            StatusCode::UNAUTHORIZED,
            Json(Value::String("invalid signature".into())),
        )
    })?;

    let mut engine = HmacEngine::<sha256::Hash>::new(webhook_secret.as_bytes());
    engine.input(body);
    let expected_hmac: Hmac<sha256::Hash> = Hmac::from_engine(engine);

    if expected_hmac.to_byte_array() != signature_bytes.as_slice() {
        trace!("invalid webhook signature");
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(Value::String("invalid signature".into())),
        ));
    }

    // Parse the body
    let payload: WebhookPayload = serde_json::from_slice(body).map_err(|e| {
        trace!("invalid webhook payload: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid payload".into())),
        )
    })?;

    // Only process lightning receive finished events
    if payload.event_type != "SPARK_LIGHTNING_RECEIVE_FINISHED" {
        debug!("ignoring webhook event type: {}", payload.event_type);
        return Ok(());
    }

    let payment_preimage = payload.payment_preimage.ok_or_else(|| {
        trace!("missing payment_preimage in webhook payload");
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("missing payment_preimage".into())),
        )
    })?;

    let receiver_pubkey = payload.receiver_identity_public_key.ok_or_else(|| {
        trace!("missing receiver_identity_public_key in webhook payload");
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("missing receiver_identity_public_key".into())),
        )
    })?;

    // Compute payment hash from preimage
    let preimage_bytes = hex::decode(&payment_preimage).map_err(|e| {
        trace!("invalid preimage hex: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid preimage".into())),
        )
    })?;
    let payment_hash = sha256::Hash::hash(&preimage_bytes).to_string();

    // Look up invoice
    let invoice = db
        .get_invoice_by_payment_hash(&payment_hash)
        .await
        .map_err(|e| {
            error!("failed to get invoice: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Value::String("internal server error".into())),
            )
        })?;

    let Some(invoice) = invoice else {
        debug!(
            "no invoice found for payment hash {} from webhook",
            payment_hash
        );
        return Ok(());
    };

    // Verify invoice belongs to the receiver
    if invoice.user_pubkey != receiver_pubkey {
        warn!(
            "webhook invoice user mismatch: expected={}, got={}",
            receiver_pubkey, invoice.user_pubkey
        );
        return Ok(());
    }

    // Handle the invoice paid event
    if let Err(e) =
        handle_invoice_paid(db, &payment_hash, &payment_preimage, invoice_paid_trigger).await
    {
        error!(
            "failed to handle webhook invoice paid for {}: {}",
            payment_hash, e
        );
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(Value::String("internal server error".into())),
        ));
    }

    debug!(
        "webhook processed: invoice {} paid for pubkey {}",
        payment_hash, receiver_pubkey
    );
    Ok(())
}

fn validate_nostr_zap_request(
    amount_msat: u64,
    event: &Event,
) -> Result<(), (StatusCode, Json<Value>)> {
    if event.kind != Kind::ZapRequest {
        trace!("nostr event is incorrect kind");
        return Err(lnurl_error("invalid nostr event"));
    }

    // 1. It MUST have a valid nostr signature
    if event.verify().is_err() {
        trace!("invalid nostr event, does not verify");
        return Err(lnurl_error("invalid nostr event"));
    }

    // 2. It MUST have tags
    if event.tags.is_empty() {
        trace!("invalid nostr event, missing tags");
        return Err(lnurl_error("invalid nostr event"));
    }

    // 3. It MUST have only one p tag
    if event
        .tags
        .iter()
        .filter_map(nostr::Tag::single_letter_tag)
        .filter(|t| t.is_lowercase() && t.character == Alphabet::P)
        .count()
        != 1
    {
        trace!("invalid nostr event, missing or multiple 'p' tags");
        return Err(lnurl_error("invalid nostr event"));
    }

    // 4. It MUST have 0 or 1 e tags
    if event
        .tags
        .iter()
        .filter_map(nostr::Tag::single_letter_tag)
        .filter(|t| t.is_lowercase() && t.character == Alphabet::E)
        .count()
        > 1
    {
        trace!("invalid nostr event, multiple 'e' tags");
        return Err(lnurl_error("invalid nostr event"));
    }

    // 5. There should be a relays tag with the relays to send the zap receipt to.
    if !event
        .tags
        .iter()
        .any(|t| matches!(t.as_standardized(), Some(TagStandard::Relays(_))))
    {
        trace!("invalid nostr event, missing relay tag");
        return Err(lnurl_error("invalid nostr event"));
    }

    // 6. If there is an amount tag, it MUST be equal to the amount query parameter.
    if let Some(millisats) = event.tags.iter().find_map(|t| {
        if let Some(TagStandard::Amount { millisats, .. }) = t.as_standardized() {
            Some(millisats)
        } else {
            None
        }
    }) && *millisats != amount_msat
    {
        trace!("invalid nostr event, amount does not match");
        return Err(lnurl_error("invalid nostr event"));
    }

    // 7. If there is an 'a' tag, it MUST be a valid event coordinate
    // NOTE: Assuming the tag is well-formed and contains the necessary fields, because it's standard.

    // 8. There MUST be 0 or 1 P tags. If there is one, it MUST be equal to the zap receipt's pubkey.
    // TODO: Implement this check.
    Ok(())
}

fn validate_username(username: &str) -> Result<(), (StatusCode, Json<Value>)> {
    if username.chars().take(65).count() > 64 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(Value::String("username too long".into())),
        ));
    }

    let regex = Regex::new(USERNAME_VALIDATION_REGEX).map_err(|e| {
        error!("failed to compile regex: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(Value::String("internal server error".into())),
        )
    })?;

    if !regex.is_match(username) {
        trace!("invalid username doesn't match regex");
        return Err((
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid username".into())),
        ));
    }

    Ok(())
}

async fn validate<DB>(
    pubkey: &str,
    signature: &str,
    message: &str,
    timestamp: Option<u64>,
    state: &State<DB>,
) -> Result<PublicKey, (StatusCode, Json<Value>)> {
    let pubkey = parse_pubkey(pubkey)?;
    let signature = hex::decode(signature).map_err(|e| {
        trace!("invalid signature, could not decode: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid signature".into())),
        )
    })?;
    let signature = Signature::from_der(&signature).map_err(|e| {
        trace!("invalid signature, could not parse: {:?}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid signature".into())),
        )
    })?;

    // This should be the preferred way to validate going forward. We accept both for backward
    // compatibility, but log a warning if the timestamp is missing. Remove the old way after a
    // deprecation period.
    if let Some(timestamp) = timestamp {
        let now = now_u64();
        let diff = timestamp.abs_diff(now);
        if diff > ACCEPTABLE_TIME_DIFF_SECS {
            trace!(
                "invalid timestamp, too far off: {}, now: {}, diff: {}",
                timestamp, now, diff
            );
            return Err((
                StatusCode::BAD_REQUEST,
                Json(Value::String("invalid timestamp".into())),
            ));
        }

        state
            .wallet
            .verify_message(&format!("{message}-{timestamp}"), &signature, &pubkey)
            .await
            .map_err(|e| {
                trace!("invalid signature with timestamp, could not verify: {}", e);
                (
                    StatusCode::BAD_REQUEST,
                    Json(Value::String("invalid signature".into())),
                )
            })?;

        return Ok(pubkey);
    }

    warn!("Use of endpoint without timestamp is deprecated, pubkey: {pubkey}, message: {message}");
    state
        .wallet
        .verify_message(message, &signature, &pubkey)
        .await
        .map_err(|e| {
            trace!("invalid signature, could not verify: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(Value::String("invalid signature".into())),
            )
        })?;

    Ok(pubkey)
}

fn parse_pubkey(pubkey: &str) -> Result<PublicKey, (StatusCode, Json<Value>)> {
    let pubkey = hex::decode(pubkey).map_err(|e| {
        trace!("invalid pubkey, could not decode: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid pubkey".into())),
        )
    })?;
    let pubkey = PublicKey::from_slice(&pubkey).map_err(|e| {
        trace!("invalid pubkey, could not parse: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid pubkey".into())),
        )
    })?;
    Ok(pubkey)
}

fn get_metadata(domain: &str, user: &User) -> String {
    json!(vec![
        vec!["text/plain", &user.description],
        vec!["text/identifier", &format!("{}@{}", user.name, domain)],
    ])
    .to_string()
}

fn lnurl_error(message: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(Value::Object(
            vec![
                ("status".into(), Value::String("ERROR".to_string())),
                ("reason".into(), Value::String(message.to_string())),
            ]
            .into_iter()
            .collect(),
        )),
    )
}

#[derive(Debug, Deserialize)]
struct WebhookPayload {
    #[serde(rename = "type")]
    event_type: String,
    payment_preimage: Option<String>,
    receiver_identity_public_key: Option<String>,
}

fn sanitize_domain<DB>(
    state: &State<DB>,
    domain: &str,
) -> Result<String, (StatusCode, Json<Value>)> {
    let domain = domain.trim().to_lowercase();
    // If domains list is empty allow all domains (for testing)
    if state.domains.is_empty() || state.domains.contains(&domain) {
        return Ok(domain);
    }
    warn!("domain not allowed: {}", domain);
    Err((StatusCode::NOT_FOUND, Json(Value::String(String::new()))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::{Invoice, LnurlRepositoryError, LnurlSenderComment, NewlyPaid};
    use crate::user::User;
    use crate::zap::Zap;
    use axum::body::Bytes;
    use axum::http::{HeaderMap, StatusCode};
    use bitcoin::hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256};
    use lnurl_models::ListMetadataMetadata;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tokio::sync::watch;

    // -- Mock repository -------------------------------------------------------

    #[derive(Clone, Default)]
    struct MockRepository {
        invoices: std::sync::Arc<Mutex<HashMap<String, Invoice>>>,
        newly_paid: std::sync::Arc<Mutex<HashMap<String, NewlyPaid>>>,
    }

    #[async_trait::async_trait]
    impl LnurlRepository for MockRepository {
        async fn delete_user(&self, _: &str, _: &str) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }
        async fn get_user_by_name(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<User>, LnurlRepositoryError> {
            Ok(None)
        }
        async fn get_user_by_pubkey(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<User>, LnurlRepositoryError> {
            Ok(None)
        }
        async fn upsert_user(&self, _: &User) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }
        async fn upsert_zap(&self, _: &Zap) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }
        async fn get_zap_by_payment_hash(
            &self,
            _: &str,
        ) -> Result<Option<Zap>, LnurlRepositoryError> {
            Ok(None)
        }
        async fn get_zap_monitored_users(&self) -> Result<Vec<String>, LnurlRepositoryError> {
            Ok(vec![])
        }
        async fn is_zap_monitored_user(&self, _: &str) -> Result<bool, LnurlRepositoryError> {
            Ok(false)
        }
        async fn insert_lnurl_sender_comment(
            &self,
            _: &LnurlSenderComment,
        ) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }
        async fn get_metadata_by_pubkey(
            &self,
            _: &str,
            _: u32,
            _: u32,
            _: Option<i64>,
        ) -> Result<Vec<ListMetadataMetadata>, LnurlRepositoryError> {
            Ok(vec![])
        }
        async fn list_domains(&self) -> Result<Vec<String>, LnurlRepositoryError> {
            Ok(vec![])
        }
        async fn add_domain(&self, _: &str) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }
        async fn upsert_invoice(&self, invoice: &Invoice) -> Result<(), LnurlRepositoryError> {
            self.invoices
                .lock()
                .unwrap()
                .insert(invoice.payment_hash.clone(), invoice.clone());
            Ok(())
        }
        async fn get_invoice_by_payment_hash(
            &self,
            payment_hash: &str,
        ) -> Result<Option<Invoice>, LnurlRepositoryError> {
            Ok(self.invoices.lock().unwrap().get(payment_hash).cloned())
        }
        async fn get_invoice_monitored_users(&self) -> Result<Vec<String>, LnurlRepositoryError> {
            Ok(vec![])
        }
        async fn is_invoice_monitored_user(&self, _: &str) -> Result<bool, LnurlRepositoryError> {
            Ok(false)
        }
        async fn insert_newly_paid(
            &self,
            newly_paid: &NewlyPaid,
        ) -> Result<(), LnurlRepositoryError> {
            self.newly_paid
                .lock()
                .unwrap()
                .insert(newly_paid.payment_hash.clone(), newly_paid.clone());
            Ok(())
        }
        async fn get_pending_newly_paid(&self) -> Result<Vec<NewlyPaid>, LnurlRepositoryError> {
            Ok(self.newly_paid.lock().unwrap().values().cloned().collect())
        }
        async fn update_newly_paid_retry(
            &self,
            _: &str,
            _: i32,
            _: i64,
        ) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }
        async fn delete_newly_paid(&self, payment_hash: &str) -> Result<(), LnurlRepositoryError> {
            self.newly_paid.lock().unwrap().remove(payment_hash);
            Ok(())
        }
        async fn filter_known_payment_hashes(
            &self,
            _payment_hashes: &[String],
        ) -> Result<Vec<String>, LnurlRepositoryError> {
            Ok(vec![])
        }
        async fn upsert_invoices_paid(
            &self,
            invoices: &[Invoice],
        ) -> Result<Vec<String>, LnurlRepositoryError> {
            let mut store = self.invoices.lock().unwrap();
            let mut updated = Vec::new();
            for invoice in invoices {
                store.insert(invoice.payment_hash.clone(), invoice.clone());
                updated.push(invoice.payment_hash.clone());
            }
            Ok(updated)
        }
        async fn insert_newly_paid_batch(
            &self,
            newly_paid: &[NewlyPaid],
        ) -> Result<(), LnurlRepositoryError> {
            let mut store = self.newly_paid.lock().unwrap();
            for np in newly_paid {
                store.insert(np.payment_hash.clone(), np.clone());
            }
            Ok(())
        }
    }

    // -- Test helpers ----------------------------------------------------------

    const TEST_WEBHOOK_SECRET: &str = "test_webhook_secret_0123456789abcdef";
    const TEST_PREIMAGE_HEX: &str =
        "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
    const TEST_RECEIVER_PUBKEY: &str = "02abc123";

    fn compute_payment_hash(preimage_hex: &str) -> String {
        let preimage_bytes = hex::decode(preimage_hex).unwrap();
        sha256::Hash::hash(&preimage_bytes).to_string()
    }

    fn compute_hmac(secret: &str, body: &[u8]) -> String {
        let mut engine = HmacEngine::<sha256::Hash>::new(secret.as_bytes());
        engine.input(body);
        let hmac: Hmac<sha256::Hash> = Hmac::from_engine(engine);
        hex::encode(hmac.to_byte_array())
    }

    fn make_webhook_payload(
        event_type: &str,
        preimage: Option<&str>,
        receiver_pubkey: Option<&str>,
    ) -> serde_json::Value {
        let mut payload = serde_json::json!({
            "id": "018677b5-e419-99d1-0000-a7030393c9af",
            "created_at": "2025-03-09T12:00:00Z",
            "updated_at": "2025-03-09T12:00:05Z",
            "network": "MAINNET",
            "request_status": "COMPLETED",
            "status": "TRANSFER_COMPLETED",
            "type": event_type,
            "timestamp": "2025-03-09T12:00:06Z",
            "invoice_amount": {"value": 50_000, "unit": "SATOSHI"},
            "htlc_amount": {"value": 50_000, "unit": "SATOSHI"},
        });
        if let Some(p) = preimage {
            payload["payment_preimage"] = serde_json::Value::String(p.to_string());
        }
        if let Some(r) = receiver_pubkey {
            payload["receiver_identity_public_key"] = serde_json::Value::String(r.to_string());
        }
        payload
    }

    fn signed_headers_and_body(secret: &str, payload: &serde_json::Value) -> (HeaderMap, Bytes) {
        let body = serde_json::to_vec(payload).unwrap();
        let sig = compute_hmac(secret, &body);
        let mut headers = HeaderMap::new();
        headers.insert("X-Spark-Signature", sig.parse().unwrap());
        (headers, Bytes::from(body))
    }

    fn setup_repo_with_invoice(preimage_hex: &str, receiver_pubkey: &str) -> MockRepository {
        let repo = MockRepository::default();
        let payment_hash = compute_payment_hash(preimage_hex);
        repo.invoices.lock().unwrap().insert(
            payment_hash.clone(),
            Invoice {
                payment_hash,
                user_pubkey: receiver_pubkey.to_string(),
                invoice: "lnbc1...".to_string(),
                preimage: None,
                invoice_expiry: i64::MAX,
                created_at: 0,
                updated_at: 0,
            },
        );
        repo
    }

    // -- Tests -----------------------------------------------------------------

    #[tokio::test]
    async fn webhook_valid_payment_marks_invoice_paid() {
        let repo = setup_repo_with_invoice(TEST_PREIMAGE_HEX, TEST_RECEIVER_PUBKEY);
        let (trigger, _rx) = watch::channel(());

        let payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            Some(TEST_PREIMAGE_HEX),
            Some(TEST_RECEIVER_PUBKEY),
        );
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        assert!(result.is_ok());

        let payment_hash = compute_payment_hash(TEST_PREIMAGE_HEX);
        let invoice = repo
            .invoices
            .lock()
            .unwrap()
            .get(&payment_hash)
            .cloned()
            .unwrap();
        assert_eq!(invoice.preimage.as_deref(), Some(TEST_PREIMAGE_HEX));

        assert!(repo.newly_paid.lock().unwrap().contains_key(&payment_hash));
    }

    #[tokio::test]
    async fn webhook_missing_signature_returns_unauthorized() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());
        let headers = HeaderMap::new();
        let body = Bytes::from(b"{}".to_vec());

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        let Err((status, _)) = result else {
            panic!("expected error");
        };
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_invalid_signature_returns_unauthorized() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());

        let payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            Some(TEST_PREIMAGE_HEX),
            Some(TEST_RECEIVER_PUBKEY),
        );
        let body_bytes = serde_json::to_vec(&payload).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("X-Spark-Signature", "deadbeef".repeat(8).parse().unwrap());
        let body = Bytes::from(body_bytes);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        let Err((status, _)) = result else {
            panic!("expected error");
        };
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_non_hex_signature_returns_unauthorized() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());

        let body = Bytes::from(b"{}".to_vec());
        let mut headers = HeaderMap::new();
        headers.insert("X-Spark-Signature", "not-valid-hex!".parse().unwrap());

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        let Err((status, _)) = result else {
            panic!("expected error");
        };
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_invalid_json_returns_bad_request() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());

        let body_bytes = b"not json";
        let sig = compute_hmac(TEST_WEBHOOK_SECRET, body_bytes);
        let mut headers = HeaderMap::new();
        headers.insert("X-Spark-Signature", sig.parse().unwrap());
        let body = Bytes::from(body_bytes.to_vec());

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        let Err((status, _)) = result else {
            panic!("expected error");
        };
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn webhook_non_receive_event_type_is_ignored() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());

        let payload = make_webhook_payload("SOME_OTHER_EVENT", None, None);
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn webhook_missing_preimage_returns_bad_request() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());

        let payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            None,
            Some(TEST_RECEIVER_PUBKEY),
        );
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        let Err((status, _)) = result else {
            panic!("expected error");
        };
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn webhook_missing_receiver_pubkey_returns_bad_request() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());

        let payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            Some(TEST_PREIMAGE_HEX),
            None,
        );
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        let Err((status, _)) = result else {
            panic!("expected error");
        };
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn webhook_invalid_preimage_hex_returns_bad_request() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());

        let payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            Some("not-valid-hex"),
            Some(TEST_RECEIVER_PUBKEY),
        );
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        let Err((status, _)) = result else {
            panic!("expected error");
        };
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn webhook_no_matching_invoice_succeeds_silently() {
        let repo = MockRepository::default(); // no invoices
        let (trigger, _rx) = watch::channel(());

        let payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            Some(TEST_PREIMAGE_HEX),
            Some(TEST_RECEIVER_PUBKEY),
        );
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn webhook_pubkey_mismatch_succeeds_silently() {
        let repo = setup_repo_with_invoice(TEST_PREIMAGE_HEX, "02different_pubkey");
        let (trigger, _rx) = watch::channel(());

        let payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            Some(TEST_PREIMAGE_HEX),
            Some(TEST_RECEIVER_PUBKEY), // doesn't match invoice's pubkey
        );
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        assert!(result.is_ok());

        // Invoice should NOT have been updated
        let payment_hash = compute_payment_hash(TEST_PREIMAGE_HEX);
        let invoice = repo
            .invoices
            .lock()
            .unwrap()
            .get(&payment_hash)
            .cloned()
            .unwrap();
        assert!(invoice.preimage.is_none());
    }

    #[tokio::test]
    async fn webhook_already_paid_invoice_is_idempotent() {
        let repo = MockRepository::default();
        let payment_hash = compute_payment_hash(TEST_PREIMAGE_HEX);
        repo.invoices.lock().unwrap().insert(
            payment_hash.clone(),
            Invoice {
                payment_hash: payment_hash.clone(),
                user_pubkey: TEST_RECEIVER_PUBKEY.to_string(),
                invoice: "lnbc1...".to_string(),
                preimage: Some(TEST_PREIMAGE_HEX.to_string()),
                invoice_expiry: i64::MAX,
                created_at: 0,
                updated_at: 0,
            },
        );
        let (trigger, _rx) = watch::channel(());

        let payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            Some(TEST_PREIMAGE_HEX),
            Some(TEST_RECEIVER_PUBKEY),
        );
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        assert!(result.is_ok());

        // No newly_paid entry should be created for an already-paid invoice
        assert!(repo.newly_paid.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn webhook_triggers_invoice_paid_notification() {
        let repo = setup_repo_with_invoice(TEST_PREIMAGE_HEX, TEST_RECEIVER_PUBKEY);
        let (trigger, rx) = watch::channel(());

        let payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            Some(TEST_PREIMAGE_HEX),
            Some(TEST_RECEIVER_PUBKEY),
        );
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        assert!(result.is_ok());

        // The watch channel should have been notified
        assert!(rx.has_changed().unwrap());
    }

    #[tokio::test]
    async fn webhook_signature_uses_correct_secret() {
        let repo = setup_repo_with_invoice(TEST_PREIMAGE_HEX, TEST_RECEIVER_PUBKEY);
        let (trigger, _rx) = watch::channel(());

        let payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            Some(TEST_PREIMAGE_HEX),
            Some(TEST_RECEIVER_PUBKEY),
        );
        // Sign with a different secret than the server expects
        let (headers, body) = signed_headers_and_body("wrong_secret", &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        let Err((status, _)) = result else {
            panic!("expected error");
        };
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_lightning_send_finished_is_ignored() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());

        let payload = serde_json::json!({
            "id": "018677b5-e419-99d1-0000-a7030393c9af",
            "created_at": "2025-03-09T12:00:00Z",
            "updated_at": "2025-03-09T12:00:05Z",
            "network": "MAINNET",
            "request_status": "COMPLETED",
            "status": "PREIMAGE_PROVIDED",
            "type": "SPARK_LIGHTNING_SEND_FINISHED",
            "timestamp": "2025-03-09T12:00:06Z",
            "encoded_invoice": "lnbc50u1p...",
            "fee": {"value": 100, "unit": "SATOSHI"},
            "idempotency_key": "user-defined-key-123",
            "invoice_amount": {"value": 50_000, "unit": "SATOSHI"}
        });
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn webhook_coop_exit_finished_is_ignored() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());

        let payload = serde_json::json!({
            "id": "018677b5-e419-99d1-0000-a7030393c9af",
            "created_at": "2025-03-09T12:00:00Z",
            "updated_at": "2025-03-09T12:00:05Z",
            "network": "MAINNET",
            "request_status": "COMPLETED",
            "status": "SUCCEEDED",
            "type": "SPARK_COOP_EXIT_FINISHED",
            "timestamp": "2025-03-09T12:00:06Z",
            "fee": {"value": 500, "unit": "SATOSHI"},
            "withdrawal_address": "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
            "l1_broadcast_fee": {"value": 200, "unit": "SATOSHI"},
            "exit_speed": "NORMAL",
            "coop_exit_txid": "a1b2c3d4...",
            "expires_at": "2025-03-10T12:00:00Z",
            "total_amount": {"value": 49_300, "unit": "SATOSHI"}
        });
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn webhook_static_deposit_finished_is_ignored() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());

        let payload = serde_json::json!({
            "id": "018677b5-e419-99d1-0000-a7030393c9af",
            "created_at": "2025-03-09T12:00:00Z",
            "updated_at": "2025-03-09T12:00:05Z",
            "network": "MAINNET",
            "request_status": "COMPLETED",
            "status": "TRANSFER_COMPLETED",
            "type": "SPARK_STATIC_DEPOSIT_FINISHED",
            "timestamp": "2025-03-09T12:00:06Z",
            "deposit_amount": {"value": 100_000, "unit": "SATOSHI"},
            "credit_amount": {"value": 99_500, "unit": "SATOSHI"},
            "max_fee": {"value": 1000, "unit": "SATOSHI"},
            "transaction_id": "d4e5f6a7b8c9...",
            "output_index": 0,
            "bitcoin_network": "MAINNET",
            "static_deposit_address": "bc1q..."
        });
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(&repo, TEST_WEBHOOK_SECRET, &trigger, &headers, &body).await;
        assert!(result.is_ok());
    }
}
