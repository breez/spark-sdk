use std::marker::PhantomData;

use axum::{
    Extension, Json,
    extract::{Path, Query},
    http::StatusCode,
};
use diesel::{
    RunQueryDsl, SqliteConnection,
    r2d2::{ConnectionManager, Pool},
};
use secp256k1::{PublicKey, schnorr::Signature};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, error};

use crate::{
    models::{User, users},
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
    pub signature: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterLnurlPayRequest {
    pub username: String,
    pub signature: String,
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
        let pubkey = hex::decode(&pubkey).map_err(|e| {
            debug!("failed to decode pubkey: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(Value::String("invalid pubkey".into())),
            )
        })?;
        let pubkey = PublicKey::from_slice(&pubkey).map_err(|e| {
            debug!("failed to create public key: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(Value::String("invalid pubkey".into())),
            )
        })?;
        let (x_only_pubkey, _) = pubkey.x_only_public_key();

        let signature = hex::decode(&payload.signature).map_err(|e| {
            debug!("failed to decode signature: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(Value::String("invalid signature".into())),
            )
        })?;
        let signature = signature.try_into().map_err(|e| {
            debug!("failed to convert signature: {:?}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(Value::String("invalid signature".into())),
            )
        })?;
        let signature = Signature::from_byte_array(signature);

        let secp = secp256k1::Secp256k1::verification_only();
        secp.verify_schnorr(&signature, payload.username.as_bytes(), &x_only_pubkey)
            .map_err(|e| {
                debug!("failed to verify signature: {}", e);
                (
                    StatusCode::BAD_REQUEST,
                    Json(Value::String("invalid signature".into())),
                )
            })?;

        let user = User {
            pubkey: pubkey.to_string(),
            name: payload.username.clone(),
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
            let diesel::result::Error::DatabaseError(
                database_error_kind,
                database_error_information,
            ) = &e
            else {
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
        let _pubkey = pubkey;
        let _payload = payload;
        let _state = state;
        todo!()
    }

    pub async fn recover(
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<SqlitePool>>,
        Json(payload): Json<RecoverLnurlPayRequest>,
    ) -> Result<Json<RegisterLnurlPayResponse>, (StatusCode, Json<Value>)> {
        let _pubkey = pubkey;
        let _payload = payload;
        let _state = state;
        todo!()
    }

    pub async fn handle_lnurl_pay(
        Path(identifier): Path<String>,
        Query(params): Query<LnurlPayParams>,
        Extension(state): Extension<State<SqlitePool>>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        let _identifier = identifier;
        let _params = params;
        let _state = state;
        todo!()
    }

    pub async fn handle_invoice(
        Path(name): Path<String>,
        Query(params): Query<LnurlPayCallbackParams>,
        Extension(state): Extension<State<SqlitePool>>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        let _name = name;
        let _params = params;
        let _state = state;
        todo!()
    }
}
