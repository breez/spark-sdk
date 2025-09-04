use std::marker::PhantomData;

use axum::{
    Extension, Json,
    extract::{Path, Query},
    http::StatusCode,
};
use diesel::{Connection, RunQueryDsl};
use secp256k1::{schnorr::Signature, PublicKey, VerifyOnly};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use crate::{models::{users, User}, state::State};

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

impl<DB, B> LnurlServer<DB>
where DB: Connection<Backend = B>,
      B: diesel::backend::Backend {
    pub async fn register(
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<RegisterLnurlPayRequest>,
    ) -> Result<Json<RegisterLnurlPayResponse>, (StatusCode, Json<Value>)> {
        let pubkey = hex::decode(&pubkey).map_err(|e| {
            debug!("failed to decode pubkey: {}", e);
            (StatusCode::BAD_REQUEST, Json(Value::String("invalid pubkey".into())))
        })?;
        let pubkey = PublicKey::from_slice(&pubkey).map_err(|e| {
            debug!("failed to create public key: {}", e);
            (StatusCode::BAD_REQUEST, Json(Value::String("invalid pubkey".into())))
        })?;
        let (x_only_pubkey, _) = pubkey.x_only_public_key();

        let signature = hex::decode(&payload.signature).map_err(|e| {
            debug!("failed to decode signature: {}", e);
            (StatusCode::BAD_REQUEST, Json(Value::String("invalid signature".into())))
        })?;
        let signature = signature.try_into().map_err(|e| {
            debug!("failed to convert signature: {:?}", e);
            (StatusCode::BAD_REQUEST, Json(Value::String("invalid signature".into())))
        })?;
        let signature = Signature::from_byte_array(signature);

        let secp = secp256k1::Secp256k1::verification_only();
        secp.verify_schnorr(&signature, payload.username.as_bytes(), &x_only_pubkey).map_err(|e| {
            debug!("failed to verify signature: {}", e);
            (StatusCode::BAD_REQUEST, Json(Value::String("invalid signature".into())))
        })?;

        let mut db = *state.db.as_ref();
        let user = User {
            pubkey: pubkey.to_string(),
            name: payload.username.clone(),
        };
        let err = diesel::insert_into(users::table)
            .values(user)
            .execute(&mut db)
            .map_err(|e| e.into())?;

        todo!()
    }

    pub async fn unregister(
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
        Json(payload): Json<RegisterLnurlPayRequest>,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        let _pubkey = pubkey;
        let _payload = payload;
        let _state = state;
        todo!()
    }

    pub async fn recover(
        Path(pubkey): Path<String>,
        Extension(state): Extension<State<DB>>,
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
        Extension(state): Extension<State<DB>>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        let _identifier = identifier;
        let _params = params;
        let _state = state;
        todo!()
    }

    pub async fn handle_invoice(
        Path(name): Path<String>,
        Query(params): Query<LnurlPayCallbackParams>,
        Extension(state): Extension<State<DB>>,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        let _name = name;
        let _params = params;
        let _state = state;
        todo!()
    }
}
