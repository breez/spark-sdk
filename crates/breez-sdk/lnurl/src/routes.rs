use axum::{
    Extension, Json,
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
};
use axum_extra::extract::Host;
use bitcoin::{
    hashes::{Hash, sha256},
    secp256k1::{PublicKey, XOnlyPublicKey, ecdsa::Signature},
};
use lightning_invoice::Bolt11Invoice;
use lnurl_models::{
    CheckUsernameAvailableResponse, InvoicePaidRequest, ListMetadataRequest, ListMetadataResponse,
    PublishZapReceiptRequest, PublishZapReceiptResponse, RecoverLnurlPayRequest,
    RecoverLnurlPayResponse, RegisterLnurlPayRequest, RegisterLnurlPayResponse,
    UnregisterLnurlPayRequest, sanitize_username,
};
use nostr::{Alphabet, Event, JsonUtil, Kind, TagStandard, key::Keys};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, error, trace, warn};

use crate::{
    invoice_paid::handle_invoice_paid,
    repository::LnurlSenderComment,
    time::{now_millis, now_u64},
    zap::Zap,
};
use crate::{
    repository::{LnurlRepository, LnurlRepositoryError},
    state::State,
    user::{USERNAME_VALIDATION_REGEX, User},
};

const ACCEPTABLE_TIME_DIFF_SECS: u64 = 60;
const DEFAULT_METADATA_OFFSET: u32 = 0;
const DEFAULT_METADATA_LIMIT: u32 = 100;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LnurlPayCallbackParams {
    pub amount: Option<u64>,
    pub comment: Option<String>,
    pub nostr: Option<String>,
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

        let nostr_pubkey = match payload.nostr_pubkey {
            Some(nostr_pubkey) => {
                let xonly_pubkey = XOnlyPublicKey::from_str(&nostr_pubkey).map_err(|e| {
                    trace!("invalid nostr pubkey, could not parse: {:?}", e);
                    (
                        StatusCode::BAD_REQUEST,
                        Json(Value::String("invalid nostr pubkey".into())),
                    )
                })?;
                Some(xonly_pubkey.to_string())
            }
            None => None,
        };
        let user = User {
            domain: sanitize_domain(&state, &host)?,
            pubkey: pubkey.to_string(),
            name: username,
            description: payload.description,
            nostr_pubkey,
            no_invoice_paid_support: false,
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
                    nostr_pubkey: user.nostr_pubkey,
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
            .get_metadata_by_pubkey(&pubkey.to_string(), offset, limit)
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

                let builder =
                    lnurl_models::nostr::create_zap_receipt(&zap.zap_request, invoice, preimage)
                        .map_err(|e| {
                            error!("failed to recreate zap receipt: {}", e);
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(json!({"error": "internal server error"})),
                            )
                        })?;

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

        let nostr_pubkey = match (user.nostr_pubkey.as_ref(), state.nostr_keys.as_ref()) {
            (Some(nostr_pubkey), _) => {
                let xonly_pubkey = XOnlyPublicKey::from_str(nostr_pubkey).map_err(|e| {
                    error!(
                        "invalid nostr pubkey in user record, could not parse: {:?}",
                        e
                    );
                    lnurl_error("internal server error")
                })?;
                Some(xonly_pubkey)
            }
            (None, Some(nostr_keys)) => Some(nostr_keys.public_key.xonly().map_err(|e| {
                error!(
                    "invalid nostr pubkey in server keys, could not parse: {:?}",
                    e
                );
                lnurl_error("internal server error")
            })?),
            _ => None,
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
            allows_nostr: nostr_pubkey.map(|_| true),
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

        let (nostr_pubkey, is_user_nostr_key) =
            match (user.nostr_pubkey.as_ref(), state.nostr_keys.as_ref()) {
                (Some(nostr_pubkey), _) => {
                    let xonly_pubkey = XOnlyPublicKey::from_str(nostr_pubkey).map_err(|e| {
                        error!(
                            "invalid nostr pubkey in user record, could not parse: {:?}",
                            e
                        );
                        lnurl_error("internal server error")
                    })?;
                    (Some(xonly_pubkey), true)
                }
                (None, Some(nostr_keys)) => (
                    Some(nostr_keys.public_key.xonly().map_err(|e| {
                        error!(
                            "invalid nostr pubkey in server keys, could not parse: {:?}",
                            e
                        );
                        lnurl_error("internal server error")
                    })?),
                    false,
                ),
                _ => (None, false),
            };

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
                None,
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
        // save to zap event to db
        if let Some(zap_request) = params.nostr {
            let invoice_expiry: i64 = i64::try_from(expiry_timestamp.as_secs()).map_err(|e| {
                error!(
                    "invoice has invalid expiry for i64: duration since epoch {}s, expiry time: {}s: {e}",
                    invoice.duration_since_epoch().as_secs(),
                    invoice.expiry_time().as_secs(),
                );
                lnurl_error("internal server error")
            })?;

            let zap = Zap {
                payment_hash: invoice.payment_hash().to_string(),
                zap_request,
                zap_event: None,
                user_pubkey: user.pubkey.clone(),
                invoice_expiry,
                updated_at,
                is_user_nostr_key,
            };
            if let Err(e) = state.db.upsert_zap(&zap).await {
                error!("failed to save zap event: {}", e);
                return Err(lnurl_error("internal server error"));
            }

            // Subscribe to user if not already subscribed (only if nostr is enabled, and
            // the user doesn't handle the zap receipt itself)
            if !is_user_nostr_key && let Some(nostr_keys) = &state.nostr_keys {
                crate::zap::create_rpc_client_and_subscribe(
                    state.db.clone(),
                    pubkey,
                    &state.connection_manager,
                    &state.coordinator,
                    state.signer.clone(),
                    state.session_manager.clone(),
                    state.service_provider.clone(),
                    nostr_keys.clone(),
                    Arc::clone(&state.subscribed_keys),
                )
                .await
                .map_err(|e| {
                    error!("failed to subscribe to user for zaps: {}", e);
                    lnurl_error("internal server error")
                })?;
            }
        }

        if let Some(comment) = params.comment {
            let comment = comment.trim();
            if !comment.is_empty()
                && let Err(e) = state
                    .db
                    .insert_lnurl_sender_comment(&LnurlSenderComment {
                        comment: comment.to_string(),
                        payment_hash: invoice.payment_hash().to_string(),
                        user_pubkey: user.pubkey.clone(),
                        updated_at,
                    })
                    .await
            {
                error!("Failed to insert lnurl sender comment: {:?}", e);
                return Err(lnurl_error("internal server error"));
            }
        }

        // TODO: Save things like the invoice/preimage/transfer id?
        // TODO: Validate invoice?
        // TODO: Add lnurl-verify

        Ok(Json(json!({
            "pr": res.invoice,
            "routes": Vec::<String>::new(),
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

    /// Invoice-paid notification endpoint.
    /// Client notifies server that an invoice was paid with the preimage.
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
        if timestamp.abs_diff(now_u64()) > ACCEPTABLE_TIME_DIFF_SECS {
            trace!("invalid timestamp, too far off: {}", timestamp);
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
