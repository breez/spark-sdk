use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use platform_utils::tokio;
use prost::Message;
use tonic::{Code, Request};
use tracing::{debug, error, warn};

use super::OperatorRpcError;
use super::error::Result;
use super::spark_authn::{
    GetChallengeRequest, RefreshSessionRequest, RefreshSessionResponse, VerifyChallengeRequest,
    spark_authn_service_client::SparkAuthnServiceClient,
};
use crate::header_provider::{HeaderProvider, HeaderProviderError};
use crate::operator::rpc::transport::grpc_client::Transport;
use crate::session_store::{Session, SessionStore, SessionStoreError};
use crate::signer::SparkSigner;

const REFRESH_ATTEMPTS: usize = 3;
const REFRESH_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct SoAuthHeaderProvider {
    transport: Transport,
    spark_signer: Arc<dyn SparkSigner>,
    session_store: Arc<dyn SessionStore>,
    identity_public_key: PublicKey,
}

impl SoAuthHeaderProvider {
    pub fn new(
        transport: Transport,
        spark_signer: Arc<dyn SparkSigner>,
        session_store: Arc<dyn SessionStore>,
        identity_public_key: PublicKey,
    ) -> Self {
        Self {
            transport,
            spark_signer,
            session_store,
            identity_public_key,
        }
    }

    async fn get_or_authenticate(&self, force_refresh: bool) -> Result<Session> {
        if force_refresh {
            self.session_store
                .invalidate(&self.identity_public_key)
                .await;
        }
        let stored = self.stored_session().await;
        if !force_refresh
            && let Some(session) = &stored
            && session.has_valid_token()
        {
            return Ok(session.clone());
        }
        let refreshed = if let Some(session) = &stored
            && session.has_valid_refresh_token()
            && let Some(refresh_token) = &session.refresh_token
        {
            self.refresh(refresh_token).await
        } else {
            None
        };
        let session = match refreshed {
            Some(session) => session,
            None => self.authenticate().await?,
        };
        self.session_store
            .set_session(&self.identity_public_key, session.clone())
            .await?;
        Ok(session)
    }

    async fn stored_session(&self) -> Option<Session> {
        match self
            .session_store
            .get_session(&self.identity_public_key)
            .await
        {
            Ok(session) => Some(session),
            Err(SessionStoreError::NotFound) => {
                debug!("Operator session not found");
                None
            }
            Err(SessionStoreError::Generic(e)) => {
                error!("Failed to get operator session from session store: {e}");
                None
            }
        }
    }

    async fn refresh(&self, refresh_token: &str) -> Option<Session> {
        let mut auth_client = SparkAuthnServiceClient::new(self.transport.clone());
        let request = RefreshSessionRequest {
            refresh_token: refresh_token.to_string(),
            refresh_attempt_id: refresh_attempt_id(refresh_token),
        };
        for attempt in 1..=REFRESH_ATTEMPTS {
            match auth_client
                .refresh_session(Request::new(request.clone()))
                .await
            {
                Ok(response) => {
                    return session_from_refresh(response.into_inner())
                        .inspect_err(|e| warn!("Operator returned an invalid session: {e}"))
                        .ok();
                }
                Err(status) if is_transient(status.code()) && attempt < REFRESH_ATTEMPTS => {
                    debug!(
                        "Operator session refresh failed with {:?}, retrying",
                        status.code()
                    );
                    tokio::time::sleep(REFRESH_RETRY_DELAY).await;
                }
                Err(status) if status.code() == Code::Unauthenticated => {
                    warn!("Operator rejected the refresh token, signing a challenge");
                    return None;
                }
                Err(status) => {
                    debug!(
                        "Operator session refresh failed with {:?}, signing a challenge",
                        status.code()
                    );
                    return None;
                }
            }
        }
        None
    }

    async fn authenticate(&self) -> Result<Session> {
        let pk = self.spark_signer.get_identity_public_key().await?;
        let challenge_req = GetChallengeRequest {
            public_key: pk.serialize().to_vec(),
        };

        let mut auth_client = SparkAuthnServiceClient::new(self.transport.clone());

        let spark_authn_response = auth_client
            .get_challenge(Request::new(challenge_req))
            .await?
            .into_inner();

        let protected_challenge =
            spark_authn_response
                .protected_challenge
                .ok_or(OperatorRpcError::Authentication(
                    "Missing challenge".to_string(),
                ))?;

        let challenge =
            protected_challenge
                .challenge
                .clone()
                .ok_or(OperatorRpcError::Authentication(
                    "Invalid challenge".to_string(),
                ))?;

        let challenge_bytes = challenge.encode_to_vec();

        let signature = self
            .spark_signer
            .sign_authentication_challenge(&challenge_bytes)
            .await?;

        let verify_req = VerifyChallengeRequest {
            protected_challenge: Some(protected_challenge),
            signature: signature.serialize_der().to_vec(),
            public_key: pk.serialize().to_vec(),
            request_refresh_token: true,
        };

        let verify_resp = auth_client
            .verify_challenge(Request::new(verify_req))
            .await?
            .into_inner();

        let (refresh_token, refresh_token_expiration) = refresh_credential(
            verify_resp.refresh_token,
            verify_resp.refresh_token_expiration,
        );
        Ok(Session {
            token: verify_resp.session_token,
            expiration: verify_resp.expiration_timestamp.try_into().map_err(|_| {
                OperatorRpcError::Authentication("Invalid expiration timestamp".to_string())
            })?,
            refresh_token,
            refresh_token_expiration,
        })
    }
}

