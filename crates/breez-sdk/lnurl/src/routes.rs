use std::marker::PhantomData;

use axum::{
    Extension, Json,
    extract::{Path, Query},
    http::StatusCode,
};
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{PublicKey, ecdsa::Signature};
use lnurl::{Tag, pay::PayResponse};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, error, trace};

use crate::{
    repository::{LnurlRepository, LnurlRepositoryError},
    state::State,
    user::{USERNAME_VALIDATION_REGEX, User},
};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LnurlPayParams {}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LnurlPayCallbackParams {
    pub amount: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecoverLnurlPayRequest {
    pub signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecoverLnurlPayResponse {
    pub lnurl: String,
    pub lightning_address: String,
    pub username: String,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterLnurlPayRequest {
    pub username: String,
    pub signature: String,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterLnurlPayResponse {
    pub lnurl: String,
    pub lightning_address: String,
}

pub struct LnurlServer<DB> {
    db: PhantomData<DB>,
}

impl<DB> LnurlServer<DB>
where
    DB: LnurlRepository,
{
    pub async fn register(
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<RegisterLnurlPayRequest>,
    ) -> Result<Json<RegisterLnurlPayResponse>, (StatusCode, Json<Value>)> {
        let pubkey = validate(&pubkey, &payload.signature, &payload.username, &state).await?;
        if payload.description.chars().take(256).count() > 255 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(Value::String("description too long".into())),
            ));
        }
        let user = User {
            pubkey: pubkey.to_string(),
            name: payload.username,
            description: payload.description,
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
        Ok(Json(RegisterLnurlPayResponse {
            lnurl: format!("{}://{}/lnurlp/{}", state.scheme, state.domain, user.name),
            lightning_address: format!("{}@{}", user.name, state.domain),
        }))
    }

    pub async fn unregister(
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<RegisterLnurlPayRequest>,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        let pubkey = validate(&pubkey, &payload.signature, &payload.username, &state).await?;

        state
            .db
            .delete_user(&pubkey.to_string())
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
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<RecoverLnurlPayRequest>,
    ) -> Result<Json<RecoverLnurlPayResponse>, (StatusCode, Json<Value>)> {
        let pubkey = validate(&pubkey, &payload.signature, &pubkey, &state).await?;

        let user = state
            .db
            .get_user_by_pubkey(&pubkey.to_string())
            .await
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?;

        match user {
            Some(user) => Ok(Json(RecoverLnurlPayResponse {
                lnurl: format!("{}://{}/lnurlp/{}", state.scheme, state.domain, user.name),
                lightning_address: format!("{}@{}", user.name, state.domain),
                username: user.name,
                description: user.description,
            })),
            None => Err((
                StatusCode::NOT_FOUND,
                Json(Value::String("user not found".into())),
            )),
        }
    }

    pub async fn handle_lnurl_pay(
        Path(identifier): Path<String>,
        Extension(state): Extension<State<DB>>,
    ) -> Result<Json<PayResponse>, (StatusCode, Json<Value>)> {
        if identifier.is_empty() {
            return Err((StatusCode::NOT_FOUND, Json(Value::String(String::new()))));
        }

        let user = state.db.get_user_by_name(&identifier).await.map_err(|e| {
            error!("failed to execute query: {}", e);
            lnurl_error("internal server error")
        })?;

        let Some(user) = user else {
            return Err((StatusCode::NOT_FOUND, Json(Value::String(String::new()))));
        };

        Ok(Json(PayResponse {
            callback: format!(
                "{}://{}/lnurlp/{}/invoice",
                state.scheme, state.domain, user.name
            ),
            max_sendable: state.max_sendable,
            min_sendable: state.min_sendable,
            tag: Tag::PayRequest,
            metadata: get_metadata(&state.domain, &user),
            comment_allowed: None,
            allows_nostr: None,
            nostr_pubkey: None,
        }))
    }

    pub async fn handle_invoice(
        Path(identifier): Path<String>,
        Query(params): Query<LnurlPayCallbackParams>,
        Extension(state): Extension<State<DB>>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        if identifier.is_empty() {
            return Err((StatusCode::NOT_FOUND, Json(Value::String(String::new()))));
        }

        let user = state.db.get_user_by_name(&identifier).await.map_err(|e| {
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

        let metadata = get_metadata(&state.domain, &user);
        let desc_hash = sha256::Hash::hash(metadata.as_bytes());
        let pubkey = parse_pubkey(&user.pubkey)?;
        let invoice = state
            .wallet
            .create_lightning_invoice(
                amount_msat / 1000,
                Some(spark_wallet::InvoiceDescription::DescriptionHash(
                    desc_hash.to_byte_array(),
                )),
                Some(pubkey),
            )
            .await
            .map_err(|e| {
                error!("failed to create lightning invoice: {}", e);
                lnurl_error("failed to create invoice")
            })?;

        debug!("Created lightning invoice: {:?}", invoice);

        // TODO: Save things like the invoice/preimage/transfer id?
        // TODO: Validate invoice?
        // TODO: Add lnurl-verify

        Ok(Json(json!({
            "pr": invoice.invoice,
            "routes": Vec::<String>::new(),
        })))
    }
}

async fn validate<DB>(
    pubkey: &str,
    signature: &str,
    username: &str,
    state: &State<DB>,
) -> Result<PublicKey, (StatusCode, Json<Value>)> {
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

    state
        .wallet
        .verify_message(username, &signature, &pubkey)
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
