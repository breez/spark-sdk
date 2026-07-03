use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use bitcoin::secp256k1::PublicKey;
use platform_utils::HttpClient;
use tracing::{debug, error};

use crate::header_provider::{HeaderProvider, HeaderProviderError};
use crate::session_store::{Session, SessionStore, SessionStoreError};
use crate::signer::SparkSigner;
use crate::ssp::graphql::client::post_graphql_query;
use crate::ssp::graphql::error::{GraphQLError, GraphQLResult};
use crate::ssp::graphql::queries::{self, get_challenge, verify_challenge};

/// Header provider that authenticates with the Spark Service Provider via
/// challenge-response and emits an `Authorization: Bearer <token>` header.
///
/// Sessions are cached in the user-supplied [`SessionStore`] keyed by the
/// SSP's identity public key.
pub struct SspAuthHeaderProvider {
    client: Arc<dyn HttpClient>,
    full_url: String,
    spark_signer: Arc<dyn SparkSigner>,
    session_store: Arc<dyn SessionStore>,
    ssp_identity_public_key: PublicKey,
}

impl SspAuthHeaderProvider {
    pub fn new(
        base_url: &str,
        schema_endpoint: Option<&str>,
        client: Arc<dyn HttpClient>,
        spark_signer: Arc<dyn SparkSigner>,
        session_store: Arc<dyn SessionStore>,
        ssp_identity_public_key: PublicKey,
    ) -> Self {
        let schema_endpoint = schema_endpoint.unwrap_or("graphql/spark/2025-03-19");
        Self {
            client,
            full_url: format!("{base_url}/{schema_endpoint}"),
            spark_signer,
            session_store,
            ssp_identity_public_key,
        }
    }

    async fn get_or_authenticate(&self) -> GraphQLResult<Session> {
        let cached = self
            .session_store
            .get_session(&self.ssp_identity_public_key)
            .await;
        match cached {
            Ok(session) if session.is_valid() => return Ok(session),
            Ok(_) => debug!("SSP session expired, authenticating"),
            Err(SessionStoreError::NotFound) => {
                debug!("SSP session not found, authenticating")
            }
            Err(SessionStoreError::Generic(e)) => {
                error!("Failed to get SSP session from session store: {}", e)
            }
        }
        let session = self.authenticate().await?;
        self.session_store
            .set_session(&self.ssp_identity_public_key, session.clone())
            .await?;
        Ok(session)
    }

    async fn authenticate(&self) -> GraphQLResult<Session> {
        debug!("Authenticating with ssp");

        let identity_public_key = hex::encode(
            self.spark_signer
                .get_identity_public_key()
                .await?
                .serialize(),
        );

        let challenge_vars = get_challenge::Variables {
            input: get_challenge::GetChallengeInput {
                public_key: identity_public_key.clone(),
            },
        };
        let headers = HashMap::new();

        let challenge_response = post_graphql_query::<queries::GetChallenge, _>(
            self.client.as_ref(),
            &self.full_url,
            &headers,
            challenge_vars,
        )
        .await?;

        let challenge_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&challenge_response.get_challenge.protected_challenge)
            .map_err(|e| GraphQLError::serialization(e.to_string()))?;

        let signature = self
            .spark_signer
            .sign_authentication_challenge(&challenge_bytes)
            .await?
            .serialize_der()
            .to_vec();

        let verify_vars = verify_challenge::Variables {
            input: verify_challenge::VerifyChallengeInput {
                protected_challenge: challenge_response.get_challenge.protected_challenge,
                signature: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&signature),
                identity_public_key,
                provider: None,
            },
        };

        let verify_response = post_graphql_query::<queries::VerifyChallenge, _>(
            self.client.as_ref(),
            &self.full_url,
            &headers,
            verify_vars,
        )
        .await?;

        Ok(Session {
            token: verify_response.verify_challenge.session_token,
            expiration: verify_response
                .verify_challenge
                .valid_until
                .timestamp()
                .try_into()
                .map_err(|_| {
                    GraphQLError::Authentication("Invalid expiration timestamp".to_string())
                })?,
        })
    }
}

#[macros::async_trait]
impl HeaderProvider for SspAuthHeaderProvider {
    async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
        let session = self
            .get_or_authenticate()
            .await
            .map_err(|e| HeaderProviderError::Generic(e.to_string()))?;
        Ok(HashMap::from([(
            "Authorization".to_string(),
            format!("Bearer {}", session.token),
        )]))
    }
}
