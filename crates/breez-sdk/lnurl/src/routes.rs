use std::marker::PhantomData;

use axum::{
    Extension, Json,
    extract::{Path, Query},
    http::StatusCode,
};
use bitcoin::secp256k1::{PublicKey, schnorr::Signature};
use bitcoin::{
    hashes::{Hash, sha256},
    key::Secp256k1,
    secp256k1::Message,
};
use diesel::{
    ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl, SqliteConnection,
    r2d2::{ConnectionManager, Pool},
};
use lnurl::{Tag, pay::PayResponse};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, error};

use crate::{
    models::{USERNAME_VALIDATION_REGEX, User, users},
    state::State,
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

// Currently using SQLite, but designed to be extensible for other backends like PostgreSQL
// For Postgres support, you would:
// 1. Add the 'pg' feature to Diesel in Cargo.toml
// 2. Create a similar impl for PgPool
// 3. Or create a generic implementation using traits to support both backends
type SqlitePool = Pool<ConnectionManager<SqliteConnection>>;

impl LnurlServer<SqlitePool> {
    pub async fn register(
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<SqlitePool>>,
        Json(payload): Json<RegisterLnurlPayRequest>,
    ) -> Result<Json<RegisterLnurlPayResponse>, (StatusCode, Json<Value>)> {
        let pubkey = validate(&pubkey, &payload.signature, &payload.username)?;
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

        let mut conn = state.db.get().map_err(|e| {
            error!("failed to get database connection: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Value::String("internal server error".into())),
            )
        })?;

        if let Err(e) = diesel::insert_into(users::table)
            .values(&user)
            .on_conflict(users::pubkey)
            .do_update()
            .set(&user)
            .execute(&mut *conn)
        {
            let diesel::result::Error::DatabaseError(database_error_kind, _) = &e else {
                error!("failed to execute query: {}", e);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                ));
            };

            let diesel::result::DatabaseErrorKind::UniqueViolation = database_error_kind else {
                error!("failed to execute query: {}", e);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                ));
            };

            debug!("name already exists: {}", user.name);
            return Err((
                StatusCode::CONFLICT,
                Json(Value::String("name already taken".into())),
            ));
        }

        Ok(Json(RegisterLnurlPayResponse {
            lnurl: format!("https://{}/lnurlp/{}", state.domain, user.name),
            lightning_address: format!("{}@{}", user.name, state.domain),
        }))
    }

    pub async fn unregister(
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<SqlitePool>>,
        Json(payload): Json<RegisterLnurlPayRequest>,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        let pubkey = validate(&pubkey, &payload.signature, &payload.username)?;
        let mut conn = state.db.get().map_err(|e| {
            error!("failed to get database connection: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Value::String("internal server error".into())),
            )
        })?;
        diesel::delete(users::table)
            .filter(users::pubkey.eq(pubkey.to_string()))
            .execute(&mut *conn)
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?;
        Ok(())
    }

    pub async fn recover(
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<SqlitePool>>,
        Json(payload): Json<RecoverLnurlPayRequest>,
    ) -> Result<Json<RecoverLnurlPayResponse>, (StatusCode, Json<Value>)> {
        let pubkey = validate(&pubkey, &payload.signature, &pubkey)?;
        let mut conn = state.db.get().map_err(|e| {
            error!("failed to get database connection: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Value::String("internal server error".into())),
            )
        })?;
        let user = users::table
            .filter(users::pubkey.eq(pubkey.to_string()))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(Value::String("internal server error".into())),
                )
            })?;

        match user {
            Some(user) => Ok(Json(RecoverLnurlPayResponse {
                lnurl: format!("https://{}/lnurlp/{}", state.domain, user.name),
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
        Extension(state): Extension<State<SqlitePool>>,
    ) -> Result<Json<PayResponse>, (StatusCode, Json<Value>)> {
        if identifier.is_empty() {
            return Err((StatusCode::NOT_FOUND, Json(Value::String("".into()))));
        }

        let mut conn = state.db.get().map_err(|e| {
            error!("failed to get database connection: {}", e);
            lnurl_error("internal server error")
        })?;
        let user = users::table
            .filter(users::name.eq(identifier))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                lnurl_error("internal server error")
            })?;
        let Some(user) = user else {
            return Err((StatusCode::NOT_FOUND, Json(Value::String("".into()))));
        };

        Ok(Json(PayResponse {
            callback: format!("https://{}/lnurlp/{}/invoice", state.domain, user.name),
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
        Extension(state): Extension<State<SqlitePool>>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        if identifier.is_empty() {
            return Err((StatusCode::NOT_FOUND, Json(Value::String("".into()))));
        }

        let mut conn = state.db.get().map_err(|e| {
            error!("failed to get database connection: {}", e);
            lnurl_error("internal server error")
        })?;
        let user = users::table
            .filter(users::name.eq(identifier))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|e| {
                error!("failed to execute query: {}", e);
                lnurl_error("internal server error")
            })?;
        let Some(user) = user else {
            return Err((StatusCode::NOT_FOUND, Json(Value::String("".into()))));
        };

        let Some(amount_msat) = params.amount else {
            debug!("missing amount");
            return Err(lnurl_error("missing amount"));
        };

        if amount_msat % 1000 != 0 {
            debug!("invalid amount");
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

        // TODO: Save things like the invoice/preimage/transfer id?
        // TODO: Validate invoice?

        // TODO: Add lnurl-verify
        Ok(Json(json!({
            "pr": invoice.invoice,
            "routes": Vec::<String>::new(),
        })))
    }
}

fn validate(
    pubkey: &str,
    signature: &str,
    username: &str,
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
        return Err((
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid username".into())),
        ));
    }

    let pubkey = parse_pubkey(pubkey)?;
    let (x_only_pubkey, _) = pubkey.x_only_public_key();

    let signature = hex::decode(signature).map_err(|e| {
        debug!("failed to decode signature: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid signature".into())),
        )
    })?;
    let signature = Signature::from_slice(&signature).map_err(|e| {
        debug!("failed to parse signature: {:?}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid signature".into())),
        )
    })?;

    let secp = Secp256k1::verification_only();
    secp.verify_schnorr(
        &signature,
        &Message::from_digest(sha256::Hash::hash(username.as_bytes()).to_byte_array()),
        &x_only_pubkey,
    )
    .map_err(|e| {
        debug!("failed to verify signature: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid signature".into())),
        )
    })?;

    Ok(pubkey)
}

fn parse_pubkey(pubkey: &str) -> Result<PublicKey, (StatusCode, Json<Value>)> {
    let pubkey = hex::decode(pubkey).map_err(|e| {
        debug!("failed to decode pubkey: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid pubkey".into())),
        )
    })?;
    let pubkey = PublicKey::from_slice(&pubkey).map_err(|e| {
        debug!("failed to parse public key: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(Value::String("invalid pubkey".into())),
        )
    })?;
    Ok(pubkey)
}

fn get_metadata(domain: &str, user: &User) -> String {
    Value::Array(vec![
        Value::Array(vec![
            Value::String("text/plain".to_string()),
            Value::String(user.description.clone()),
        ]),
        Value::Array(vec![
            Value::String("text/identifier".to_string()),
            Value::String(format!("{}@{}", user.name, domain)),
        ]),
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
