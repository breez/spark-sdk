use std::sync::Arc;

use bitcoin::hashes::Hash;
use bitcoin::secp256k1::PublicKey;
use bitcoin::{Transaction, TxOut};

use crate::bitcoin::sighash_from_tx;
use crate::services::SigningResult;
use crate::signer::{
    AggregateFrostRequest, FrostSigningCommitmentsWithNonces, PrivateKeySource, SignerError,
};
use crate::signer::{SignFrostRequest, Signer};

pub struct SignAggregateFrostParams<'a> {
    pub signer: &'a Arc<dyn Signer>,
    pub tx: &'a Transaction,
    pub prev_out: &'a TxOut,
    pub signing_public_key: &'a PublicKey,
    pub aggregating_public_key: &'a PublicKey,
    pub signing_private_key: &'a PrivateKeySource,
    pub self_nonce_commitment: &'a FrostSigningCommitmentsWithNonces,
    pub adaptor_public_key: Option<&'a PublicKey>,
    pub verifying_key: &'a PublicKey,
    pub signing_result: SigningResult,
}

/// Performs a complete FROST signing and aggregation flow for a Bitcoin transaction.
///
/// This function handles the full FROST (Flexible Round-Optimized Schnorr Threshold) signature process:
/// 1. Creates a sighash for the transaction
/// 2. Signs the sighash using FROST with the user's key
/// 3. Aggregates the user's signature with signatures from statechain participants
///
/// The function supports optional adaptor signatures when an adaptor public key is provided.
/// Adaptor signatures allow the signature to be "encrypted" under an adaptor key,
/// requiring additional knowledge to extract the complete signature.
///
/// # Arguments
///
/// * `params` - A `SignAggregateFrostParams` struct containing:
///   - `signer`: Reference to the signer implementation
///   - `tx`: The Bitcoin transaction to sign
///   - `prev_out`: The previous transaction output being spent
///   - `signing_public_key`: The public key to use for signing (user's key)
///   - `aggregating_public_key`: The public key to use for aggregation
///   - `signing_private_key`: The private key source for signing
///   - `self_nonce_commitment`: User's FROST nonce commitments with nonces
///   - `adaptor_public_key`: Optional public key for adaptor signatures
///   - `verifying_key`: The combined public key used to verify the signature
///   - `signing_result`: Contains signature shares and commitments from statechain
///
/// # Returns
///
/// A `Result` containing:
/// - `Ok(frost_secp256k1_tr::Signature)`: The aggregated FROST signature on success
/// - `Err(SignerError)`: If any part of the signing or aggregation process fails
pub async fn sign_aggregate_frost(
    params: SignAggregateFrostParams<'_>,
) -> Result<frost_secp256k1_tr::Signature, SignerError> {
    // Create the sighash for the transaction
    let sighash = sighash_from_tx(params.tx, 0, params.prev_out)
        .map_err(|e| SignerError::Generic(e.to_string()))?;

    // Sign with FROST
    let user_signature = params
        .signer
        .sign_frost(SignFrostRequest {
            message: sighash.as_byte_array(),
            public_key: params.signing_public_key,
            private_key: params.signing_private_key,
            verifying_key: params.verifying_key,
            self_nonce_commitment: params.self_nonce_commitment,
            statechain_commitments: params.signing_result.signing_commitments.clone(),
            adaptor_public_key: params.adaptor_public_key,
        })
        .await?;

    // Aggregate FROST signatures
    let aggregate_signature = params
        .signer
        .aggregate_frost(AggregateFrostRequest {
            message: sighash.as_byte_array(),
            statechain_signatures: params.signing_result.signature_shares,
            statechain_public_keys: params.signing_result.public_keys,
            verifying_key: params.verifying_key,
            statechain_commitments: params.signing_result.signing_commitments,
            self_commitment: &params.self_nonce_commitment.commitments,
            public_key: params.aggregating_public_key,
            self_signature: &user_signature,
            adaptor_public_key: params.adaptor_public_key,
        })
        .await?;

    Ok(aggregate_signature)
}
