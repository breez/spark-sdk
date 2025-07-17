use std::sync::Arc;

use super::OperatorRpcError;
use super::error::Result;
use super::spark::spark_service_client::SparkServiceClient;
use super::spark_authn::{
    GetChallengeRequest, VerifyChallengeRequest,
    spark_authn_service_client::SparkAuthnServiceClient,
};
use crate::signer::Signer;
use prost::Message;
use tokio::sync::Mutex;
use tonic::Request;
use tonic::Status;
use tonic::metadata::Ascii;
use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

#[derive(Clone, Debug)]
pub struct OperatorAuth<S> {
    channel: Channel,
    signer: Arc<S>,
    session: Arc<Mutex<Option<OperationSession>>>,
}

#[derive(Clone, Debug)]
pub struct OperationSession {
    token: MetadataValue<Ascii>,
    expiration: u64,
}

impl<S> OperatorAuth<S>
where
    S: Signer,
{
    pub fn new(channel: Channel, signer: Arc<S>) -> Self {
        Self {
            channel,
            signer,
            session: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn spark_service_client(
        &self,
    ) -> Result<SparkServiceClient<InterceptedService<Channel, OperationSession>>> {
        let session = self.get_authenticated_session().await?;
        Ok(SparkServiceClient::with_interceptor(
            self.channel.clone(),
            session,
        ))
    }

    pub async fn get_authenticated_session(&self) -> Result<OperationSession> {
        if let Some(session) = self.session.lock().await.as_ref() {
            // Check if the session is still valid
            if session.expiration > tokio::time::Instant::now().elapsed().as_secs() {
                return Ok(session.clone());
            }
        }

        let session = self.authenticate().await?;
        self.session.lock().await.replace(session.clone());
        Ok(session)
    }

    async fn authenticate(&self) -> Result<OperationSession> {
        let pk = self.signer.get_identity_public_key()?;
        let challenge_req = GetChallengeRequest {
            public_key: pk.serialize().to_vec(),
        };

        let mut auth_client = SparkAuthnServiceClient::new(self.channel.clone());

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

        let session = OperationSession {
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

impl Interceptor for OperationSession {
    fn call(&mut self, mut req: Request<()>) -> std::result::Result<Request<()>, Status> {
        req.metadata_mut()
            .insert("authorization", self.token.clone());
        Ok(req)
    }
}
