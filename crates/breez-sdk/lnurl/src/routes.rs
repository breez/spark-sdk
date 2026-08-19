use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, Query},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use axum_extra::extract::Host;
use bitcoin::{
    bech32,
    hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256},
    secp256k1::{PublicKey, Secp256k1, XOnlyPublicKey, ecdsa::Signature},
};
use lightning_invoice::Bolt11Invoice;
use lnurl_models::{
    CheckUsernameAvailableRequest, CheckUsernameAvailableResponse, ListMetadataRequest,
    ListMetadataResponse, RecoverLnurlPayRequest, RecoverLnurlPayResponse, RegisterLnurlPayRequest,
    RegisterLnurlPayResponse, TransferLnurlPayRequest, TransferLnurlPayResponse,
    UnregisterLnurlPayRequest, sanitize_username, signed_message,
};
use nostr::{Alphabet, Event, JsonUtil, Kind, TagStandard};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use spark::utils::verify_signature::verify_signature_ecdsa;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::{debug, error, info, trace, warn};

use crate::{
    invoice_paid::{create_invoice, handle_invoice_paid},
    repository::LnurlSenderComment,
    time::{now_millis, now_u64},
    zap::{Zap, derive_user_nostr_keys},
};
use crate::{
    repository::{
        LnurlRepository, LnurlRepositoryError, NameStatus, StatementClaim, TransferRequest,
    },
    state::State,
    user::{USERNAME_VALIDATION_REGEX, User},
};

