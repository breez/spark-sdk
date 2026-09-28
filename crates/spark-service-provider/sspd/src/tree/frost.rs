use std::sync::Arc;

use bitcoin::TapSighash;
use bitcoin::hashes::Hash as _;
use bitcoin::secp256k1::PublicKey;
use spark::services::SigningResult;
use spark::signer::{
    AggregateFrostRequest, FrostSigningCommitmentsWithNonces, SecretSource, SignFrostRequest,
    Signer, SignerError,
};
use spark::utils::frost::aggregate_frost;

pub struct SignAggregateFrostParams<'a> {
    pub signer: &'a Arc<dyn Signer>,
    pub sighash: &'a TapSighash,
    pub signing_public_key: &'a PublicKey,
    pub aggregating_public_key: &'a PublicKey,
    pub signing_private_key: &'a SecretSource,
    pub self_nonce_commitment: &'a FrostSigningCommitmentsWithNonces,
    pub adaptor_public_key: Option<&'a PublicKey>,
    pub verifying_key: &'a PublicKey,
    pub signing_result: SigningResult,
}

pub async fn sign_aggregate_frost(
    params: SignAggregateFrostParams<'_>,
) -> Result<frost_secp256k1_tr::Signature, SignerError> {
    let user_signature = params
        .signer
        .sign_frost(SignFrostRequest {
            message: params.sighash.as_byte_array(),
            public_key: params.signing_public_key,
            private_key: params.signing_private_key,
            verifying_key: params.verifying_key,
            self_nonce_commitment: params.self_nonce_commitment,
            statechain_commitments: params.signing_result.signing_commitments.clone(),
            adaptor_public_key: params.adaptor_public_key,
        })
        .await?;

    aggregate_frost(AggregateFrostRequest {
        message: params.sighash.as_byte_array(),
        statechain_signatures: params.signing_result.signature_shares,
        statechain_public_keys: params.signing_result.public_keys,
        verifying_key: params.verifying_key,
        statechain_commitments: params.signing_result.signing_commitments,
        self_commitment: &params.self_nonce_commitment.commitments,
        public_key: params.aggregating_public_key,
        self_signature: &user_signature,
        adaptor_public_key: params.adaptor_public_key,
    })
}
