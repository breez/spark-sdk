use std::sync::Arc;

use super::OperatorRpcError;
use super::error::Result;
use super::spark_authn::{
    GetChallengeRequest, VerifyChallengeRequest,
    spark_authn_service_client::SparkAuthnServiceClient,
};
use crate::operator::OperatorSession;
use crate::operator::rpc::transport::grpc_client::Transport;
use crate::signer::Signer;
use prost::Message;
use tonic::Request;
use web_time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct OperatorAuth {
    transport: Transport,
    signer: Arc<dyn Signer>,
}

impl OperatorAuth {
    pub fn new(transport: Transport, signer: Arc<dyn Signer>) -> Self {
        Self { transport, signer }
    }

    pub async fn get_authenticated_session(
        &self,
        session: Option<OperatorSession>,
    ) -> Result<OperatorSession> {
        // Check if the session is still valid
        if let Some(session) = session
            && session.expiration
                > SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| {
                        OperatorRpcError::Unexpected("UNIX_EPOCH is in the future".to_string())
                    })?
                    .as_secs()
        {
            return Ok(session);
        }

        let session = self.authenticate().await?;
        Ok(session)
    }

    async fn authenticate(&self) -> Result<OperatorSession> {
        let pk = self.signer.get_identity_public_key()?;
        let challenge_req = GetChallengeRequest {
            public_key: pk.serialize().to_vec(),
        };

        let mut auth_client = SparkAuthnServiceClient::new(self.transport.clone());

        // get the challenge from Spark Authn Service
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

        // sign the challenge
        let challenge =
            protected_challenge
                .challenge
                .clone()
                .ok_or(OperatorRpcError::Authentication(
                    "Invalid challenge".to_string(),
                ))?;

        // Serialize the challenge to match what the server uses
        let challenge_bytes = challenge.encode_to_vec(); // This is the same as proto.Marshal in Go.

        let signature = self
            .signer
            .sign_message_ecdsa_with_identity_key(&challenge_bytes)?;

        let verify_req = VerifyChallengeRequest {
            protected_challenge: Some(protected_challenge),
            signature: signature.serialize_der().to_vec(),
            public_key: pk.serialize().to_vec(),
        };

        let verify_resp = auth_client
            .verify_challenge(Request::new(verify_req))
            .await?
            .into_inner();

        let session = OperatorSession {
            token: verify_resp.session_token.parse().map_err(|_| {
                OperatorRpcError::Authentication("Invalid session token".to_string())
            })?,
            expiration: verify_resp.expiration_timestamp.try_into().map_err(|_| {
                OperatorRpcError::Authentication("Invalid expiration timestamp".to_string())
            })?,
        };
        Ok(session)
    }
}
