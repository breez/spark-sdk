use bitcoin::hashes::{Hash, sha256};
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

use crate::{
    address::SparkAddress,
    operator::rpc,
    services::{FreezeIssuerTokenResponse, ServiceError},
    signer::TokenTransactionKind,
    token::{TokenService, bech32m_decode_token_id},
};

impl TokenService {
    pub async fn freeze_issuer_token(
        &self,
        spark_address: &SparkAddress,
        should_unfreeze: bool,
    ) -> Result<FreezeIssuerTokenResponse, ServiceError> {
        let owner_public_key = spark_address.identity_public_key.serialize().to_vec();
        let token_metadata = self.get_issuer_token_metadata().await?;
        if !token_metadata.is_freezable {
            return Err(ServiceError::Generic(
                "Issuer token is not freezable".to_string(),
            ));
        }

        let token_identifier =
            bech32m_decode_token_id(&token_metadata.identifier, Some(self.network))?;
        let issuer_provided_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                ServiceError::Generic("issuer_provided_timestamp is before UNIX_EPOCH".to_string())
            })?
            .as_millis() as u64;

        let mut requests = Vec::new();
        for operator in self.operator_pool.get_all_operators() {
            let freeze_tokens_payload = rpc::spark_token::FreezeTokensPayload {
                version: 1,
                owner_public_key: Some(owner_public_key.clone()),
                token_identifier: Some(token_identifier.clone()),
                should_unfreeze,
                issuer_provided_timestamp,
                operator_identity_public_key: operator.identity_public_key.serialize().to_vec(),
                ..Default::default()
            };
            let payload_hash = hash_freeze_tokens_payload(&freeze_tokens_payload)?;
            let issuer_signature = self
                .sign_token_digest(TokenTransactionKind::Freeze, &payload_hash)
                .await?;

            let request = operator
                .client
                .freeze_tokens(rpc::spark_token::FreezeTokensRequest {
                    freeze_tokens_payload: Some(freeze_tokens_payload),
                    issuer_signature,
                });
            requests.push(request);
        }
        let responses = futures::future::try_join_all(requests).await?;
        info!("Freeze tokens responses: {:?}", responses);

        responses
            .first()
            .ok_or(ServiceError::Generic(
                "No response from freeze tokens request".to_string(),
            ))?
            .clone()
            .try_into()
    }
}

fn hash_freeze_tokens_payload(
    payload: &rpc::spark_token::FreezeTokensPayload,
) -> Result<Vec<u8>, ServiceError> {
    let mut all_hashes = Vec::new();
    let empty_bytes = vec![];

    let version_hash = sha256::Hash::hash(&payload.version.to_be_bytes())
        .to_byte_array()
        .to_vec();
    all_hashes.push(version_hash);

    let owner_public_key = payload.owner_public_key.as_ref().unwrap_or(&empty_bytes);
    let owner_public_key_hash = sha256::Hash::hash(owner_public_key)
        .to_byte_array()
        .to_vec();
    all_hashes.push(owner_public_key_hash);

    let token_identifier = payload.token_identifier.as_ref().unwrap_or(&empty_bytes);
    let token_identifier_hash = sha256::Hash::hash(token_identifier)
        .to_byte_array()
        .to_vec();
    all_hashes.push(token_identifier_hash);

    let should_unfreeze = if payload.should_unfreeze { 1u8 } else { 0u8 };
    let should_unfreeze_hash = sha256::Hash::hash(&[should_unfreeze])
        .to_byte_array()
        .to_vec();
    all_hashes.push(should_unfreeze_hash);

    let issuer_provided_timestamp_hash =
        sha256::Hash::hash(&payload.issuer_provided_timestamp.to_le_bytes())
            .to_byte_array()
            .to_vec();
    all_hashes.push(issuer_provided_timestamp_hash);

    let operator_identity_public_key_hash =
        sha256::Hash::hash(&payload.operator_identity_public_key)
            .to_byte_array()
            .to_vec();
    all_hashes.push(operator_identity_public_key_hash);

    let final_hash = sha256::Hash::hash(&all_hashes.concat())
        .to_byte_array()
        .to_vec();

    Ok(final_hash)
}
