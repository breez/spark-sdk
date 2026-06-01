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
use crate::session_store::{Session, SessionStore, SessionStoreError};
use crate::signer::SparkSigner;

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

    async fn get_or_authenticate(&self) -> Result<Session> {
        let cached = self
            .session_store
            .get_session(&self.identity_public_key)
            .await;
        match cached {
            Ok(session) if session.is_valid() => return Ok(session),
            Ok(_) => debug!("Operator session expired, authenticating"),
            Err(SessionStoreError::NotFound) => {
                debug!("Operator session not found, authenticating")
            }
            Err(SessionStoreError::Generic(e)) => {
                error!("Failed to get operator session from session store: {}", e)
            }
        }
        let session = self.authenticate().await?;
        self.session_store
            .set_session(&self.identity_public_key, session.clone())
            .await?;
        Ok(session)
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