/// Named locally for how it reads at the call sites, defined in `lnurl-models`
/// so this server and every client bound a timestamp identically.
const ACCEPTABLE_TIME_DIFF_SECS: u64 = signed_message::VALIDITY_SECS;
/// LUD-17 scheme prefixes an `lnurl` tag may carry in place of http(s).
const LNURL_SCHEME_PREFIXES: [&str; 4] = ["lnurlp://", "lnurlw://", "lnurlc://", "keyauth://"];
const DEFAULT_METADATA_OFFSET: u32 = 0;
const DEFAULT_METADATA_LIMIT: u32 = 100;
/// Maximum size (bytes) of a nostr event JSON (zap request or zap receipt).
const MAX_NOSTR_EVENT_SIZE: usize = 32_768;
/// Maximum length of a sender comment (LUD-12).
const MAX_COMMENT_LENGTH: usize = 255;
/// Where `list_metadata` reads its credential from, in preference to the query
/// string. A GET's query string lands in proxy and access logs; the response
/// carries preimages.
const METADATA_SIGNATURE_HEADER: &str = "x-breez-signature";
const METADATA_TIMESTAMP_HEADER: &str = "x-breez-timestamp";
/// How often the accumulated legacy signed-message counts are logged.
const LEGACY_REPORT_INTERVAL: Duration = Duration::from_mins(1);

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
    DB: LnurlRepository + crate::webhooks::WebhookRepository + Clone + Send + Sync + 'static,
{
    pub async fn available(
        Host(host): Host,
        Path(identifier): Path<String>,
        Extension(state): Extension<State<DB>>,
    ) -> Result<Json<CheckUsernameAvailableResponse>, (StatusCode, Json<Value>)> {
        let username = sanitize_username(&identifier);
        validate_username(&username)?;
        let domain = sanitize_domain(&state, &host).await?;
        // Answers for nobody, so a name held for the pubkey that gave it up
        // reads as unavailable here too. Telling that pubkey apart would mean
        // saying who a name is held for, and this route carries no signature.
        // `available_for_pubkey` is the one that answers a wallet about its own
        // released name.
        let status = name_status(&state, &domain, &username, None).await?;

        Ok(Json(CheckUsernameAvailableResponse {
            available: status == NameStatus::Free,
        }))
    }

    /// The availability check answered for the pubkey that signed it, which is
    /// what lets a wallet see a name it gave up as one it may take back.
    pub async fn available_for_pubkey(
        Host(host): Host,
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<CheckUsernameAvailableRequest>,
    ) -> Result<Json<CheckUsernameAvailableResponse>, (StatusCode, Json<Value>)> {
        let CheckUsernameAvailableRequest {
            username,
            signature,
            timestamp,
        } = payload;
        let username = sanitize_username(&username);
        validate_username(&username)?;
        let domain = sanitize_domain(&state, &host).await?;

        let (identity, signature) = parse_signed_request(&pubkey, &signature)?;
        if !timestamp_is_fresh(timestamp) {
            return Err(invalid_timestamp());
        }
        // Claims nothing, for the same reason as `recover`. No legacy message
        // either: the route is new, so every signature it ever sees is v2.
        let message =
            signed_message::available(&domain, &identity.to_string(), &username, timestamp);
        let secp = Secp256k1::new();
        if verify_signature_ecdsa(&secp, &message, &signature, &identity).is_err() {
            trace!("invalid signature for availability check on domain '{domain}'");
            return Err(invalid_signature_for_domain(&domain));
        }

        let status = name_status(&state, &domain, &username, Some(&identity.to_string())).await?;

        Ok(Json(CheckUsernameAvailableResponse {
            available: status == NameStatus::Free,
        }))
    }

    pub async fn register(
        Host(host): Host,
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<RegisterLnurlPayRequest>,
    ) -> Result<Json<RegisterLnurlPayResponse>, (StatusCode, Json<Value>)> {
        let RegisterLnurlPayRequest {
            username,
            signature,
            timestamp,
            description,
        } = payload;
        let username = sanitize_username(&username);
        validate_username(&username)?;
        // The description and the resolved domain are both fields of the v2
        // message, so both are settled before anything verifies a signature.
        // Resolving the domain first also refuses a request naming a host this
        // server does not serve without spending the statement.
        validate_description(&description)?;
        let domain = sanitize_domain(&state, &host).await?;

        let (pubkey, signature) = parse_signed_request(&pubkey, &signature)?;
        if !timestamp_is_fresh(timestamp) {
            return Err(invalid_timestamp());
        }
        let candidates = register_candidates(&domain, &username, &description, timestamp);
        let candidate = verify_candidates(&pubkey, &signature, &candidates, &domain)?;
        // Register and the legacy unregister candidate cover identical bytes,
        // so claiming here spends the statement for both routes. The claim
        // outlives that candidate: it also keeps a spent statement from
        // resurrecting an address the pubkey has since unregistered.
        claim_statement(&state, &pubkey, candidate).await?;

        let user = User {
            domain,
            pubkey: pubkey.to_string(),
            name: username,
            description,
        };

        if let Err(e) = state.db.upsert_user(&user).await {
            match e {
                LnurlRepositoryError::NameTaken => {
                    trace!("name already taken: {}", user.name);
                    return Err((
                        StatusCode::CONFLICT,
                        Json(Value::String("name already taken".into())),
                    ));
                }
                LnurlRepositoryError::NameReserved => {
                    trace!(
                        "name reserved for the pubkey that released it: {}",
                        user.name
                    );
                    return Err((
                        StatusCode::CONFLICT,
                        Json(Value::String("name is reserved".into())),
                    ));
                }
                e => {
                    error!("failed to execute query: {}", e);
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(Value::String("internal server error".into())),
                    ));
                }
            }
        }

        debug!("registered user '{}' for pubkey {}", user.name, pubkey);
        let lnurl = format!("lnurlp://{}/lnurlp/{}", user.domain, user.name);
        Ok(Json(RegisterLnurlPayResponse {
            lnurl,
            lightning_address: format!("{}@{}", user.name, user.domain),
        }))
    }

    #[allow(clippy::too_many_lines)]
    pub async fn transfer(
        Host(host): Host,
        Path(to_pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<TransferLnurlPayRequest>,
    ) -> Result<Json<TransferLnurlPayResponse>, (StatusCode, Json<Value>)> {
        let TransferLnurlPayRequest {
            username,
            description,
            from_pubkey,
            from_signature,
            to_signature,
            timestamp,
        } = payload;
        let username = sanitize_username(&username);
        validate_username(&username)?;
        validate_description(&description)?;
        let domain = sanitize_domain(&state, &host).await?;

        let (from_pk, from_sig) = parse_signed_request(&from_pubkey, &from_signature)?;
        let (to_pk, to_sig) = parse_signed_request(&to_pubkey, &to_signature)?;

        // A timestamp selects the v2 messages on both signatures. Bounded with
        // the same symmetric window as every other route: an asymmetric
        // future bound would cap a stolen authorization slightly tighter at the
        // cost of a route that fails on a device where every other route works.
        if let Some(timestamp) = timestamp
            && !timestamp_is_fresh(timestamp)
        {
            return Err(transfer_expired());
        }

        // Role-tagged, so the current owner's signature does not verify in the
        // transferee's slot. The transferee's message commits to the description
        // because the transferee is the one choosing it.
        let from_candidates =
            transfer_from_candidates(&domain, &username, &from_pk, &to_pk, &to_pubkey, timestamp);
        let to_candidates = transfer_to_candidates(
            &domain,
            &username,
            &from_pk,
            &to_pk,
            &to_pubkey,
            &description,
            timestamp,
        );
        let authorization = verify_candidates(&from_pk, &from_sig, &from_candidates, &domain)?;
        verify_candidates(&to_pk, &to_sig, &to_candidates, &domain)?;

        if from_pk == to_pk {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(Value::String(
                    "transfer source and target are the same pubkey".into(),
                )),
            ));
        }

        // Claimed in the same transaction as the transfer, so a transfer that
        // rolls back leaves the statement actionable.
        if let Err(e) = state
            .db
            .transfer_username(
                TransferRequest {
                    domain: &domain,
                    from_pubkey: &from_pk.to_string(),
                    to_pubkey: &to_pk.to_string(),
                    username: &username,
                    description: &description,
                },
                StatementClaim {
                    hash: &statement_hash(&from_pk, &authorization.message),
                    expires_at: authorization.expiry.expires_at(),
                },
            )
            .await
        {
            return Err(match e {
                LnurlRepositoryError::SourceNotOwner => {
                    trace!("transfer source pubkey does not own username '{username}'");
                    (
                        StatusCode::NOT_FOUND,
                        Json(Value::String(
                            "source pubkey does not own this username".into(),
                        )),
                    )
                }
                LnurlRepositoryError::NameTaken => {
                    trace!("name already taken during transfer: {username}");
                    (
                        StatusCode::CONFLICT,
                        Json(Value::String("name already taken".into())),
                    )
                }
                LnurlRepositoryError::StatementAlreadyUsed => {
                    trace!("transfer of '{username}' from {from_pk} to {to_pk} already performed");
                    (
                        StatusCode::CONFLICT,
                        Json(Value::String("signature has already been used".into())),
                    )
                }
                // A held name has no owner to transfer it, so the source check
                // inside the transfer answers first and a transfer never
                // reports a name as reserved. Matched rather than folded into a
                // catch-all, so a transfer that starts checking holds has to
                // answer for the status code here.
                LnurlRepositoryError::NameReserved | LnurlRepositoryError::General(_) => {
                    error!("failed to execute transfer query: {e}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(Value::String("internal server error".into())),
                    )
                }
            });
        }

        debug!("transferred '{username}' from {from_pk} to {to_pk}");
        let lnurl = format!("lnurlp://{domain}/lnurlp/{username}");
        Ok(Json(TransferLnurlPayResponse {
            lnurl,
            lightning_address: format!("{username}@{domain}"),
        }))
    }

    pub async fn unregister(
        Host(host): Host,
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<UnregisterLnurlPayRequest>,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        let UnregisterLnurlPayRequest {
            username,
            signature,
            timestamp,
        } = payload;
        let username = sanitize_username(&username);
        // Resolved before verification because the v2 message names it, and
        // before the claim so a request naming a host this server does not serve
        // is refused without spending the statement.
        let domain = sanitize_domain(&state, &host).await?;

        let (pubkey, signature) = parse_signed_request(&pubkey, &signature)?;
        if !timestamp_is_fresh(timestamp) {
            return Err(invalid_timestamp());
        }
        let candidates = unregister_candidates(&domain, &username, timestamp);
        let candidate = verify_candidates(&pubkey, &signature, &candidates, &domain)?;
        // Register claims the same bare statement, so a statement spent there
        // is already claimed here. The claim outlives that candidate: it also
        // keeps a spent statement from acting again after the name is
        // re-registered.
        claim_statement(&state, &pubkey, candidate).await?;

        let registered = state
            .db
            .get_user_by_pubkey(&domain, &pubkey.to_string())
            .await
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?;

        match unregister_action(&username, registered.as_ref()) {
            UnregisterAction::Delete => {}
            UnregisterAction::AlreadyGone => {
                debug!("pubkey {pubkey} holds no address, nothing to unregister");
                return Ok(());
            }
            UnregisterAction::NameMismatch => {
                debug!(
                    "unregister signature names '{username}', not the address pubkey {pubkey} holds"
                );
                return Err(unregister_name_mismatch());
            }
        }

        let removed = state
            .db
            .delete_user(&domain, &pubkey.to_string(), &username)
            .await
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?;

        if !removed {
            debug!("address for pubkey {pubkey} changed while unregistering '{username}'");
            return Err(unregister_name_mismatch());
        }

        debug!("unregistered user for pubkey {}", pubkey);
        Ok(())
    }

    pub async fn recover(
        Host(host): Host,
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<RecoverLnurlPayRequest>,
    ) -> Result<Json<RecoverLnurlPayResponse>, (StatusCode, Json<Value>)> {
        let RecoverLnurlPayRequest {
            signature,
            timestamp,
        } = payload;
        let domain = sanitize_domain(&state, &host).await?;

        let (identity, signature) = parse_signed_request(&pubkey, &signature)?;
        if !timestamp_is_fresh(timestamp) {
            return Err(invalid_timestamp());
        }
        let candidates = recover_candidates(&domain, &identity, &pubkey, timestamp);
        // Claims nothing: this is a read, and claiming it would break a
        // legitimate client retry.
        verify_candidates(&identity, &signature, &candidates, &domain)?;

        let user = state
            .db
            .get_user_by_pubkey(&domain, &identity.to_string())
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
        Host(host): Host,
        Path(pubkey): Path<String>,
        headers: HeaderMap,
        Query(params): Query<ListMetadataRequest>,
        Extension(state): Extension<State<DB>>,
    ) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
        let ListMetadataRequest {
            signature,
            timestamp,
            offset,
            limit,
            updated_after,
        } = params;
        let (signature, timestamp) = metadata_credentials(&headers, signature, timestamp)?;
        let domain = sanitize_domain(&state, &host).await?;

        let (identity, signature) = parse_signed_request(&pubkey, &signature)?;
        if !timestamp_is_fresh(timestamp) {
            return Err(invalid_timestamp());
        }
        let candidates = metadata_candidates(&domain, &identity, &pubkey, timestamp);
        // Claims nothing, for the same reason as `recover`.
        verify_candidates(&identity, &signature, &candidates, &domain)?;

        // Rows are keyed by the identity pubkey, which is the same principal on
        // every domain a deployment serves, so they are not filtered by domain:
        // the domain in the message is doing anti-replay work, not authorization
        // work.
        let metadata = state
            .db
            .get_metadata_by_pubkey(
                &identity.to_string(),
                offset.unwrap_or(DEFAULT_METADATA_OFFSET),
                limit.unwrap_or(DEFAULT_METADATA_LIMIT),
                updated_after,
            )
            .await
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?;

        // The response carries preimages, so it must not be retained by any
        // cache between here and the client.
        Ok((
            [(header::CACHE_CONTROL, "no-store, private")],
            Json(ListMetadataResponse { metadata }),
        ))
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
            .get_user_by_name(&sanitize_domain(&state, &host).await?, &username)
            .await
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                lnurl_error("internal server error")
            })?;

        let Some(user) = user else {
            return Err((StatusCode::NOT_FOUND, Json(Value::String(String::new()))));
        };

        let nostr_pubkey = user_nostr_pubkey(state.nostr_keys.as_ref(), &user.pubkey)?;
        let allows_nostr = nostr_pubkey.is_some().then_some(true);
        Ok(Json(PayResponse {
            callback: format!(
                "{}://{}/lnurlp/{}/invoice",
                state.scheme, &user.domain, user.name
            ),
            max_sendable: state.max_sendable,
            min_sendable: state.min_sendable,
            tag: Tag::Pay,
            metadata: get_metadata(&user.domain, &user),
            #[allow(clippy::cast_possible_truncation)]
            comment_allowed: Some(MAX_COMMENT_LENGTH as u32),
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
        let domain = sanitize_domain(&state, &host).await?;

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

        validate_amount_bounds(amount_msat, state.min_sendable, state.max_sendable)?;

        let zap_request = match &params.nostr {
            Some(event) => {
                let Some(receipt_pubkey) =
                    user_nostr_pubkey(state.nostr_keys.as_ref(), &user.pubkey)?
                else {
                    trace!("nostr zap not supported");
                    return Err(lnurl_error("nostr zap not supported"));
                };

                if event.len() > MAX_NOSTR_EVENT_SIZE {
                    return Err(lnurl_error("nostr event too large"));
                }

                let event = Event::from_json(event).map_err(|e| {
                    trace!("invalid nostr event, could not parse: {}", e);
                    lnurl_error("invalid nostr event")
                })?;
                validate_nostr_zap_request(amount_msat, &event, receipt_pubkey)?;
                log_lnurl_tag_mismatch(&event, &domain, &user.name);

                Some(event)
            }
            None => None,
        };

        let desc_hash = if let Some(event) = &zap_request {
            sha256::Hash::hash(event.as_json().as_bytes())
        } else {
            let metadata = get_metadata(&user.domain, &user);
            sha256::Hash::hash(metadata.as_bytes())
        };

        let pubkey = parse_pubkey(&user.pubkey)?;
        let wallet = state.invoice_wallet(&domain).await;
        let res = wallet
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
        if let Some(event) = zap_request {
            let zap = Zap {
                payment_hash: payment_hash.clone(),
                // Canonical form, matching the description hash the invoice
                // commits to and the receipt's description tag.
                zap_request: event.as_json(),
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
            if comment.len() > MAX_COMMENT_LENGTH {
                return Err(lnurl_error("comment too long"));
            }
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

        // Store invoice for LUD-21 verify support and webhook delivery
        if let Err(e) = create_invoice(
            &state.db,
            &payment_hash,
            &user.pubkey,
            &res.invoice,
            invoice_expiry,
            &domain,
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

    /// Webhook endpoint for SSP payment notifications.
    /// Verifies HMAC-SHA256 signature and processes payment preimages.
    pub async fn webhook(
        Extension(state): Extension<State<DB>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        process_webhook(
            &state.db,
            &state.webhook_service,
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
    webhook_service: &crate::webhooks::WebhookService<DB>,
    webhook_secret: &str,
    invoice_paid_trigger: &tokio::sync::watch::Sender<()>,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<(), (StatusCode, Json<Value>)>
where
    DB: LnurlRepository + crate::webhooks::WebhookRepository + Clone + Send + Sync + 'static,
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
    let payload: SspWebhookPayload = serde_json::from_slice(body).map_err(|e| {
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

    let amount_received_sat = match &payload.htlc_amount {
        Some(amount) if amount.unit == "SATOSHI" => Some(amount.value),
        Some(amount) if amount.unit == "MILLISATOSHI" => {
            if amount.value % 1000 != 0 {
                warn!(
                    "truncating htlc_amount from {} msat to {} sat",
                    amount.value,
                    amount.value / 1000
                );
            }
            Some(amount.value / 1000)
        }
        Some(amount) => {
            warn!("unexpected htlc_amount unit: {}", amount.unit);
            None
        }
        None => None,
    };

    // Handle the invoice paid event
    if let Err(e) = handle_invoice_paid(
        db,
        webhook_service,
        &payment_hash,
        &payment_preimage,
        amount_received_sat,
        invoice_paid_trigger,
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
        payment_hash, receiver_pubkey
    );
    Ok(())
}

fn validate_nostr_zap_request(
    amount_msat: u64,
    event: &Event,
    receipt_pubkey: XOnlyPublicKey,
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

    // 8. There MUST be 0 or 1 P tags. If there is one, it MUST be equal to the
    // zap receipt's pubkey, which is the key this user's receipts are signed
    // with. A sender holds it already, having read it from the LNURL-pay
    // response, so setting it pins the key they expect to see sign the receipt.
    let mut uppercase_p_tags = event.tags.iter().filter(|t| {
        t.single_letter_tag()
            .is_some_and(|t| t.is_uppercase() && t.character == Alphabet::P)
    });
    if let Some(tag) = uppercase_p_tags.next() {
        if uppercase_p_tags.next().is_some() {
            trace!("invalid nostr event, multiple 'P' tags");
            return Err(lnurl_error("invalid nostr event"));
        }

        let pinned_signer = tag
            .content()
            .and_then(|content| nostr::PublicKey::parse(content).ok())
            .and_then(|pubkey| pubkey.xonly().ok());
        if pinned_signer != Some(receipt_pubkey) {
            trace!("invalid nostr event, 'P' tag is not the receipt signer");
            return Err(lnurl_error("invalid nostr event"));
        }
    }

    Ok(())
}

/// The `nostrPubkey` an address advertises. NIP-57 validates a receipt against
/// this, so it must be the key the publisher signs that address's receipts
/// with. `None` when the service runs without a nostr key, which is also how it
/// reports that it supports no zaps.
fn user_nostr_pubkey(
    nostr_keys: Option<&nostr::Keys>,
    owner_pubkey: &str,
) -> Result<Option<XOnlyPublicKey>, (StatusCode, Json<Value>)> {
    let Some(nostr_keys) = nostr_keys else {
        return Ok(None);
    };

    let xonly_pubkey = derive_user_nostr_keys(nostr_keys, owner_pubkey)
        .map_err(|e| {
            error!("could not derive the receipt signing key: {e}");
            lnurl_error("internal server error")
        })?
        .public_key
        .xonly()
        .map_err(|e| {
            error!("derived nostr pubkey could not be parsed: {:?}", e);
            lnurl_error("internal server error")
        })?;
    Ok(Some(xonly_pubkey))
}

/// Warn when a zap request names an lnurl other than the address being paid,
/// which NIP-57 expects to match.
///
/// Logged rather than rejected while the shape of real traffic is measured:
/// senders emit bech32, bare lightning addresses and plain URLs, and a
/// deployment serving several domains may see legitimate cross-domain values.
fn log_lnurl_tag_mismatch(event: &Event, domain: &str, username: &str) {
    let Some(tag) = event.tags.iter().find_map(|t| {
        if let Some(TagStandard::Lnurl(lnurl)) = t.as_standardized() {
            Some(lnurl.clone())
        } else {
            None
        }
    }) else {
        return;
    };

    match decode_lnurl_tag(&tag) {
        Some(decoded) if lnurl_targets_address(&decoded, domain, username) => {}
        Some(decoded) => warn!(
            "zap request lnurl tag names {}, but the invoice is for {username}@{domain}",
            for_log(&decoded)
        ),
        None => warn!(
            "zap request lnurl tag could not be decoded: {}",
            for_log(&tag)
        ),
    }
}

/// Bound a sender-supplied value before it reaches the log. A tag carries
/// whatever fits in a zap request, which is `MAX_NOSTR_EVENT_SIZE` bytes.
fn for_log(value: &str) -> String {
    /// Longer than any real lnurl, which runs to a couple of hundred
    /// characters in its bech32 form.
    const MAX_LOGGED_CHARS: usize = 256;

    let mut logged: String = value.chars().take(MAX_LOGGED_CHARS).collect();
    if logged.chars().count() < value.chars().count() {
        logged.push_str("... (truncated)");
    }
    logged
}

/// Resolve an `lnurl` tag to the http(s) URL it stands for. Accepts the bech32
/// form the spec describes plus the bare lightning address and plain URL forms
/// senders also emit.
fn decode_lnurl_tag(tag: &str) -> Option<String> {
    if let Ok((hrp, data)) = bech32::decode(tag)
        && hrp.to_lowercase() == "lnurl"
    {
        return String::from_utf8(data).ok();
    }

    if let Some((name, domain)) = tag.split_once('@')
        && !name.is_empty()
        && !domain.is_empty()
        && !domain.contains('/')
    {
        return Some(format!("https://{domain}/.well-known/lnurlp/{name}"));
    }

    tag.contains("://").then(|| tag.to_string())
}

/// Whether an lnurl-pay URL addresses `username` at `domain`, across the LUD-16
/// well-known path and the shorter path this service also serves.
fn lnurl_targets_address(lnurl: &str, domain: &str, username: &str) -> bool {
    let rest = LNURL_SCHEME_PREFIXES
        .iter()
        .find_map(|prefix| lnurl.strip_prefix(prefix))
        .or_else(|| lnurl.split_once("://").map(|(_, rest)| rest))
        .unwrap_or(lnurl)
        .trim_end_matches('/')
        .to_lowercase();

    let domain = domain.to_lowercase();
    let username = sanitize_username(username);
    rest == format!("{domain}/.well-known/lnurlp/{username}")
        || rest == format!("{domain}/lnurlp/{username}")
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

/// Enforce the advertised LUD-06 min/max sendable bounds on the callback. The
/// sending wallet is expected to honor them, but a direct client can request
/// any amount, so the service rejects out-of-bounds amounts itself. Bounds and
/// `amount_msat` are all in millisatoshi.
fn validate_amount_bounds(
    amount_msat: u64,
    min_sendable: u64,
    max_sendable: u64,
) -> Result<(), (StatusCode, Json<Value>)> {
    if amount_msat < min_sendable || amount_msat > max_sendable {
        trace!(
            "amount out of bounds: {amount_msat} msat, allowed {min_sendable}..={max_sendable} msat"
        );
        return Err(lnurl_error("amount out of bounds"));
    }
    Ok(())
}

fn validate_description(description: &str) -> Result<(), (StatusCode, Json<Value>)> {
    if description.chars().take(256).count() > 255 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(Value::String("description too long".into())),
        ));
    }
    Ok(())
}

/// Take the metadata request's signature and timestamp from the headers,
/// falling back to the query string only where a header is absent.
///
/// A present header wins outright, and one that fails to parse is refused
/// rather than falling through, so a query parameter can never override what a
/// client put in the header. The response carries preimages, which is why the
/// credential belongs in a header: a query string lands in proxy and access
/// logs.
fn metadata_credentials(
    headers: &HeaderMap,
    query_signature: Option<String>,
    query_timestamp: Option<u64>,
) -> Result<(String, u64), (StatusCode, Json<Value>)> {
    let signature = match headers.get(METADATA_SIGNATURE_HEADER) {
        Some(value) => Some(
            value
                .to_str()
                .map_err(|_| malformed_header(METADATA_SIGNATURE_HEADER))?
                .to_string(),
        ),
        None => query_signature,
    };
    let timestamp = match headers.get(METADATA_TIMESTAMP_HEADER) {
        Some(value) => Some(
            value
                .to_str()
                .ok()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| malformed_header(METADATA_TIMESTAMP_HEADER))?,
        ),
        None => query_timestamp,
    };

    match (signature, timestamp) {
        (Some(signature), Some(timestamp)) => Ok((signature, timestamp)),
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(Value::String("missing signature or timestamp".into())),
        )),
    }
}

fn malformed_header(name: &str) -> (StatusCode, Json<Value>) {
    trace!("malformed '{name}' header");
    (
        StatusCode::BAD_REQUEST,
        Json(Value::String(format!("malformed '{name}' header"))),
    )
}

/// Parse the identity pubkey and DER signature a signed request carries.
/// Independent of the message the signature covers.
fn parse_signed_request(
    pubkey: &str,
    signature: &str,
) -> Result<(PublicKey, Signature), (StatusCode, Json<Value>)> {
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

    Ok((pubkey, signature))
}

/// Whether a signed request's timestamp is inside the accept window, bounded
/// symmetrically so a device whose clock runs fast still works.
fn timestamp_is_fresh(timestamp: u64) -> bool {
    is_fresh_at(timestamp, now_u64())
}

fn is_fresh_at(timestamp: u64, now: u64) -> bool {
    let diff = timestamp.abs_diff(now);
    if diff > ACCEPTABLE_TIME_DIFF_SECS {
        trace!(
            "invalid timestamp, too far off: {}, now: {}, diff: {}",
            timestamp, now, diff
        );
        return false;
    }
    true
}

fn invalid_timestamp() -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(Value::String("invalid timestamp".into())),
    )
}

/// Distinct from a rejected signature so a partner can tell a stale
/// authorization from a mis-signed one. Both sides sign the same timestamp, so
/// this covers a handover that took longer than the accept window.
fn transfer_expired() -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(Value::String(
            "transfer authorization is expired or not yet valid".into(),
        )),
    )
}