impl SoAuthHeaderProvider {
    async fn auth_headers(
        &self,
        force_refresh: bool,
    ) -> std::result::Result<HashMap<String, String>, HeaderProviderError> {
        let session = self.get_or_authenticate(force_refresh).await.map_err(|e| {
            if e.is_unavailable() {
                HeaderProviderError::Unavailable(e.to_string())
            } else {
                HeaderProviderError::Generic(e.to_string())
            }
        })?;
        Ok(HashMap::from([(
            "authorization".to_string(),
            session.token,
        )]))
    }
}

#[macros::async_trait]
impl HeaderProvider for SoAuthHeaderProvider {
    async fn headers(&self) -> std::result::Result<HashMap<String, String>, HeaderProviderError> {
        self.auth_headers(false).await
    }

    async fn headers_refresh(
        &self,
    ) -> std::result::Result<HashMap<String, String>, HeaderProviderError> {
        self.auth_headers(true).await
    }
}

fn refresh_attempt_id(refresh_token: &str) -> String {
    let hash = sha256::Hash::hash(refresh_token.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash.as_byte_array()[..16]);
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string()
}

fn is_transient(code: Code) -> bool {
    matches!(code, Code::Unavailable | Code::Unknown | Code::Internal)
}

fn refresh_credential(
    refresh_token: String,
    expiration: Option<prost_types::Timestamp>,
) -> (Option<String>, Option<u64>) {
    match expiration.and_then(|t| u64::try_from(t.seconds).ok()) {
        Some(expiration) if !refresh_token.is_empty() => (Some(refresh_token), Some(expiration)),
        _ => (None, None),
    }
}

fn session_from_refresh(response: RefreshSessionResponse) -> Result<Session> {
    let expiration = response
        .expiration
        .and_then(|t| u64::try_from(t.seconds).ok())
        .ok_or_else(|| {
            OperatorRpcError::Authentication("Invalid session expiration".to_string())
        })?;
    let (refresh_token, refresh_token_expiration) =
        refresh_credential(response.refresh_token, response.refresh_token_expiration);
    Ok(Session {
        token: response.session_token,
        expiration,
        refresh_token,
        refresh_token_expiration,
    })
}

#[cfg(test)]
mod tests {
    use macros::test_all;

    use super::*;

    fn timestamp(seconds: i64) -> prost_types::Timestamp {
        prost_types::Timestamp { seconds, nanos: 0 }
    }

    #[test_all]
    fn attempt_id_is_a_uuid_derived_from_the_token() {
        let id = refresh_attempt_id("token-a");
        assert_eq!(id, refresh_attempt_id("token-a"));
        assert_ne!(id, refresh_attempt_id("token-b"));
        assert!(uuid::Uuid::parse_str(&id).is_ok());
    }

    #[test_all]
    fn refresh_token_with_expiration_is_a_credential() {
        assert_eq!(
            refresh_credential("refresh".to_string(), Some(timestamp(100))),
            (Some("refresh".to_string()), Some(100))
        );
    }

    #[test_all]
    fn empty_refresh_token_is_no_credential() {
        assert_eq!(
            refresh_credential(String::new(), Some(timestamp(100))),
            (None, None)
        );
    }

    #[test_all]
    fn refresh_token_without_expiration_is_no_credential() {
        assert_eq!(
            refresh_credential("refresh".to_string(), None),
            (None, None)
        );
    }

    #[test_all]
    fn negative_expiration_is_no_credential() {
        assert_eq!(
            refresh_credential("refresh".to_string(), Some(timestamp(-1))),
            (None, None)
        );
    }

    #[test_all]
    fn refreshed_session_carries_rotated_token() {
        let session = session_from_refresh(RefreshSessionResponse {
            session_token: "session".to_string(),
            expiration: Some(timestamp(200)),
            refresh_token: "rotated".to_string(),
            refresh_token_expiration: Some(timestamp(300)),
        })
        .unwrap();
        assert_eq!(session.token, "session");
        assert_eq!(session.expiration, 200);
        assert_eq!(session.refresh_token.as_deref(), Some("rotated"));
        assert_eq!(session.refresh_token_expiration, Some(300));
    }

    #[test_all]
    fn refreshed_session_without_expiration_is_rejected() {
        assert!(
            session_from_refresh(RefreshSessionResponse {
                session_token: "session".to_string(),
                expiration: None,
                refresh_token: "rotated".to_string(),
                refresh_token_expiration: Some(timestamp(300)),
            })
            .is_err()
        );
    }

    #[test_all]
    fn only_transient_codes_are_retried() {
        assert!(is_transient(tonic::Code::Unavailable));
        assert!(is_transient(tonic::Code::Unknown));
        assert!(is_transient(tonic::Code::Internal));
        assert!(!is_transient(tonic::Code::Unauthenticated));
        assert!(!is_transient(tonic::Code::Unimplemented));
        assert!(!is_transient(tonic::Code::DeadlineExceeded));
    }
}
