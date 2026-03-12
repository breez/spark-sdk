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
    CheckUsernameAvailableResponse, InvoicePaidRequest, ListMetadataRequest, ListMetadataResponse,
    PublishZapReceiptRequest, PublishZapReceiptResponse, RecoverLnurlPayRequest,
    RecoverLnurlPayResponse, RegisterLnurlPayRequest, RegisterLnurlPayResponse,
    UnregisterLnurlPayRequest, sanitize_username,
};
use nostr::{Alphabet, Event, EventBuilder, JsonUtil, Kind, TagStandard, key::Keys};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, error, trace, warn};

use crate::{
    invoice_paid::{HandleInvoicePaidError, create_invoice, handle_invoice_paid},
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
            webhook_secret: payload.webhook_secret.clone(),
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

        let webhook = lnurl_models::WebhookInfo {
            url: format!("{}://{}/webhook/{}", state.scheme, host, pubkey),
            secret: payload.webhook_secret,
        };

        Ok(Json(RegisterLnurlPayResponse {
            lnurl,
            lightning_address: format!("{}@{}", user.name, user.domain),
            webhook,
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

                let webhook = lnurl_models::WebhookInfo {
                    url: format!("{}://{}/webhook/{}", state.scheme, host, pubkey),
                    secret: user.webhook_secret.clone(),
                };

                Ok(Json(RecoverLnurlPayResponse {
                    lnurl,
                    lightning_address: format!("{}@{}", user.name, &user.domain),
                    username: user.name,
                    description: user.description,
                    webhook,
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

        // In LNURL private mode, omit nostr fields entirely
        // Otherwise, always return server's nostrPubkey for zap receipt signing
        let (allows_nostr, nostr_pubkey) = if let Some(nostr_keys) = state.nostr_keys.as_ref()
            && !user.lnurl_private_mode_enabled
        {
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

        // Store all invoices for LUD-21 verify support (unless user opted out)
        let verify_url = if user.lnurl_private_mode_enabled {
            None
        } else {
            // Store invoice in invoices table
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

            crate::background::create_rpc_client_and_subscribe(
                state.db.clone(),
                pubkey,
                &state.connection_manager,
                &state.coordinator,
                state.signer.clone(),
                state.session_manager.clone(),
                state.service_provider.clone(),
                Arc::clone(&state.subscribed_keys),
                state.invoice_paid_trigger.clone(),
            )
            .await
            .map_err(|e| {
                error!("failed to subscribe to user for invoice monitoring: {}", e);
                lnurl_error("internal server error")
            })?;

            // Build verify URL
            Some(format!(
                "{}://{}/verify/{}",
                state.scheme, domain, payment_hash
            ))
        };

        let mut response = json!({
            "pr": res.invoice,
            "routes": Vec::<String>::new(),
        });

        if let Some(verify) = verify_url {
            response["verify"] = json!(verify);
        }

        Ok(Json(response))
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

    /// Webhook endpoint for SSP payment notifications.
    /// Verifies HMAC-SHA256 signature and processes payment preimages.
    #[allow(clippy::too_many_lines)]
    pub async fn webhook(
        Host(host): Host,
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        let domain = sanitize_domain(&state, &host)?;

        let user = state
            .db
            .get_user_by_pubkey(&domain, &pubkey)
            .await
            .map_err(|e| {
                error!("failed to look up user for webhook: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?;

        let expected_secret = user.and_then(|u| u.webhook_secret).ok_or_else(|| {
            trace!("no webhook secret for pubkey {}", pubkey);
            (
                StatusCode::NOT_FOUND,
                Json(Value::String("webhooks not configured".into())),
            )
        })?;

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

        let secret_bytes = hex::decode(&expected_secret).map_err(|_| {
            error!("failed to decode derived webhook secret");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Value::String("internal server error".into())),
            )
        })?;

        let mut engine = HmacEngine::<sha256::Hash>::new(&secret_bytes);
        engine.input(&body);
        let expected_hmac: Hmac<sha256::Hash> = Hmac::from_engine(engine);

        if expected_hmac.to_byte_array() != signature_bytes.as_slice() {
            trace!("invalid webhook signature for pubkey {}", pubkey);
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(Value::String("invalid signature".into())),
            ));
        }

        // Parse the body
        let payload: WebhookPayload = serde_json::from_slice(&body).map_err(|e| {
            trace!("invalid webhook payload: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(Value::String("invalid payload".into())),
            )
        })?;

        // Only process lightning receive finished events
        if payload.event_type != "SPARK_LIGHTNING_RECEIVE_FINISHED" {
            debug!(
                "ignoring webhook event type: {} for pubkey {}",
                payload.event_type, pubkey
            );
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

        // Verify URL pubkey matches payload's receiver pubkey
        if pubkey != receiver_pubkey {
            warn!(
                "webhook pubkey mismatch: URL={}, payload={}",
                pubkey, receiver_pubkey
            );
            return Ok(());
        }

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
        let invoice = state
            .db
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

        // Verify invoice belongs to this user
        if invoice.user_pubkey != pubkey {
            warn!(
                "webhook invoice user mismatch: expected={}, got={}",
                pubkey, invoice.user_pubkey
            );
            return Ok(());
        }

        // Handle the invoice paid event
        if let Err(e) = handle_invoice_paid(
            &state.db,
            &payment_hash,
            &payment_preimage,
            &state.invoice_paid_trigger,
        )
        .await
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
            payment_hash, pubkey
        );
        Ok(())
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

#[derive(Debug, Deserialize)]
struct WebhookPayload {
    #[serde(rename = "type")]
    event_type: String,
    payment_preimage: Option<String>,
    receiver_identity_public_key: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Extension, Router, routing::post};
    use bitcoin::hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256};
    use tower::ServiceExt;

    use crate::repository::{Invoice, LnurlRepositoryError, LnurlSenderComment, NewlyPaid};
    use crate::user::User;
    use crate::zap::Zap;

    // --- Minimal in-memory repository for testing ---

    #[derive(Clone, Default)]
    struct MockRepository {
        users: std::sync::Arc<tokio::sync::Mutex<Vec<User>>>,
        invoices: std::sync::Arc<tokio::sync::Mutex<Vec<Invoice>>>,
        newly_paid: std::sync::Arc<tokio::sync::Mutex<Vec<NewlyPaid>>>,
    }

    #[async_trait::async_trait]
    impl crate::repository::LnurlRepository for MockRepository {
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
            domain: &str,
            pubkey: &str,
        ) -> Result<Option<User>, LnurlRepositoryError> {
            let users = self.users.lock().await;
            Ok(users
                .iter()
                .find(|u| u.domain == domain || u.pubkey == pubkey)
                .cloned())
        }
        async fn upsert_user(&self, user: &User) -> Result<(), LnurlRepositoryError> {
            let mut users = self.users.lock().await;
            users.retain(|u| !(u.domain == user.domain && u.pubkey == user.pubkey));
            users.push(user.clone());
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
        ) -> Result<Vec<lnurl_models::ListMetadataMetadata>, LnurlRepositoryError> {
            Ok(vec![])
        }
        async fn list_domains(&self) -> Result<Vec<String>, LnurlRepositoryError> {
            Ok(vec![])
        }
        async fn add_domain(&self, _: &str) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }
        async fn upsert_invoice(&self, invoice: &Invoice) -> Result<(), LnurlRepositoryError> {
            let mut invoices = self.invoices.lock().await;
            invoices.retain(|i| i.payment_hash != invoice.payment_hash);
            invoices.push(invoice.clone());
            Ok(())
        }
        async fn get_invoice_by_payment_hash(
            &self,
            payment_hash: &str,
        ) -> Result<Option<Invoice>, LnurlRepositoryError> {
            let invoices = self.invoices.lock().await;
            Ok(invoices
                .iter()
                .find(|i| i.payment_hash == payment_hash)
                .cloned())
        }
        async fn get_invoice_monitored_users(&self) -> Result<Vec<String>, LnurlRepositoryError> {
            Ok(vec![])
        }
        async fn is_invoice_monitored_user(&self, _: &str) -> Result<bool, LnurlRepositoryError> {
            Ok(false)
        }
        async fn insert_newly_paid(&self, np: &NewlyPaid) -> Result<(), LnurlRepositoryError> {
            self.newly_paid.lock().await.push(np.clone());
            Ok(())
        }
        async fn get_pending_newly_paid(&self) -> Result<Vec<NewlyPaid>, LnurlRepositoryError> {
            Ok(vec![])
        }
        async fn update_newly_paid_retry(
            &self,
            _: &str,
            _: i32,
            _: i64,
        ) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }
        async fn delete_newly_paid(&self, _: &str) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }
    }

    // --- Helper to build a minimal test State ---

    async fn build_test_state(repo: MockRepository) -> State<MockRepository> {
        use spark::operator::rpc::DefaultConnectionManager;
        use spark::session_manager::InMemorySessionManager;
        use spark::ssp::ServiceProvider;
        use spark::token::InMemoryTokenOutputStore;
        use spark::tree::InMemoryTreeStore;
        use spark_wallet::{DefaultSigner, Network, SparkWalletConfig};
        use std::collections::HashSet;
        use tokio::sync::{Mutex, watch};

        let auth_seed: [u8; 32] = [1u8; 32];
        let network = Network::Regtest;
        let spark_config = SparkWalletConfig::default_config(network);
        let signer = Arc::new(DefaultSigner::new(&auth_seed, network).unwrap());
        let session_manager = Arc::new(InMemorySessionManager::default());
        let connection_manager: Arc<dyn spark::operator::rpc::ConnectionManager> =
            Arc::new(DefaultConnectionManager::new());
        let coordinator = spark_config.operator_pool.get_coordinator().clone();
        let service_provider = Arc::new(ServiceProvider::new(
            spark_config.service_provider_config.clone(),
            signer.clone(),
            session_manager.clone(),
        ));

        let wallet = Arc::new(
            spark_wallet::SparkWallet::new(
                spark_config,
                signer.clone(),
                session_manager.clone(),
                Arc::new(InMemoryTreeStore::default()),
                Arc::new(InMemoryTokenOutputStore::default()),
                Arc::clone(&connection_manager),
                None,
                true,
                None,
            )
            .await
            .unwrap(),
        );

        let (invoice_paid_trigger, _rx) = watch::channel(());

        State {
            db: repo,
            wallet,
            scheme: "http".to_string(),
            min_sendable: 1000,
            max_sendable: 1_000_000_000,
            include_spark_address: false,
            domains: HashSet::new(),
            nostr_keys: None,
            ca_cert: None,
            connection_manager,
            coordinator,
            signer,
            session_manager,
            service_provider,
            subscribed_keys: Arc::new(Mutex::new(HashSet::new())),
            invoice_paid_trigger,
        }
    }

    async fn seed_user_with_webhook_secret(repo: &MockRepository, pubkey: &str, secret: &str) {
        repo.upsert_user(&User {
            domain: String::new(),
            pubkey: pubkey.to_string(),
            name: "test".to_string(),
            description: String::new(),
            lnurl_private_mode_enabled: false,
            webhook_secret: Some(secret.to_string()),
        })
        .await
        .unwrap();
    }

    fn build_webhook_router(state: State<MockRepository>) -> Router {
        Router::new()
            .route(
                "/webhook/{pubkey}",
                post(LnurlServer::<MockRepository>::webhook),
            )
            .layer(Extension(state))
    }

    fn compute_hmac(secret_hex: &str, body: &[u8]) -> String {
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let mut engine = HmacEngine::<sha256::Hash>::new(&secret_bytes);
        engine.input(body);
        let hmac: Hmac<sha256::Hash> = Hmac::from_engine(engine);
        hex::encode(hmac.to_byte_array())
    }

    // --- Tests ---

    #[tokio::test]
    async fn test_webhook_returns_404_when_not_configured() {
        let repo = MockRepository::default();
        let state = build_test_state(repo).await;
        let app = build_webhook_router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/webhook/somepubkey")
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_webhook_rejects_missing_signature() {
        let pubkey = "somepubkey";
        let secret = "aa".repeat(32);

        let repo = MockRepository::default();
        seed_user_with_webhook_secret(&repo, pubkey, &secret).await;
        let state = build_test_state(repo).await;
        let app = build_webhook_router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/webhook/{pubkey}"))
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_webhook_rejects_invalid_signature() {
        let pubkey = "somepubkey";
        let secret = "aa".repeat(32);

        let repo = MockRepository::default();
        seed_user_with_webhook_secret(&repo, pubkey, &secret).await;
        let state = build_test_state(repo).await;
        let app = build_webhook_router(state);

        let body = r#"{"type":"SPARK_LIGHTNING_RECEIVE_FINISHED","payment_preimage":"aa","receiver_identity_public_key":"somepubkey"}"#;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/webhook/{pubkey}"))
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .header("X-Spark-Signature", "deadbeef")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_webhook_ignores_unknown_event_type() {
        let pubkey = "aabbccdd";
        let secret = "bb".repeat(32);

        let repo = MockRepository::default();
        seed_user_with_webhook_secret(&repo, pubkey, &secret).await;
        let state = build_test_state(repo).await;
        let app = build_webhook_router(state);

        let body = r#"{"type":"SPARK_LIGHTNING_SEND_FINISHED"}"#;
        let sig = compute_hmac(&secret, body.as_bytes());

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/webhook/{pubkey}"))
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .header("X-Spark-Signature", sig)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_webhook_rejects_missing_preimage() {
        let pubkey = "aabbccdd";
        let secret = "bb".repeat(32);

        let repo = MockRepository::default();
        seed_user_with_webhook_secret(&repo, pubkey, &secret).await;
        let state = build_test_state(repo).await;
        let app = build_webhook_router(state);

        let body = serde_json::json!({
            "type": "SPARK_LIGHTNING_RECEIVE_FINISHED",
            "receiver_identity_public_key": pubkey
        })
        .to_string();
        let sig = compute_hmac(&secret, body.as_bytes());

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/webhook/{pubkey}"))
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .header("X-Spark-Signature", sig)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_webhook_ignores_pubkey_mismatch() {
        let url_pubkey = "aabbccdd";
        let payload_pubkey = "11223344";
        let secret = "bb".repeat(32);

        let repo = MockRepository::default();
        seed_user_with_webhook_secret(&repo, url_pubkey, &secret).await;

        let preimage = "aa".repeat(32);
        let body = serde_json::json!({
            "type": "SPARK_LIGHTNING_RECEIVE_FINISHED",
            "payment_preimage": preimage,
            "receiver_identity_public_key": payload_pubkey
        })
        .to_string();
        let sig = compute_hmac(&secret, body.as_bytes());

        let state = build_test_state(repo).await;
        let app = build_webhook_router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/webhook/{url_pubkey}"))
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .header("X-Spark-Signature", sig)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return OK (silently ignore mismatch)
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_webhook_processes_valid_payment() {
        let pubkey = "aabbccdd";
        let secret = "bb".repeat(32);

        // Create a known preimage and compute its payment hash
        let preimage_bytes = [0x42u8; 32];
        let preimage_hex = hex::encode(preimage_bytes);
        let payment_hash = sha256::Hash::hash(&preimage_bytes).to_string();

        let repo = MockRepository::default();
        seed_user_with_webhook_secret(&repo, pubkey, &secret).await;

        // Seed the repository with an invoice matching this payment hash
        let invoice = Invoice {
            payment_hash: payment_hash.clone(),
            user_pubkey: pubkey.to_string(),
            invoice: "lnbc1test".to_string(),
            preimage: None,
            invoice_expiry: 9_999_999_999,
            created_at: 1000,
            updated_at: 1000,
        };
        repo.upsert_invoice(&invoice).await.unwrap();

        let state = build_test_state(repo.clone()).await;
        let app = build_webhook_router(state);

        let body = serde_json::json!({
            "type": "SPARK_LIGHTNING_RECEIVE_FINISHED",
            "payment_preimage": preimage_hex,
            "receiver_identity_public_key": pubkey
        })
        .to_string();
        let sig = compute_hmac(&secret, body.as_bytes());

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/webhook/{pubkey}"))
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .header("X-Spark-Signature", sig)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Verify the invoice now has the preimage stored
        let updated = repo
            .get_invoice_by_payment_hash(&payment_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.preimage.as_deref(), Some(preimage_hex.as_str()));

        // Verify a newly_paid entry was queued for background processing
        let queued = repo.newly_paid.lock().await;
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].payment_hash, payment_hash);
    }

    #[tokio::test]
    async fn test_webhook_ok_when_invoice_not_found() {
        let pubkey = "aabbccdd";
        let secret = "bb".repeat(32);

        let preimage_bytes = [0x42u8; 32];
        let preimage_hex = hex::encode(preimage_bytes);

        let repo = MockRepository::default();
        seed_user_with_webhook_secret(&repo, pubkey, &secret).await;
        let state = build_test_state(repo).await;
        let app = build_webhook_router(state);

        let body = serde_json::json!({
            "type": "SPARK_LIGHTNING_RECEIVE_FINISHED",
            "payment_preimage": preimage_hex,
            "receiver_identity_public_key": pubkey
        })
        .to_string();
        let sig = compute_hmac(&secret, body.as_bytes());

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/webhook/{pubkey}"))
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .header("X-Spark-Signature", sig)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        // No invoice found — returns OK gracefully
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_webhook_ok_when_invoice_belongs_to_different_user() {
        let pubkey = "aabbccdd";
        let secret = "bb".repeat(32);

        let preimage_bytes = [0x42u8; 32];
        let preimage_hex = hex::encode(preimage_bytes);
        let payment_hash = sha256::Hash::hash(&preimage_bytes).to_string();

        let repo = MockRepository::default();
        seed_user_with_webhook_secret(&repo, pubkey, &secret).await;
        // Invoice belongs to a different user
        let invoice = Invoice {
            payment_hash: payment_hash.clone(),
            user_pubkey: "other_user_pubkey".to_string(),
            invoice: "lnbc1test".to_string(),
            preimage: None,
            invoice_expiry: 9_999_999_999,
            created_at: 1000,
            updated_at: 1000,
        };
        repo.upsert_invoice(&invoice).await.unwrap();

        let state = build_test_state(repo.clone()).await;
        let app = build_webhook_router(state);

        let body = serde_json::json!({
            "type": "SPARK_LIGHTNING_RECEIVE_FINISHED",
            "payment_preimage": preimage_hex,
            "receiver_identity_public_key": pubkey
        })
        .to_string();
        let sig = compute_hmac(&secret, body.as_bytes());

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/webhook/{pubkey}"))
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .header("X-Spark-Signature", sig)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Mismatch ignored — returns OK
        assert_eq!(response.status(), StatusCode::OK);

        // Invoice should NOT have been updated
        let unchanged = repo
            .get_invoice_by_payment_hash(&payment_hash)
            .await
            .unwrap()
            .unwrap();
        assert!(unchanged.preimage.is_none());
    }
}