/// Naming the resolved domain separates "signed for another domain" from "not
/// signed by this key", which is the difference between diagnosing a proxy that
/// rewrites `Host` in minutes or with a packet capture. The caller chose the
/// header that resolved to this domain, so the response tells it nothing new.
fn invalid_signature_for_domain(domain: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(Value::String(format!(
            "invalid signature for domain '{domain}'"
        ))),
    )
}

/// Append the timestamp the way every pre-v2 message did.
fn legacy_timestamped(message: &str, timestamp: u64) -> String {
    format!("{message}-{timestamp}")
}

/// The messages a register signature may cover, most-preferred first.
fn register_candidates(
    domain: &str,
    username: &str,
    description: &str,
    timestamp: u64,
) -> Vec<Candidate> {
    vec![
        Candidate::v2(
            signed_message::register(domain, username, description, timestamp),
            SignedRoute::Register,
            timestamp,
        ),
        // Commits to neither the domain nor the description, so a legacy
        // signature can still be aimed at another served domain by header.
        // TODO: drop at the legacy cutoff, and flip
        // `register_signature_still_verifies_against_the_legacy_message`.
        Candidate::legacy(
            legacy_timestamped(username, timestamp),
            SignedRoute::Register,
            ClaimExpiry::Bounded(timestamp),
        ),
    ]
}

/// The messages an unregister signature may cover, most-preferred first.
///
/// The legacy `unregister:` prefix domain-separates deletion from the other
/// pre-v2 messages. The bare legacy form is `register`'s own, and a statement
/// claimed by one route is spent for the other.
///
/// TODO: drop both legacy candidates at the cutoff.
fn unregister_candidates(domain: &str, username: &str, timestamp: u64) -> Vec<Candidate> {
    vec![
        Candidate::v2(
            signed_message::unregister(domain, username, timestamp),
            SignedRoute::Unregister,
            timestamp,
        ),
        Candidate::legacy(
            legacy_timestamped(&format!("unregister:{username}"), timestamp),
            SignedRoute::Unregister,
            ClaimExpiry::Bounded(timestamp),
        ),
        Candidate::legacy(
            legacy_timestamped(username, timestamp),
            SignedRoute::Unregister,
            ClaimExpiry::Bounded(timestamp),
        ),
    ]
}

/// The messages a recover signature may cover, most-preferred first.
///
/// `raw_pubkey` is the path segment verbatim, which is what the legacy message
/// was built from; the v2 message uses the parsed, normalized form.
fn recover_candidates(
    domain: &str,
    pubkey: &PublicKey,
    raw_pubkey: &str,
    timestamp: u64,
) -> Vec<Candidate> {
    vec![
        Candidate::v2(
            signed_message::recover(domain, &pubkey.to_string(), timestamp),
            SignedRoute::Recover,
            timestamp,
        ),
        // Byte-identical to the legacy metadata message, so a legacy recover
        // signature is a valid credential for the metadata route until the
        // cutoff. TODO: drop at the legacy cutoff.
        Candidate::legacy(
            legacy_timestamped(raw_pubkey, timestamp),
            SignedRoute::Recover,
            ClaimExpiry::Bounded(timestamp),
        ),
    ]
}

/// The messages a metadata signature may cover, most-preferred first. See
/// [`recover_candidates`] for the shared legacy message.
fn metadata_candidates(
    domain: &str,
    pubkey: &PublicKey,
    raw_pubkey: &str,
    timestamp: u64,
) -> Vec<Candidate> {
    vec![
        Candidate::v2(
            signed_message::metadata(domain, &pubkey.to_string(), timestamp),
            SignedRoute::Metadata,
            timestamp,
        ),
        // TODO: drop at the legacy cutoff.
        Candidate::legacy(
            legacy_timestamped(raw_pubkey, timestamp),
            SignedRoute::Metadata,
            ClaimExpiry::Bounded(timestamp),
        ),
    ]
}

/// The message the current owner's transfer signature may cover.
///
/// A timestamp selects v2 outright with no legacy fallback, which is what makes
/// stripping it from a v2 request fail rather than downgrade.
fn transfer_from_candidates(
    domain: &str,
    username: &str,
    from_pubkey: &PublicKey,
    to_pubkey: &PublicKey,
    raw_to_pubkey: &str,
    timestamp: Option<u64>,
) -> Vec<Candidate> {
    match timestamp {
        Some(timestamp) => vec![Candidate::v2(
            signed_message::transfer_from(
                domain,
                username,
                &from_pubkey.to_string(),
                &to_pubkey.to_string(),
                timestamp,
            ),
            SignedRoute::Transfer,
            timestamp,
        )],
        None => vec![legacy_transfer_candidate(username, raw_to_pubkey)],
    }
}

/// The message the transferee's signature may cover. Role-tagged and
/// description-committing, so it is not the bytes the current owner signed.
fn transfer_to_candidates(
    domain: &str,
    username: &str,
    from_pubkey: &PublicKey,
    to_pubkey: &PublicKey,
    raw_to_pubkey: &str,
    description: &str,
    timestamp: Option<u64>,
) -> Vec<Candidate> {
    match timestamp {
        Some(timestamp) => vec![Candidate::v2(
            signed_message::transfer_to(
                domain,
                username,
                &from_pubkey.to_string(),
                &to_pubkey.to_string(),
                description,
                timestamp,
            ),
            SignedRoute::Transfer,
            timestamp,
        )],
        None => vec![legacy_transfer_candidate(username, raw_to_pubkey)],
    }
}

