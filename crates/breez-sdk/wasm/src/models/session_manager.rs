use std::str::FromStr;

use bitcoin::secp256k1::PublicKey;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, js_sys::Promise};

use crate::models::{Session, error::js_error_to_session_manager_error};

pub struct WasmSessionManager {
    pub session_manager: SessionManager,
}

// Single-threaded WASM environment makes this safe.
unsafe impl Send for WasmSessionManager {}
unsafe impl Sync for WasmSessionManager {}

#[macros::async_trait]
impl breez_sdk_spark::SessionManager for WasmSessionManager {
    async fn get_session(
        &self,
        service_identity_key: PublicKey,
    ) -> Result<breez_sdk_spark::Session, breez_sdk_spark::SessionManagerError> {
        let pk_hex = service_identity_key.to_string();
        let promise = self
            .session_manager
            .get_session(pk_hex)
            .map_err(js_error_to_session_manager_error)?;
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_session_manager_error)?;
        let session: Session = serde_wasm_bindgen::from_value(result).map_err(|e| {
            breez_sdk_spark::SessionManagerError::Generic(format!(
                "Failed to deserialize session: {e}"
            ))
        })?;
        Ok(session.into())
    }

    async fn set_session(
        &self,
        service_identity_key: PublicKey,
        session: breez_sdk_spark::Session,
    ) -> Result<(), breez_sdk_spark::SessionManagerError> {
        let pk_hex = service_identity_key.to_string();
        let promise = self
            .session_manager
            .set_session(pk_hex, session.into())
            .map_err(js_error_to_session_manager_error)?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_session_manager_error)?;
        Ok(())
    }
}

// Hex-encoded PublicKey is exposed to JS as a `string`. The conversion lives
// here so callers passing a `PublicKey` from Rust through WASM are accepted
// transparently.
#[allow(dead_code)]
fn parse_pubkey(s: &str) -> Result<PublicKey, breez_sdk_spark::SessionManagerError> {
    PublicKey::from_str(s).map_err(|e| {
        breez_sdk_spark::SessionManagerError::Generic(format!("Invalid public key: {e}"))
    })
}

#[wasm_bindgen(typescript_custom_section)]
const SESSION_MANAGER_INTERFACE: &'static str = r#"export interface SessionManager {
    getSession: (serviceIdentityKey: string) => Promise<Session>;
    setSession: (serviceIdentityKey: string, session: Session) => Promise<void>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "SessionManager")]
    pub type SessionManager;

    #[wasm_bindgen(structural, method, js_name = getSession, catch)]
    pub fn get_session(
        this: &SessionManager,
        service_identity_key: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = setSession, catch)]
    pub fn set_session(
        this: &SessionManager,
        service_identity_key: String,
        session: Session,
    ) -> Result<Promise, JsValue>;
}
