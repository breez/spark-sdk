use std::sync::Arc;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bitcoin::secp256k1::PublicKey;
use breez_sdk_common::utils::now;
use platform_utils::tokio::sync::RwLock;
use serde::Deserialize;
use spark_wallet::{Session, SessionManager, SessionManagerError};

pub(crate) const KEY_BREEZ_JWT: &str = "breez_jwt";
const PARTNER_ID_HEADER: &str = "partner_id";
const JWT_EXPIRY_GRACE_PERIOD_SECS: u64 = 60 * 5; // Token expires 5 minutes in advance

pub(crate) struct BreezSessionManager {
    inner: Arc<dyn SessionManager>,
    token: RwLock<Option<String>>,
}

impl BreezSessionManager {
    pub(crate) fn new(inner: Arc<dyn SessionManager>) -> Self {
        Self {
            inner,
            token: RwLock::new(None),
        }
    }

    pub(crate) async fn get_token(&self) -> Option<String> {
        self.token.read().await.clone()
    }

    pub(crate) async fn set_token(&self, new_token: String) {
        *self.token.write().await = Some(new_token);
    }
}

#[macros::async_trait]
impl SessionManager for BreezSessionManager {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionManagerError> {
        let mut session = self.inner.get_session(service_identity_key).await?;
        if let Some(token) = self.token.read().await.as_ref()
            && session.headers.get(PARTNER_ID_HEADER) != Some(token)
        {
            session
                .headers
                .insert(PARTNER_ID_HEADER.to_string(), token.clone());
        }
        Ok(session)
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionManagerError> {
        self.inner.set_session(service_identity_key, session).await
    }
}

#[derive(Deserialize)]
struct Jwt {
    exp: u64,
}

pub(crate) fn calculate_expiry(exp: u64) -> u64 {
    exp.saturating_sub(Into::<u64>::into(now()).saturating_add(JWT_EXPIRY_GRACE_PERIOD_SECS))
}

pub(crate) fn is_jwt_expired(token: &str) -> bool {
    let Some(exp) = jwt_exp(token) else {
        return true;
    };
    calculate_expiry(exp) == 0
}

pub(crate) fn jwt_exp(token: &str) -> Option<u64> {
    let payload_b64 = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let payload = std::str::from_utf8(&decoded).ok()?;
    let Jwt { exp } = serde_json::from_str(payload).ok()?;
    Some(exp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jwt(exp: u64) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#));
        format!("{header}.{payload}.fakesignature")
    }

    // --- jwt_exp ---

    #[test]
    fn test_jwt_exp_extracts_value() {
        assert_eq!(jwt_exp(&make_jwt(9_999_999_999)), Some(9_999_999_999));
    }

    #[test]
    fn test_jwt_exp_missing_exp_field() {
        let payload = URL_SAFE_NO_PAD.encode(r#"{"sub":"user123"}"#);
        assert_eq!(jwt_exp(&format!("h.{payload}.s")), None);
    }

    #[test]
    fn test_jwt_exp_invalid_json() {
        let payload = URL_SAFE_NO_PAD.encode("not json");
        assert_eq!(jwt_exp(&format!("h.{payload}.s")), None);
    }

    // --- is_jwt_expired ---

    #[test]
    fn test_is_jwt_expired_far_past() {
        assert!(is_jwt_expired(&make_jwt(0)));
    }

    #[test]
    fn test_is_jwt_expired_far_future() {
        assert!(!is_jwt_expired(&make_jwt(u64::MAX / 2)));
    }

    #[test]
    fn test_is_jwt_expired_within_grace_period() {
        // Will expire in 2 minutes, which is within the 3-minute grace window.
        // Marked as expired
        let token = make_jwt(u64::from(now()) + 120);
        assert!(is_jwt_expired(&token));
    }

    #[test]
    fn test_is_jwt_expired_malformed_token() {
        assert!(is_jwt_expired("not.a.jwt"));
        assert!(is_jwt_expired("onlyone"));
    }
}