/// The pre-v2 transfer message, signed identically by both parties: it names
/// neither the domain, nor the time, nor the current owner, so one pair of
/// signatures authorizes one transfer wherever it is submitted. Its claim is
/// therefore never pruned.
///
/// TODO: drop at the legacy cutoff, along with the accumulated unbounded rows.
fn legacy_transfer_candidate(username: &str, raw_to_pubkey: &str) -> Candidate {
    Candidate::legacy(
        format!("transfer:{username}-{raw_to_pubkey}"),
        SignedRoute::Transfer,
        ClaimExpiry::Unbounded,
    )
}

/// The response when the signed name is not the one the pubkey holds, whether
/// the read saw that or the delete raced with a change.
fn unregister_name_mismatch() -> (StatusCode, Json<Value>) {
    (
        StatusCode::CONFLICT,
        Json(Value::String(
            "signature does not cover the registered address".into(),
        )),
    )
}

/// What an unregister request does to the address a pubkey holds.
#[derive(Debug, PartialEq, Eq)]
enum UnregisterAction {
    /// The signature names the registered address: remove it.
    Delete,
    /// The pubkey holds no address, so the request's goal already holds.
    AlreadyGone,
    /// The pubkey holds an address the signature does not name.
    NameMismatch,
}

/// Decide an unregister from the name the signature covers and the address the
/// pubkey holds.
///
/// The signature approves removing one specific name, so a name it does not
/// cover is never removed, whichever address the pubkey currently holds.
fn unregister_action(signed_name: &str, registered: Option<&User>) -> UnregisterAction {
    match registered {
        None => UnregisterAction::AlreadyGone,
        Some(user) if user.name == signed_name => UnregisterAction::Delete,
        Some(_) => UnregisterAction::NameMismatch,
    }
}

/// A route whose requests carry an identity-key signature.
///
/// Names the claim rows a route writes and the legacy counter it increments.
/// `Recover` and `Metadata` claim nothing: they are reads, and claiming them
/// would break legitimate client retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignedRoute {
    Register,
    Unregister,
    Transfer,
    Recover,
    Metadata,
}

impl SignedRoute {
    const ALL: [Self; 5] = [
        Self::Register,
        Self::Unregister,
        Self::Transfer,
        Self::Recover,
        Self::Metadata,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::Unregister => "unregister",
            Self::Transfer => "transfer",
            Self::Recover => "recover",
            Self::Metadata => "metadata",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Register => 0,
            Self::Unregister => 1,
            Self::Transfer => 2,
            Self::Recover => 3,
            Self::Metadata => 4,
        }
    }
}

/// How long the claim over a message must be retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimExpiry {
    /// The message carries this timestamp, so the claim may be pruned once
    /// [`timestamp_is_fresh`] rejects it on its own.
    ///
    /// Retention is tied to the accept window: widening
    /// `ACCEPTABLE_TIME_DIFF_SECS` requires extending retention first.
    Bounded(u64),
    /// Nothing bounds the message in time. Dropping the claim would put the
    /// statement back in play, since no other check refuses it later.
    Unbounded,
}

impl ClaimExpiry {
    fn expires_at(self) -> i64 {
        match self {
            // Saturating: an unrepresentable expiry only retains the claim
            // longer, and the timestamp is already bounded to now anyway.
            Self::Bounded(timestamp) => {
                i64::try_from(timestamp.saturating_add(ACCEPTABLE_TIME_DIFF_SECS))
                    .unwrap_or(i64::MAX)
            }
            Self::Unbounded => i64::MAX,
        }
    }
}

/// A message a signature may cover, carrying everything the route needs once it
/// matches.
///
/// Pairing the message with its expiry here, rather than at the claim call, is
/// what keeps a route from claiming an untimestamped message with a prunable
/// expiry, which would let the claim be forgotten while the message stays
/// valid.
struct Candidate {
    message: String,
    expiry: ClaimExpiry,
    route: SignedRoute,
    /// A pre-v2 message, accepted only for the compatibility window.
    legacy: bool,
}

impl Candidate {
    fn v2(message: String, route: SignedRoute, timestamp: u64) -> Self {
        Self {
            message,
            expiry: ClaimExpiry::Bounded(timestamp),
            route,
            legacy: false,
        }
    }

    fn legacy(message: String, route: SignedRoute, expiry: ClaimExpiry) -> Self {
        Self {
            message,
            expiry,
            route,
            legacy: true,
        }
    }
}

/// Legacy verifies since the last flush, per route.
///
/// Counted rather than logged per verify: these are unauthenticated paths, so a
/// line per legacy verify would be caller-driven unbounded log volume. The tail
/// reading zero everywhere for 30 consecutive days is the gate on dropping the
/// legacy candidates.
static LEGACY_VERIFIES: [AtomicU64; SignedRoute::ALL.len()] =
    [const { AtomicU64::new(0) }; SignedRoute::ALL.len()];

/// Target the legacy-usage line is logged on, so a deployment can route it
/// somewhere durable without keeping the rest of this module's output.
pub const LEGACY_SIGNATURE_TARGET: &str = "breez_lnurl::legacy_signatures";

fn record_legacy_verify(route: SignedRoute) {
    LEGACY_VERIFIES[route.index()].fetch_add(1, Ordering::Relaxed);
}

/// Take and reset the counts since the last drain, one entry per route
/// including the zeroes: a route silently missing from the report cannot be
/// told apart from a route that has stopped seeing legacy traffic.
fn drain_legacy_verifies() -> Vec<(&'static str, u64)> {
    SignedRoute::ALL
        .iter()
        .map(|route| {
            (
                route.as_str(),
                LEGACY_VERIFIES[route.index()].swap(0, Ordering::Relaxed),
            )
        })
        .collect()
}

/// Log one line per interval, so the pre-v2 tail is observable before the
/// legacy candidates are dropped.
pub fn spawn_legacy_signature_reporter() {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(LEGACY_REPORT_INTERVAL).await;
            let counts: Vec<String> = drain_legacy_verifies()
                .into_iter()
                .map(|(route, count)| format!("{route}={count}"))
                .collect();
            info!(
                target: LEGACY_SIGNATURE_TARGET,
                "legacy signed-message verifies: {}",
                counts.join(" ")
            );
        }
    });
}

/// Verify `signature` against each candidate, accepting the first match.
/// Returns the candidate whose exact bytes it covered, which is what identifies
/// the statement to claim.
fn verify_candidates<'a>(
    pubkey: &PublicKey,
    signature: &Signature,
    candidates: &'a [Candidate],
    domain: &str,
) -> Result<&'a Candidate, (StatusCode, Json<Value>)> {
    let secp = Secp256k1::new();
    for candidate in candidates {
        if verify_signature_ecdsa(&secp, &candidate.message, signature, pubkey).is_ok() {
            if candidate.legacy {
                record_legacy_verify(candidate.route);
            }
            return Ok(candidate);
        }
    }

    trace!("invalid signature, no candidate message verified for domain '{domain}'");
    Err(invalid_signature_for_domain(domain))
}

/// Identify the statement a signature authorized, for the claim record.
///
/// Hashes what was authorized rather than the signature bytes: ECDSA
/// signatures are malleable, so keying on them would let a reshaped signature
/// re-authorize a statement that was already claimed.
fn statement_hash(pubkey: &PublicKey, signed_message: &str) -> [u8; 32] {
    let mut engine = sha256::Hash::engine();
    engine.input(pubkey.to_string().as_bytes());
    // Separator: the pubkey is fixed-width hex, but the boundary is still worth
    // marking so no pubkey/message split can be read two ways.
    engine.input(b"\x00");
    engine.input(signed_message.as_bytes());
    sha256::Hash::from_engine(engine).to_byte_array()
}

/// Look up what stands between `username` and whoever is asking. See
/// [`LnurlRepository::name_status`] for what `asking_pubkey` changes.
async fn name_status<DB>(
    state: &State<DB>,
    domain: &str,
    username: &str,
    asking_pubkey: Option<&str>,
) -> Result<NameStatus, (StatusCode, Json<Value>)>
where
    DB: LnurlRepository,
{
    state
        .db
        .name_status(domain, username, asking_pubkey)
        .await
        .map_err(|e| {
            error!("failed to execute query: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Value::String("internal server error".into())),
            )
        })
}

