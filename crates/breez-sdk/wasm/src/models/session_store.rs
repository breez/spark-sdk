use std::str::FromStr;

use bitcoin::secp256k1::PublicKey;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, js_sys::Promise};

use crate::models::{Session, error::js_error_to_session_store_error};

pub struct WasmSessionStore {
    pub session_store: SessionStore,
}

// Single-threaded WASM environment makes this safe.
unsafe impl Send for WasmSessionStore {}
unsafe impl Sync for WasmSessionStore {}

#[macros::async_trait]
impl breez_sdk_spark::SessionStore for WasmSessionStore {
    async fn get_session(
        &self,
        service_identity_key: PublicKey,
    ) -> Result<breez_sdk_spark::Session, breez_sdk_spark::SessionStoreError> {
        let pk_hex = service_identity_key.to_string();
        let promise = self
            .session_store
            .get_session(pk_hex)
            .map_err(js_error_to_session_store_error)?;
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_session_store_error)?;
        let session: Session = serde_wasm_bindgen::from_value(result).map_err(|e| {
            breez_sdk_spark::SessionStoreError::Generic(format!(
                "Failed to deserialize session: {e}"
            ))
        })?;
        Ok(session.into())
    }

    async fn set_session(
        &self,
        service_identity_key: PublicKey,
        session: breez_sdk_spark::Session,
    ) -> Result<(), breez_sdk_spark::SessionStoreError> {
        let pk_hex = service_identity_key.to_string();
        let promise = self
            .session_store
            .set_session(pk_hex, session.into())
            .map_err(js_error_to_session_store_error)?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_session_store_error)?;
        Ok(())
    }
}

// Hex-encoded PublicKey is exposed to JS as a `string`. The conversion lives
// here so callers passing a `PublicKey` from Rust through WASM are accepted
// transparently.
#[allow(dead_code)]
fn parse_pubkey(s: &str) -> Result<PublicKey, breez_sdk_spark::SessionStoreError> {
    PublicKey::from_str(s).map_err(|e| {
        breez_sdk_spark::SessionStoreError::Generic(format!("Invalid public key: {e}"))
    })
}

#[wasm_bindgen(typescript_custom_section)]
const SESSION_STORE_INTERFACE: &'static str = r#"export interface SessionStore {
    getSession: (serviceIdentityKey: string) => Promise<Session>;
    setSession: (serviceIdentityKey: string, session: Session) => Promise<void>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "SessionStore")]
    pub type SessionStore;

    #[wasm_bindgen(structural, method, js_name = getSession, catch)]
    pub fn get_session(
        this: &SessionStore,
        service_identity_key: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = setSession, catch)]
    pub fn set_session(
        this: &SessionStore,
        service_identity_key: String,
        session: Session,
    ) -> Result<Promise, JsValue>;
}
