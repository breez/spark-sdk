use std::collections::HashMap;
use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use prost::Message;
use tonic::Request;
use tracing::{debug, error};

use super::OperatorRpcError;
use super::error::Result;
use super::spark_authn::{
    GetChallengeRequest, VerifyChallengeRequest,
    spark_authn_service_client::SparkAuthnServiceClient,
};
use crate::header_provider::{HeaderProvider, HeaderProviderError};
use crate::operator::rpc::transport::grpc_client::Transport;
use crate::session_manager::{Session, SessionManager, SessionManagerError};
use crate::signer::Signer;

#[derive(Clone)]
pub struct SoAuthHeaderProvider {
    transport: Transport,
    signer: Arc<dyn Signer>,
    session_manager: Arc<dyn SessionManager>,
    identity_public_key: PublicKey,
}

impl SoAuthHeaderProvider {
    pub fn new(
        transport: Transport,
        signer: Arc<dyn Signer>,
        session_manager: Arc<dyn SessionManager>,
        identity_public_key: PublicKey,
    ) -> Self {
        Self {
            transport,
            signer,
            session_manager,
            identity_public_key,
        }
    }

    async fn get_or_authenticate(&self) -> Result<Session> {
        let cached = self
            .session_manager
            .get_session(&self.identity_public_key)
            .await;
        let candidate = match cached {
            Ok(session) => Some(session),
            Err(SessionManagerError::NotFound) => {
                debug!("Operator session not found, authenticating");
                None
            }
            Err(SessionManagerError::Generic(e)) => {
                error!("Failed to get operator session from session manager: {}", e);
                None
            }
        };
        let session = match candidate {
            Some(s) if s.is_valid() => s,
            _ => self.authenticate().await?,
        };
        self.session_manager
            .set_session(&self.identity_public_key, session.clone())
            .await?;
        Ok(session)
    }

    async fn authenticate(&self) -> Result<Session> {
        let pk = self.signer.get_identity_public_key().await?;
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
            .signer
            .sign_message_ecdsa_with_identity_key(&challenge_bytes)
            .await?;

        let verify_req = VerifyChallengeRequest {
            protected_challenge: Some(protected_challenge),
            signature: signature.serialize_der().to_vec(),
            public_key: pk.serialize().to_vec(),
        };

        let verify_resp = auth_client
            .verify_challenge(Request::new(verify_req))
            .await?
            .into_inner();

        Ok(Session {
            token: verify_resp.session_token.parse().map_err(|_| {
                OperatorRpcError::Authentication("Invalid session token".to_string())
            })?,
            expiration: verify_resp.expiration_timestamp.try_into().map_err(|_| {
                OperatorRpcError::Authentication("Invalid expiration timestamp".to_string())
            })?,
        })
    }
}

#[macros::async_trait]
impl HeaderProvider for SoAuthHeaderProvider {
    async fn headers(&self) -> std::result::Result<HashMap<String, String>, HeaderProviderError> {
        let session = self
            .get_or_authenticate()
            .await
            .map_err(|e| HeaderProviderError::Generic(e.to_string()))?;
        Ok(HashMap::from([(
            "authorization".to_string(),
            session.token,
        )]))
    }
}