/// Claim the statement a verified signature authorized, rejecting a replay.
///
/// A statement is claimable once, whichever route presents it and whichever
/// host the request was addressed to. Claims before the route acts, so a replay
/// cannot slip in alongside the request it copies.
///
/// A client that has to retry re-signs, which produces a fresh timestamp and so
/// a statement of its own. Resending identical bytes is rejected.
async fn claim_statement<DB>(
    state: &State<DB>,
    pubkey: &PublicKey,
    candidate: &Candidate,
) -> Result<(), (StatusCode, Json<Value>)>
where
    DB: LnurlRepository,
{
    let claimed = state
        .db
        .claim_signed_message(
            &statement_hash(pubkey, &candidate.message),
            candidate.route.as_str(),
            candidate.expiry.expires_at(),
        )
        .await
        .map_err(|e| {
            error!("failed to claim signed message: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Value::String("internal server error".into())),
            )
        })?;

    if claimed {
        return Ok(());
    }

    trace!(
        "signature already used, rejecting on '{}'",
        candidate.route.as_str()
    );
    Err((
        StatusCode::CONFLICT,
        Json(Value::String("signature has already been used".into())),
    ))
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
struct SspWebhookPayload {
    #[serde(rename = "type")]
    event_type: String,
    payment_preimage: Option<String>,
    receiver_identity_public_key: Option<String>,
    htlc_amount: Option<SspAmount>,
}

#[derive(Debug, Deserialize)]
struct SspAmount {
    value: i64,
    unit: String,
}

async fn sanitize_domain<DB>(
    state: &State<DB>,
    domain: &str,
) -> Result<String, (StatusCode, Json<Value>)> {
    let domain = domain.trim().to_lowercase();
    let domains = state.domains.read().await;
    if domains.contains_key(&domain) {
        return Ok(domain);
    }
    // An empty allow-list falls open to any host, for local/test setups only.
    // Never on mainnet: there it must be an explicit deny.
    if domains.is_empty() && !state.is_mainnet {
        return Ok(domain);
    }
    warn!("domain not allowed: {}", domain);
    Err((StatusCode::NOT_FOUND, Json(Value::String(String::new()))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::{
        DomainConfig, Invoice, LnurlRepositoryError, LnurlSenderComment, PendingZapReceipt,
    };
    use crate::user::User;
    use crate::webhooks::repository::WebhookRepositoryError;
    use crate::zap::Zap;
    use axum::body::Bytes;
    use axum::http::{HeaderMap, StatusCode};
    use bitcoin::hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256};
    use lnurl_models::ListMetadataMetadata;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tokio::sync::watch;

    // -- Mock repository -------------------------------------------------------

    /// Claimed statement hash to the route that claimed it.
    type ClaimedMessages = std::sync::Arc<Mutex<HashMap<Vec<u8>, String>>>;

    #[derive(Clone, Default)]
    struct MockRepository {
        invoices: std::sync::Arc<Mutex<HashMap<String, Invoice>>>,
        pending_zap_receipts: std::sync::Arc<Mutex<HashMap<String, PendingZapReceipt>>>,
        claimed_messages: ClaimedMessages,
    }

    #[async_trait::async_trait]
    impl LnurlRepository for MockRepository {
        async fn delete_user(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<bool, LnurlRepositoryError> {
            Ok(true)
        }
        async fn claim_signed_message(
            &self,
            statement_hash: &[u8],
            route: &str,
            _: i64,
        ) -> Result<bool, LnurlRepositoryError> {
            let mut claimed = self.claimed_messages.lock().unwrap();
            match claimed.entry(statement_hash.to_vec()) {
                std::collections::hash_map::Entry::Occupied(_) => Ok(false),
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(route.to_string());
                    Ok(true)
                }
            }
        }
        async fn delete_expired_signed_messages(
            &self,
            _: i64,
        ) -> Result<u64, LnurlRepositoryError> {
            Ok(0)
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
        async fn name_status(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> Result<NameStatus, LnurlRepositoryError> {
            Ok(NameStatus::Free)
        }
        async fn transfer_username(
            &self,
            _: TransferRequest<'_>,
            _: StatementClaim<'_>,
        ) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }
        async fn upsert_zap(&self, _: &Zap) -> Result<(), LnurlRepositoryError> {
            Ok(())
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
        async fn list_domains(&self) -> Result<Vec<DomainConfig>, LnurlRepositoryError> {
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
        async fn get_zap_and_invoice_by_payment_hash(
            &self,
            payment_hash: &str,
        ) -> Result<(Option<Zap>, Option<Invoice>), LnurlRepositoryError> {
            Ok((
                None,
                self.invoices.lock().unwrap().get(payment_hash).cloned(),
            ))
        }
        async fn insert_pending_zap_receipt(
            &self,
            pending: &PendingZapReceipt,
        ) -> Result<(), LnurlRepositoryError> {
            self.pending_zap_receipts
                .lock()
                .unwrap()
                .insert(pending.payment_hash.clone(), pending.clone());
            Ok(())
        }
        async fn take_pending_zap_receipts(
            &self,
            _limit: u32,
        ) -> Result<Vec<PendingZapReceipt>, LnurlRepositoryError> {
            Ok(self
                .pending_zap_receipts
                .lock()
                .unwrap()
                .values()
                .cloned()
                .collect())
        }
        async fn update_pending_zap_receipt_retry(
            &self,
            _: &str,
            _: i32,
            _: i64,
        ) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }
        async fn delete_pending_zap_receipt(
            &self,
            payment_hash: &str,
        ) -> Result<(), LnurlRepositoryError> {
            self.pending_zap_receipts
                .lock()
                .unwrap()
                .remove(payment_hash);
            Ok(())
        }
        async fn get_or_create_setting(
            &self,
            _key: &str,
            default_value: &str,
        ) -> Result<String, LnurlRepositoryError> {
            Ok(default_value.to_string())
        }

        async fn set_domain_jwt(
            &self,
            _domain: &str,
            _jwt: &str,
        ) -> Result<(), LnurlRepositoryError> {
            Ok(())
        }

        async fn get_webhook_payloads(
            &self,
            _: &[String],
        ) -> Result<Vec<crate::repository::WebhookPayloadData>, LnurlRepositoryError> {
            Ok(vec![])
        }
    }

    #[async_trait::async_trait]
    impl crate::webhooks::WebhookRepository for MockRepository {
        async fn insert_webhook_deliveries(
            &self,
            _: &[crate::webhooks::NewWebhookDelivery],
        ) -> Result<(), WebhookRepositoryError> {
            Ok(())
        }
        async fn take_pending_webhook_deliveries(
            &self,
        ) -> Result<Vec<crate::webhooks::repository::WebhookDelivery>, WebhookRepositoryError>
        {
            Ok(vec![])
        }
        async fn update_webhook_delivery_success(
            &self,
            _: i64,
            _: i64,
            _: &str,
        ) -> Result<(), WebhookRepositoryError> {
            Ok(())
        }
        async fn update_webhook_delivery_failure(
            &self,
            _: i64,
            _: i32,
            _: i64,
            _: Option<i32>,
            _: Option<&str>,
            _: &str,
        ) -> Result<(), WebhookRepositoryError> {
            Ok(())
        }
        async fn unclaim_webhook_deliveries(
            &self,
            _: &[i64],
        ) -> Result<(), WebhookRepositoryError> {
            Ok(())
        }
        async fn delete_webhook_deliveries_older_than(
            &self,
            _: i64,
        ) -> Result<u64, WebhookRepositoryError> {
            Ok(0)
        }
        async fn delete_webhook_delivery(&self, _: i64) -> Result<(), WebhookRepositoryError> {
            Ok(())
        }
        async fn park_webhook_delivery(&self, _: i64) -> Result<(), WebhookRepositoryError> {
            Ok(())
        }
        async fn list_webhook_configs(
            &self,
        ) -> Result<Vec<crate::webhooks::repository::WebhookConfig>, WebhookRepositoryError>
        {
            Ok(vec![])
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
                domain: None,
                amount_received_sat: None,
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        assert!(
            repo.pending_zap_receipts
                .lock()
                .unwrap()
                .contains_key(&payment_hash)
        );
    }

    #[tokio::test]
    async fn webhook_millisatoshi_htlc_amount_converts_to_sat() {
        let repo = setup_repo_with_invoice(TEST_PREIMAGE_HEX, TEST_RECEIVER_PUBKEY);
        let (trigger, _rx) = watch::channel(());

        let mut payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            Some(TEST_PREIMAGE_HEX),
            Some(TEST_RECEIVER_PUBKEY),
        );
        payload["htlc_amount"] = serde_json::json!({"value": 50_000_000, "unit": "MILLISATOSHI"});
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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
        assert_eq!(invoice.amount_received_sat, Some(50_000));
    }

    #[tokio::test]
    async fn webhook_missing_signature_returns_unauthorized() {
        let repo = MockRepository::default();
        let (trigger, _rx) = watch::channel(());
        let headers = HeaderMap::new();
        let body = Bytes::from(b"{}".to_vec());

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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
                domain: None,
                amount_received_sat: None,
            },
        );
        let (trigger, _rx) = watch::channel(());

        let payload = make_webhook_payload(
            "SPARK_LIGHTNING_RECEIVE_FINISHED",
            Some(TEST_PREIMAGE_HEX),
            Some(TEST_RECEIVER_PUBKEY),
        );
        let (headers, body) = signed_headers_and_body(TEST_WEBHOOK_SECRET, &payload);

        // Process the same webhook twice; it must be idempotent.
        for _ in 0..2 {
            let result = process_webhook(
                &repo,
                &crate::webhooks::WebhookService::new(repo.clone()),
                TEST_WEBHOOK_SECRET,
                &trigger,
                &headers,
                &body,
            )
            .await;
            assert!(result.is_ok());
        }

        // The zap receipt is enqueued so a prior partial failure can recover, but
        // repeated processing does not create duplicate pending entries
        // (ON CONFLICT DO NOTHING), and the publisher drops it if already published.
        assert_eq!(repo.pending_zap_receipts.lock().unwrap().len(), 1);
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
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

        let result = process_webhook(
            &repo,
            &crate::webhooks::WebhookService::new(repo.clone()),
            TEST_WEBHOOK_SECRET,
            &trigger,
            &headers,
            &body,
        )
        .await;
        assert!(result.is_ok());
    }

    // -- Signed messages -------------------------------------------------------
    //
    // Routes verify via SparkWallet::verify_message, which delegates to
    // verify_signature_ecdsa. These exercise the candidate sets and that
    // verification directly, without constructing a wallet.

    use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
    use spark::utils::verify_signature::verify_signature_ecdsa;

    const TEST_DOMAIN: &str = "lnurl.example.com";
    const OTHER_DOMAIN: &str = "other.example.com";
    const TEST_DESCRIPTION: &str = "Pay to alice";

    /// Deterministic keypair from a seed byte.
    fn transfer_key(seed: u8) -> (SecretKey, PublicKey) {
        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(&[seed; 32]).expect("valid secret key");
        let public = PublicKey::from_secret_key(&secp, &secret);
        (secret, public)
    }

    /// Sign `message` the way the SDK does: ECDSA over `sha256(message)`.
    fn sign(secret: &SecretKey, message: &str) -> Signature {
        let secp = Secp256k1::new();
        let digest = sha256::Hash::hash(message.as_bytes());
        secp.sign_ecdsa(&Message::from_digest(digest.to_byte_array()), secret)
    }

    /// The candidate `signature` verifies against, mirroring what
    /// [`verify_candidates`] accepts.
    fn matching<'a>(
        candidates: &'a [Candidate],
        signature: &Signature,
        public_key: &PublicKey,
    ) -> Option<&'a Candidate> {
        let secp = Secp256k1::new();
        candidates.iter().find(|candidate| {
            verify_signature_ecdsa(&secp, &candidate.message, signature, public_key).is_ok()
        })
    }

    fn verifies(candidates: &[Candidate], signature: &Signature, public_key: &PublicKey) -> bool {
        matching(candidates, signature, public_key).is_some()
    }

    // -- Byte-level golden vectors ---------------------------------------------

    /// Pins the exact bytes each route's v2 candidate covers, from the server's
    /// side of the wire: a refactor of the builders that changes them changes
    /// the protocol, and every deployed client with it.
    #[test]
    fn v2_candidate_messages_are_pinned() {
        let (_, alice) = transfer_key(0x11);
        let (_, bob) = transfer_key(0x22);
        let desc_hash = signed_message::description_hash(TEST_DESCRIPTION);

        assert_eq!(
            register_candidates(TEST_DOMAIN, "alice", TEST_DESCRIPTION, TEST_TIMESTAMP)[0].message,
            format!(
                "breez-lnurl:v2\nregister\n{TEST_DOMAIN}\nalice\n{desc_hash}\n{TEST_TIMESTAMP}"
            )
        );
        assert_eq!(
            unregister_candidates(TEST_DOMAIN, "alice", TEST_TIMESTAMP)[0].message,
            format!("breez-lnurl:v2\nunregister\n{TEST_DOMAIN}\nalice\n{TEST_TIMESTAMP}")
        );
        assert_eq!(
            recover_candidates(TEST_DOMAIN, &alice, &alice.to_string(), TEST_TIMESTAMP)[0].message,
            format!("breez-lnurl:v2\nrecover\n{TEST_DOMAIN}\n{alice}\n{TEST_TIMESTAMP}")
        );
        assert_eq!(
            metadata_candidates(TEST_DOMAIN, &alice, &alice.to_string(), TEST_TIMESTAMP)[0].message,
            format!("breez-lnurl:v2\nmetadata\n{TEST_DOMAIN}\n{alice}\n{TEST_TIMESTAMP}")
        );
        assert_eq!(
            transfer_from_candidates(
                TEST_DOMAIN,
                "alice",
                &alice,
                &bob,
                &bob.to_string(),
                Some(TEST_TIMESTAMP)
            )[0]
            .message,
            format!(
                "breez-lnurl:v2\ntransfer-from\n{TEST_DOMAIN}\nalice\n{alice}\n{bob}\n{TEST_TIMESTAMP}"
            )
        );
        assert_eq!(
            transfer_to_candidates(
                TEST_DOMAIN,
                "alice",
                &alice,
                &bob,
                &bob.to_string(),
                TEST_DESCRIPTION,
                Some(TEST_TIMESTAMP)
            )[0]
            .message,
            format!(
                "breez-lnurl:v2\ntransfer-to\n{TEST_DOMAIN}\nalice\n{alice}\n{bob}\n{desc_hash}\n{TEST_TIMESTAMP}"
            )
        );
    }

    /// No two distinct request tuples build the same message, across every
    /// route and both transfer roles.
    #[test]
    fn no_two_request_tuples_build_the_same_message() {
        let (_, alice) = transfer_key(0x11);
        let (_, bob) = transfer_key(0x22);
        let hex_alice = alice.to_string();
        let mut messages: Vec<String> = Vec::new();
        for domain in [TEST_DOMAIN, OTHER_DOMAIN] {
            for name in ["alice", "bob"] {
                messages.extend(
                    register_candidates(domain, name, TEST_DESCRIPTION, TEST_TIMESTAMP)
                        .into_iter()
                        .chain(unregister_candidates(domain, name, TEST_TIMESTAMP))
                        .chain(transfer_from_candidates(
                            domain,
                            name,
                            &alice,
                            &bob,
                            &hex_alice,
                            Some(TEST_TIMESTAMP),
                        ))
                        .chain(transfer_to_candidates(
                            domain,
                            name,
                            &alice,
                            &bob,
                            &hex_alice,
                            TEST_DESCRIPTION,
                            Some(TEST_TIMESTAMP),
                        ))
                        .filter(|candidate| !candidate.legacy)
                        .map(|candidate| candidate.message),
                );
            }
            messages.extend(
                recover_candidates(domain, &alice, &hex_alice, TEST_TIMESTAMP)
                    .into_iter()
                    .chain(metadata_candidates(
                        domain,
                        &alice,
                        &hex_alice,
                        TEST_TIMESTAMP,
                    ))
                    .filter(|candidate| !candidate.legacy)
                    .map(|candidate| candidate.message),
            );
            // Built inline by its route rather than by a candidate builder: it
            // is v2-only, so it has nothing to choose between.
            messages.extend(
                ["alice", "bob"].map(|name| {
                    signed_message::available(domain, &hex_alice, name, TEST_TIMESTAMP)
                }),
            );
        }

        let unique: std::collections::HashSet<&String> = messages.iter().collect();
        assert_eq!(unique.len(), messages.len(), "messages must be distinct");
    }

    // -- Domain binding --------------------------------------------------------

    /// The whole of the fix: a v2 signature is refused once a caller-supplied
    /// `Forwarded` / `X-Forwarded-Host` steers the request at another served
    /// domain. The header still wins domain resolution; the signature is what
    /// refuses the result.
    #[test]
    fn a_v2_signature_does_not_verify_for_another_served_domain() {
        let (secret, public) = transfer_key(0x11);
        let signature = sign(
            &secret,
            &register_candidates(TEST_DOMAIN, "alice", TEST_DESCRIPTION, TEST_TIMESTAMP)[0].message,
        );

        assert!(verifies(
            &register_candidates(TEST_DOMAIN, "alice", TEST_DESCRIPTION, TEST_TIMESTAMP),
            &signature,
            &public
        ));
        assert!(
            !verifies(
                &register_candidates(OTHER_DOMAIN, "alice", TEST_DESCRIPTION, TEST_TIMESTAMP),
                &signature,
                &public
            ),
            "a v2 signature must not verify against a different resolved domain"
        );
    }

    // -- Backward compatibility -----------------------------------------------
    //
    // These drive the real `verify_candidates`, over the exact bytes a pre-v2
    // client signs, so a client that has not migrated keeps working for the
    // whole compatibility window. Every one of them flips to `is_err()` at the
    // cutoff (see the TODOs on the candidate builders).

    /// Serializes the tests that verify a legacy candidate, since doing so
    /// increments the process-wide legacy counters that
    /// `a_legacy_verify_is_counted_and_a_v2_verify_is_not` asserts on.
    fn legacy_counter_guard() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: Mutex<()> = Mutex::new(());
        // A test that fails while holding the guard must not cascade into the
        // others reporting a poisoned mutex instead of their own result.
        GUARD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// What a pre-v2 client signs on each route, paired with the candidate set
    /// the route offers it against.
    ///
    /// The messages are reproduced literally rather than built from the
    /// candidate helpers: a helper that stopped emitting the legacy form would
    /// otherwise silently make the assertions vacuous.
    fn old_client_requests(
        pubkey: &PublicKey,
        to_pubkey: &PublicKey,
    ) -> Vec<(&'static str, String, Vec<Candidate>)> {
        let hex_pubkey = pubkey.to_string();
        let hex_to = to_pubkey.to_string();
        vec![
            (
                "register",
                format!("alice-{TEST_TIMESTAMP}"),
                register_candidates(TEST_DOMAIN, "alice", TEST_DESCRIPTION, TEST_TIMESTAMP),
            ),
            (
                "unregister, prefixed",
                format!("unregister:alice-{TEST_TIMESTAMP}"),
                unregister_candidates(TEST_DOMAIN, "alice", TEST_TIMESTAMP),
            ),
            (
                "unregister, bare",
                format!("alice-{TEST_TIMESTAMP}"),
                unregister_candidates(TEST_DOMAIN, "alice", TEST_TIMESTAMP),
            ),
            (
                "recover",
                format!("{hex_pubkey}-{TEST_TIMESTAMP}"),
                recover_candidates(TEST_DOMAIN, pubkey, &hex_pubkey, TEST_TIMESTAMP),
            ),
            (
                "metadata",
                format!("{hex_pubkey}-{TEST_TIMESTAMP}"),
                metadata_candidates(TEST_DOMAIN, pubkey, &hex_pubkey, TEST_TIMESTAMP),
            ),
            (
                "transfer",
                format!("transfer:alice-{hex_to}"),
                transfer_from_candidates(TEST_DOMAIN, "alice", pubkey, to_pubkey, &hex_to, None),
            ),
        ]
    }

    /// Every route still accepts the signature a pre-v2 client produces.
    #[test]
    fn every_route_still_accepts_a_pre_v2_signature() {
        let _guard = legacy_counter_guard();
        let (secret, public) = transfer_key(0x11);
        let (_, bob) = transfer_key(0x22);

        for (label, message, candidates) in old_client_requests(&public, &bob) {
            let signature = sign(&secret, &message);
            let matched = verify_candidates(&public, &signature, &candidates, TEST_DOMAIN)
                .unwrap_or_else(|_| panic!("{label}: a pre-v2 signature must still verify"));

            assert!(matched.legacy, "{label}: must match the legacy candidate");
            assert_eq!(matched.message, message, "{label}");
        }
    }

    /// The pre-v2 transfer message is signed identically by both parties, so
    /// one signature pair verifies in both slots. The v2 role tags are what end
    /// that, and only for requests that carry a timestamp.
    #[test]
    fn a_pre_v2_transfer_still_verifies_in_both_slots() {
        let _guard = legacy_counter_guard();
        let (alice_secret, alice) = transfer_key(0x11);
        let (bob_secret, bob) = transfer_key(0x22);
        let hex_bob = bob.to_string();
        let from = transfer_from_candidates(TEST_DOMAIN, "alice", &alice, &bob, &hex_bob, None);
        let to = transfer_to_candidates(
            TEST_DOMAIN,
            "alice",
            &alice,
            &bob,
            &hex_bob,
            TEST_DESCRIPTION,
            None,
        );

        assert_eq!(
            from[0].message, to[0].message,
            "both parties signed the same bytes before v2"
        );

        let legacy = format!("transfer:alice-{hex_bob}");
        assert!(
            verify_candidates(&alice, &sign(&alice_secret, &legacy), &from, TEST_DOMAIN).is_ok()
        );
        assert!(verify_candidates(&bob, &sign(&bob_secret, &legacy), &to, TEST_DOMAIN).is_ok());
    }

    /// A v2 signature is preferred over the legacy candidate, so a migrated
    /// client is never recorded as legacy traffic and never claims the bare
    /// statement a pre-v2 client would.
    #[test]
    fn the_v2_candidate_is_matched_before_the_legacy_one() {
        let (secret, public) = transfer_key(0x11);
        let candidates =
            register_candidates(TEST_DOMAIN, "alice", TEST_DESCRIPTION, TEST_TIMESTAMP);
        let signature = sign(&secret, &candidates[0].message);

        let matched = verify_candidates(&public, &signature, &candidates, TEST_DOMAIN)
            .expect("a v2 signature must verify");

        assert!(!matched.legacy);
        assert_eq!(matched.expiry, ClaimExpiry::Bounded(TEST_TIMESTAMP));
    }

    /// The counters are the gate on dropping the legacy candidates, so a legacy
    /// verify that failed to increment one would make the tail look empty while
    /// clients still depend on it.
    #[test]
    fn a_legacy_verify_is_counted_and_a_v2_verify_is_not() {
        let _guard = legacy_counter_guard();
        let (secret, public) = transfer_key(0x33);
        let candidates = unregister_candidates(TEST_DOMAIN, "alice", TEST_TIMESTAMP);
        drain_legacy_verifies();

        verify_candidates(
            &public,
            &sign(&secret, &candidates[0].message),
            &candidates,
            TEST_DOMAIN,
        )
        .expect("v2 verifies");
        assert!(
            drain_legacy_verifies().iter().all(|(_, count)| *count == 0),
            "a v2 verify must not be counted as legacy"
        );

        verify_candidates(
            &public,
            &sign(&secret, &format!("unregister:alice-{TEST_TIMESTAMP}")),
            &candidates,
            TEST_DOMAIN,
        )
        .expect("legacy verifies");
        let counts = drain_legacy_verifies();
        assert_eq!(
            counts
                .iter()
                .find(|(route, _)| *route == "unregister")
                .map(|(_, count)| *count),
            Some(1),
            "the legacy verify must be counted against its own route"
        );
        assert!(
            counts
                .iter()
                .filter(|(route, _)| *route != "unregister")
                .all(|(_, count)| *count == 0),
            "and against no other route"
        );

        // Draining resets, so each interval reports only its own traffic.
        assert!(drain_legacy_verifies().iter().all(|(_, count)| *count == 0));
    }

    /// A legacy signature stays domain-free for the whole compatibility window,
    /// so it can still be aimed elsewhere by header. Flip at the cutoff.
    #[test]
    fn a_legacy_signature_still_verifies_for_any_served_domain() {
        let (secret, public) = transfer_key(0x11);
        let signature = sign(&secret, &format!("alice-{TEST_TIMESTAMP}"));

        for domain in [TEST_DOMAIN, OTHER_DOMAIN] {
            assert!(verifies(
                &register_candidates(domain, "alice", TEST_DESCRIPTION, TEST_TIMESTAMP),
                &signature,
                &public
            ));
        }
    }

    // -- Description binding ---------------------------------------------------

    #[test]
    fn a_tampered_description_rejects_the_register_signature() {
        let (secret, public) = transfer_key(0x11);
        let signature = sign(
            &secret,
            &register_candidates(TEST_DOMAIN, "alice", TEST_DESCRIPTION, TEST_TIMESTAMP)[0].message,
        );

        assert!(
            !verifies(
                &register_candidates(TEST_DOMAIN, "alice", "Pay to mallory", TEST_TIMESTAMP)
                    .into_iter()
                    .filter(|candidate| !candidate.legacy)
                    .collect::<Vec<_>>(),
                &signature,
                &public
            ),
            "the description is covered by the signature"
        );
    }

    #[test]
    fn a_tampered_description_rejects_the_transferee_signature() {
        let (secret, bob) = transfer_key(0x22);
        let (_, alice) = transfer_key(0x11);
        let to_candidates = |description| {
            transfer_to_candidates(
                TEST_DOMAIN,
                "alice",
                &alice,
                &bob,
                &bob.to_string(),
                description,
                Some(TEST_TIMESTAMP),
            )
        };
        let signature = sign(&secret, &to_candidates(TEST_DESCRIPTION)[0].message);

        assert!(verifies(&to_candidates(TEST_DESCRIPTION), &signature, &bob));
        assert!(!verifies(
            &to_candidates("Pay to mallory"),
            &signature,
            &bob
        ));
    }

    // -- Role separation -------------------------------------------------------

    /// The role tags are what keep the current owner's signature from serving
    /// as the transferee's, and the other way round.
    #[test]
    fn the_two_transfer_roles_sign_different_bytes() {
        let (alice_secret, alice) = transfer_key(0x11);
        let (_, bob) = transfer_key(0x22);
        let hex_bob = bob.to_string();
        let from = transfer_from_candidates(
            TEST_DOMAIN,
            "alice",
            &alice,
            &bob,
            &hex_bob,
            Some(TEST_TIMESTAMP),
        );
        let to = transfer_to_candidates(
            TEST_DOMAIN,
            "alice",
            &alice,
            &bob,
            &hex_bob,
            TEST_DESCRIPTION,
            Some(TEST_TIMESTAMP),
        );
        let signature = sign(&alice_secret, &from[0].message);

        assert!(verifies(&from, &signature, &alice));
        assert!(
            !verifies(&to, &signature, &alice),
            "the current owner's signature must not fill the transferee's slot"
        );
    }

    /// A transfer authorization names the direction, so the reversed pair is a
    /// different transfer rather than the same one.
    #[test]
    fn a_transfer_authorization_names_its_direction() {
        let (alice_secret, alice) = transfer_key(0x11);
        let (_, bob) = transfer_key(0x22);
        let forward = transfer_from_candidates(
            TEST_DOMAIN,
            "alice",
            &alice,
            &bob,
            &bob.to_string(),
            Some(TEST_TIMESTAMP),
        );
        let reversed = transfer_from_candidates(
            TEST_DOMAIN,
            "alice",
            &bob,
            &alice,
            &alice.to_string(),
            Some(TEST_TIMESTAMP),
        );
        let signature = sign(&alice_secret, &forward[0].message);

        assert!(!verifies(&reversed, &signature, &alice));
    }

    /// A recover signature is not a metadata credential, and the reverse. Until
    /// the cutoff the shared legacy message keeps both interchangeable, which is
    /// what the second half asserts; flip it then.
    #[test]
    fn recover_and_metadata_sign_different_bytes() {
        let (secret, public) = transfer_key(0x11);
        let hex = public.to_string();
        let recover = recover_candidates(TEST_DOMAIN, &public, &hex, TEST_TIMESTAMP);
        let metadata = metadata_candidates(TEST_DOMAIN, &public, &hex, TEST_TIMESTAMP);
        let v2_only = |candidates: Vec<Candidate>| {
            candidates
                .into_iter()
                .filter(|candidate| !candidate.legacy)
                .collect::<Vec<_>>()
        };

        let signature = sign(&secret, &recover[0].message);
        assert!(!verifies(&v2_only(metadata), &signature, &public));

        let legacy = sign(&secret, &format!("{hex}-{TEST_TIMESTAMP}"));
        assert!(
            verifies(&recover, &legacy, &public)
                && verifies(
                    &metadata_candidates(TEST_DOMAIN, &public, &hex, TEST_TIMESTAMP),
                    &legacy,
                    &public
                ),
            "the shared legacy message keeps the two interchangeable until the cutoff"
        );
    }

    // -- Transfer timestamp ----------------------------------------------------

    /// Stripping `timestamp` from a v2 transfer request must fail rather than
    /// fall back to the legacy path: the absent timestamp selects the legacy
    /// candidate alone, and the v2 signature does not verify against it.
    #[test]
    fn stripping_the_transfer_timestamp_does_not_downgrade() {
        let (alice_secret, alice) = transfer_key(0x11);
        let (_, bob) = transfer_key(0x22);
        let hex_bob = bob.to_string();
        let signed = transfer_from_candidates(
            TEST_DOMAIN,
            "alice",
            &alice,
            &bob,
            &hex_bob,
            Some(TEST_TIMESTAMP),
        );
        let signature = sign(&alice_secret, &signed[0].message);

        let stripped = transfer_from_candidates(TEST_DOMAIN, "alice", &alice, &bob, &hex_bob, None);
        assert!(
            stripped.iter().all(|candidate| candidate.legacy),
            "an absent timestamp offers only the legacy message"
        );
        assert!(!verifies(&stripped, &signature, &alice));
    }

    #[test]
    fn a_v2_transfer_claim_is_bounded_and_a_legacy_one_is_not() {
        let (_, alice) = transfer_key(0x11);
        let (_, bob) = transfer_key(0x22);
        let hex_bob = bob.to_string();

        assert_eq!(
            transfer_from_candidates(
                TEST_DOMAIN,
                "alice",
                &alice,
                &bob,
                &hex_bob,
                Some(TEST_TIMESTAMP)
            )[0]
            .expiry,
            ClaimExpiry::Bounded(TEST_TIMESTAMP)
        );
        assert_eq!(
            transfer_from_candidates(TEST_DOMAIN, "alice", &alice, &bob, &hex_bob, None)[0].expiry,
            ClaimExpiry::Unbounded,
            "nothing bounds the legacy message in time, so its claim is never pruned"
        );
    }

    #[test]
    fn a_bounded_claim_outlives_the_timestamp_it_covers() {
        // Pruning the claim any earlier would put the statement back in play
        // while `timestamp_is_fresh` still accepts it.
        assert!(
            ClaimExpiry::Bounded(TEST_TIMESTAMP).expires_at()
                >= i64::try_from(TEST_TIMESTAMP + ACCEPTABLE_TIME_DIFF_SECS).unwrap()
        );
        assert_eq!(ClaimExpiry::Unbounded.expires_at(), i64::MAX);
    }

    /// Bounded symmetrically, so a device whose clock runs fast still works.
    /// An asymmetric future bound would cap a stolen authorization slightly
    /// tighter at the cost of a route that fails where every other one works.
    #[test]
    fn the_accept_window_is_symmetric_around_now() {
        // Fixed `now`, so a clock tick between the bound and the check cannot
        // move the boundary by a second.
        let now = TEST_TIMESTAMP;

        assert!(is_fresh_at(now, now));
        assert!(is_fresh_at(now - ACCEPTABLE_TIME_DIFF_SECS, now));
        assert!(is_fresh_at(now + ACCEPTABLE_TIME_DIFF_SECS, now));
        assert!(!is_fresh_at(now - ACCEPTABLE_TIME_DIFF_SECS - 1, now));
        assert!(!is_fresh_at(now + ACCEPTABLE_TIME_DIFF_SECS + 1, now));
    }

    #[test]
    fn a_transfer_outside_the_window_is_told_apart_from_a_bad_signature() {
        assert_ne!(transfer_expired().1.0, invalid_timestamp().1.0);
        assert_ne!(
            invalid_signature_for_domain(TEST_DOMAIN).1.0,
            invalid_signature_for_domain(OTHER_DOMAIN).1.0
        );
    }

    // -- Metadata credential source --------------------------------------------

    fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                axum::http::HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn the_metadata_signature_header_wins_over_the_query_param() {
        let headers = header_map(&[
            (METADATA_SIGNATURE_HEADER, "beef"),
            (METADATA_TIMESTAMP_HEADER, "1752000000"),
        ]);

        assert_eq!(
            metadata_credentials(&headers, Some("dead".into()), Some(1)).unwrap(),
            ("beef".to_string(), 1_752_000_000)
        );
    }

    #[test]
    fn the_metadata_query_params_are_the_fallback() {
        assert_eq!(
            metadata_credentials(&HeaderMap::new(), Some("dead".into()), Some(7)).unwrap(),
            ("dead".to_string(), 7)
        );
    }

    #[test]
    fn a_malformed_metadata_header_is_refused_rather_than_falling_through() {
        let headers = header_map(&[(METADATA_TIMESTAMP_HEADER, "not-a-number")]);
        let err = metadata_credentials(&headers, Some("dead".into()), Some(7))
            .expect_err("a header that fails to parse must not fall back to the query param");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_metadata_request_with_no_credential_at_all_is_refused() {
        assert_eq!(
            metadata_credentials(&HeaderMap::new(), None, None)
                .expect_err("neither source present")
                .0,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn amount_bounds_accepts_within_range() {
        for amount in [1_000, 2_500_000, 4_000_000_000] {
            assert!(
                validate_amount_bounds(amount, 1_000, 4_000_000_000).is_ok(),
                "{amount} msat is within bounds and must be accepted"
            );
        }
    }

    #[test]
    fn amount_bounds_rejects_out_of_range() {
        // Below min (including zero) and above max must both be rejected.
        for amount in [0, 999, 4_000_000_001] {
            let err = validate_amount_bounds(amount, 1_000, 4_000_000_000)
                .expect_err("out-of-bounds amount must be rejected");
            assert_eq!(err.0, StatusCode::OK, "LNURL errors use HTTP 200");
            assert_eq!(err.1.0["reason"], "amount out of bounds");
        }
    }

    // -- unregister signature domain separation --------------------------------

    const TEST_TIMESTAMP: u64 = 1_752_000_000;

    /// Whether `signature` authorizes unregistering `username`, mirroring what
    /// the route accepts.
    fn authorizes_unregister(
        public_key: &PublicKey,
        signature: &Signature,
        username: &str,
        timestamp: u64,
    ) -> bool {
        verifies(
            &unregister_candidates(TEST_DOMAIN, username, timestamp),
            signature,
            public_key,
        )
    }

    fn registered_as(name: &str) -> User {
        User {
            domain: "example.com".to_string(),
            pubkey: "02abc123".to_string(),
            name: name.to_string(),
            description: String::new(),
        }
    }

    #[test]
    fn unregister_accepts_the_v2_and_both_legacy_messages() {
        let messages: Vec<String> = unregister_candidates(TEST_DOMAIN, "alice", TEST_TIMESTAMP)
            .into_iter()
            .map(|candidate| candidate.message)
            .collect();

        assert_eq!(
            messages,
            vec![
                format!("breez-lnurl:v2\nunregister\n{TEST_DOMAIN}\nalice\n{TEST_TIMESTAMP}"),
                format!("unregister:alice-{TEST_TIMESTAMP}"),
                format!("alice-{TEST_TIMESTAMP}"),
            ]
        );
    }

    #[test]
    fn unregister_signature_authorizes_unregister() {
        let (secret_key, public_key) = transfer_key(0x11);
        let signature = sign(&secret_key, &format!("unregister:alice-{TEST_TIMESTAMP}"));

        assert!(authorizes_unregister(
            &public_key,
            &signature,
            "alice",
            TEST_TIMESTAMP
        ));
    }

    #[test]
    fn register_signature_still_verifies_against_the_legacy_message() {
        let (secret_key, public_key) = transfer_key(0x11);
        let signature = sign(&secret_key, &format!("alice-{TEST_TIMESTAMP}"));

        // Register's format keeps verifying while clients migrate. What stops it
        // deleting the address is the claim register takes on the same
        // statement, covered by
        // `register_and_legacy_unregister_claim_one_statement`. Flip this to
        // assert!(!...) when the legacy candidate is removed.
        assert!(authorizes_unregister(
            &public_key,
            &signature,
            "alice",
            TEST_TIMESTAMP
        ));
    }

    #[test]
    fn register_and_legacy_unregister_claim_one_statement() {
        let (_, public_key) = transfer_key(0x11);

        // Register signs "{username}-{timestamp}" and the legacy unregister
        // candidate covers those exact bytes, so both routes claim one
        // statement and the second to present it is refused.
        let registered = statement_hash(&public_key, &format!("alice-{TEST_TIMESTAMP}"));
        let mut candidates = unregister_candidates(TEST_DOMAIN, "alice", TEST_TIMESTAMP)
            .into_iter()
            .map(|candidate| statement_hash(&public_key, &candidate.message));

        assert!(candidates.any(|hash| hash == registered));
    }

    /// The v2 messages carry distinct route tags, so the two no longer share a
    /// statement once both sides speak v2.
    #[test]
    fn v2_register_and_unregister_claim_different_statements() {
        let (_, public_key) = transfer_key(0x11);

        assert_ne!(
            statement_hash(
                &public_key,
                &register_candidates(TEST_DOMAIN, "alice", TEST_DESCRIPTION, TEST_TIMESTAMP)[0]
                    .message
            ),
            statement_hash(
                &public_key,
                &unregister_candidates(TEST_DOMAIN, "alice", TEST_TIMESTAMP)[0].message
            )
        );
    }

    #[test]
    fn prefixed_unregister_claims_a_different_statement_than_register() {
        let (_, public_key) = transfer_key(0x11);

        assert_ne!(
            statement_hash(&public_key, &format!("alice-{TEST_TIMESTAMP}")),
            statement_hash(&public_key, &format!("unregister:alice-{TEST_TIMESTAMP}"))
        );
    }

    #[test]
    fn a_statement_is_bound_to_the_pubkey_that_signed_it() {
        let (_, first) = transfer_key(0x11);
        let (_, second) = transfer_key(0x22);
        let signed = format!("alice-{TEST_TIMESTAMP}");

        assert_ne!(
            statement_hash(&first, &signed),
            statement_hash(&second, &signed)
        );
    }

    #[test]
    fn a_pubkey_signature_does_not_delete_the_registered_address() {
        // recover and list_metadata sign "{pubkey}-{timestamp}", and a metadata
        // signature travels in the request URL. That signature does verify against
        // the legacy candidate, so the name it covers is what stops the deletion.
        let (secret_key, public_key) = transfer_key(0x11);
        let pubkey_hex = public_key.to_string();
        let signature = sign(&secret_key, &format!("{pubkey_hex}-{TEST_TIMESTAMP}"));

        assert!(
            authorizes_unregister(&public_key, &signature, &pubkey_hex, TEST_TIMESTAMP),
            "the signature itself still verifies while the legacy form is accepted"
        );
        assert_eq!(
            unregister_action(&pubkey_hex, Some(&registered_as("alice"))),
            UnregisterAction::NameMismatch
        );
    }

    #[test]
    fn deleting_requires_the_signed_name_to_be_the_registered_one() {
        assert_eq!(
            unregister_action("alice", Some(&registered_as("alice"))),
            UnregisterAction::Delete
        );
        assert_eq!(
            unregister_action("bob", Some(&registered_as("alice"))),
            UnregisterAction::NameMismatch
        );
    }

    #[test]
    fn unregistering_an_address_that_is_already_gone_succeeds() {
        // Nothing to remove, so the request's goal already holds. Reporting
        // success is what lets a client holding a stale name clear it.
        assert_eq!(
            unregister_action("alice", None),
            UnregisterAction::AlreadyGone
        );
    }

    #[test]
    fn a_username_cannot_produce_the_unregister_prefix() {
        // The prefix only separates because ':' is outside the username charset.
        assert!(validate_username("unregister:alice").is_err());
    }

    // -- Zap request validation ------------------------------------------------

    fn zap_request(tags: Vec<nostr::Tag>, keys: &nostr::Keys) -> Event {
        nostr::EventBuilder::new(nostr::Kind::ZapRequest, "")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    /// The tags every zap request needs to get past the earlier rules.
    fn base_tags() -> Vec<nostr::Tag> {
        vec![
            nostr::Tag::public_key(nostr::Keys::generate().public_key),
            nostr::Tag::from_standardized_without_cell(TagStandard::Relays(vec![
                "wss://relay.example".parse().unwrap(),
            ])),
        ]
    }

    fn uppercase_p_tag(pubkey: nostr::PublicKey) -> nostr::Tag {
        nostr::Tag::from_standardized_without_cell(TagStandard::PublicKey {
            public_key: pubkey,
            relay_url: None,
            alias: None,
            uppercase: true,
        })
    }

    /// The key the request would be receipted with, as a sender reads it from
    /// the LNURL-pay response.
    fn receipt_signer() -> (nostr::Keys, XOnlyPublicKey) {
        let keys = nostr::Keys::generate();
        let xonly = keys.public_key.xonly().unwrap();
        (keys, xonly)
    }

    #[test]
    fn zap_request_without_a_p_tag_is_accepted() {
        let (_, signer) = receipt_signer();
        let event = zap_request(base_tags(), &nostr::Keys::generate());
        assert!(validate_nostr_zap_request(1000, &event, signer).is_ok());
    }

    #[test]
    fn zap_request_with_a_stale_created_at_is_accepted() {
        // NIP-57 sets no timestamp tolerance, and a sender's clock is not ours
        // to police; a request is held to one receipt instead.
        let (_, signer) = receipt_signer();
        let event = nostr::EventBuilder::new(nostr::Kind::ZapRequest, "")
            .tags(base_tags())
            .custom_created_at(nostr::Timestamp::from(1_600_000_000))
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap();
        assert!(validate_nostr_zap_request(1000, &event, signer).is_ok());
    }

    /// Rule 8: a `P` tag pins the key the sender expects to sign the receipt,
    /// so it has to be this user's, not the sender's own.
    #[test]
    fn zap_request_p_tag_must_be_the_receipt_signer() {
        let (signer_keys, signer) = receipt_signer();
        let sender = nostr::Keys::generate();

        let mut tags = base_tags();
        tags.push(uppercase_p_tag(signer_keys.public_key));
        assert!(validate_nostr_zap_request(1000, &zap_request(tags, &sender), signer).is_ok());

        let mut tags = base_tags();
        tags.push(uppercase_p_tag(sender.public_key));
        assert!(
            validate_nostr_zap_request(1000, &zap_request(tags, &sender), signer).is_err(),
            "a 'P' tag pinning any key but the receipt signer must be rejected"
        );
    }

    #[test]
    fn zap_request_with_multiple_p_tags_is_rejected() {
        let (signer_keys, signer) = receipt_signer();
        let sender = nostr::Keys::generate();
        let mut tags = base_tags();
        tags.push(uppercase_p_tag(signer_keys.public_key));
        tags.push(uppercase_p_tag(signer_keys.public_key));
        assert!(validate_nostr_zap_request(1000, &zap_request(tags, &sender), signer).is_err());
    }

    // -- Advertised nostr pubkey ----------------------------------------------

    /// The key an address advertises must be the one its receipts are signed
    /// with, which is the whole of NIP-57 Appendix F. Signing here mirrors
    /// `publish_zap_receipt`, so advertising the server key, or deriving from
    /// the wrong field, fails this.
    #[test]
    fn an_address_advertises_the_key_its_receipts_are_signed_with() {
        let server = nostr::Keys::generate();
        let owner = "02a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
        let other_owner = "03f0e1d2c3b4a5968778695a4b3c2d1e0f9a8b7c6d5e4f30211203f4e5d6c7b8a9";

        let request = zap_request(base_tags(), &nostr::Keys::generate());
        let receipt =
            nostr::EventBuilder::zap_receipt("lnbc1", Some("preimage".to_string()), &request)
                .sign_with_keys(&crate::zap::derive_user_nostr_keys(&server, owner).unwrap())
                .unwrap();

        let advertised = user_nostr_pubkey(Some(&server), owner).unwrap();
        assert_eq!(
            advertised,
            Some(receipt.pubkey.xonly().unwrap()),
            "the advertised key does not verify this address's own receipts"
        );
        assert_ne!(
            user_nostr_pubkey(Some(&server), other_owner).unwrap(),
            advertised,
            "two owners advertising one key makes their receipts interchangeable"
        );
    }

    #[test]
    fn no_nostr_key_advertises_no_zap_support() {
        assert_eq!(user_nostr_pubkey(None, "02ab").unwrap(), None);
    }

    // -- lnurl tag ------------------------------------------------------------

    #[test]
    fn a_logged_tag_is_bounded() {
        let real = "lnurl1dp68gurn8ghj7cmpddjjucmpwd5z7tnhv4kxctttdehhwm30d3h82unvwqhkyetyw35k6etkd93x2uch9e86k";
        assert_eq!(for_log(real), real, "a real lnurl must not be truncated");

        let huge = "a".repeat(32_000);
        let logged = for_log(&huge);
        assert!(
            logged.chars().count() < 300,
            "logged {} chars",
            logged.chars().count()
        );
        assert!(logged.ends_with("... (truncated)"));
    }

    #[test]
    fn lnurl_tag_decodes_every_shape_senders_emit() {
        // bech32, the only form NIP-57 describes, in both cases.
        let bech32_lower = "lnurl1dp68gurn8ghj7cmpddjjucmpwd5z7tnhv4kxctttdehhwm30d3h82unvwqhkyetyw35k6etkd93x2uch9e86k";
        assert_eq!(
            decode_lnurl_tag(bech32_lower),
            decode_lnurl_tag(&bech32_lower.to_uppercase()),
            "bech32 is case insensitive, so both spellings must resolve alike"
        );
        assert!(
            decode_lnurl_tag(bech32_lower).is_some_and(|decoded| decoded.starts_with("https://")),
            "expected an http(s) url, got {:?}",
            decode_lnurl_tag(bech32_lower)
        );

        // A bare lightning address, which senders emit despite the spec.
        assert_eq!(
            decode_lnurl_tag("alice@example.com").as_deref(),
            Some("https://example.com/.well-known/lnurlp/alice")
        );

        // A plain url, likewise.
        assert_eq!(
            decode_lnurl_tag("https://example.com/.well-known/lnurlp/alice").as_deref(),
            Some("https://example.com/.well-known/lnurlp/alice")
        );

        assert_eq!(decode_lnurl_tag("not an lnurl"), None);
    }

    #[test]
    fn lnurl_tag_matches_only_the_address_being_paid() {
        for lnurl in [
            "https://example.com/.well-known/lnurlp/alice",
            "https://example.com/lnurlp/alice",
            "https://example.com/lnurlp/alice/",
            "lnurlp://example.com/lnurlp/alice",
            "https://EXAMPLE.com/lnurlp/Alice",
        ] {
            assert!(
                lnurl_targets_address(lnurl, "example.com", "alice"),
                "{lnurl} should address alice@example.com"
            );
        }

        for lnurl in [
            "https://example.com/.well-known/lnurlp/bob",
            "https://other.com/.well-known/lnurlp/alice",
            "https://example.com/lnurlp/alice/extra",
        ] {
            assert!(
                !lnurl_targets_address(lnurl, "example.com", "alice"),
                "{lnurl} should not address alice@example.com"
            );
        }
    }
}
