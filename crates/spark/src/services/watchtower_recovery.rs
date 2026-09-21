use std::sync::Arc;

use bitcoin::consensus::serialize;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::PublicKey;
use bitcoin::{Transaction, TxOut, Witness};
use tracing::debug;

use crate::{
    Network,
    bitcoin::sighash_from_tx,
    operator::{
        OperatorPool,
        rpc::spark::{RecoverWatchtowerExitedLeafRequest, SigningJob},
    },
    services::{ServiceError, SigningResult},
    signer::{
        SignWatchtowerExitRecoveryRequest, SparkSigner, StartWatchtowerExitRecoveryRequest,
        StartedWatchtowerExitRecovery,
    },
    tree::TreeNodeId,
};

const RECOVER_ACTION: &str = "recover_watchtower_exited_leaf";

/// Co-signs a spend of the output a watchtower-exited leaf's value landed in.
///
/// The leaf's pre-signed exit is gone, so this is the only route to that value:
/// a fresh FROST round against the operators, who hold their share of the key it
/// pays to. Every recovery transaction spends the same output, so at most one can
/// confirm and re-signing at a higher fee is safe.
pub struct WatchtowerRecoveryService {
    spark_signer: Arc<dyn SparkSigner>,
    network: Network,
    operator_pool: Arc<OperatorPool>,
}

impl WatchtowerRecoveryService {
    pub fn new(
        spark_signer: Arc<dyn SparkSigner>,
        network: Network,
        operator_pool: Arc<OperatorPool>,
    ) -> Self {
        Self {
            spark_signer,
            network,
            operator_pool,
        }
    }

    /// Returns `recovery_tx` with its single input witnessed.
    ///
    /// `prev_out` is the output being spent: the operators re-derive it from
    /// their own rows and reject the call unless it pays the leaf's verifying
    /// key, so a wrong one fails rather than signing something else.
    pub async fn sign_recovery_tx(
        &self,
        leaf_id: &TreeNodeId,
        mut recovery_tx: Transaction,
        prev_out: &TxOut,
    ) -> Result<Transaction, ServiceError> {
        let sighash = sighash_from_tx(&recovery_tx, 0, prev_out)?;
        let sighash = sighash.to_raw_hash().to_byte_array();

        let StartedWatchtowerExitRecovery {
            signing_public_key,
            nonce_commitment,
            user_signature,
        } = self
            .spark_signer
            .start_watchtower_exit_recovery(StartWatchtowerExitRecoveryRequest {
                leaf_id: leaf_id.clone(),
                user_statement: recovery_statement(self.network, leaf_id, &sighash),
            })
            .await?;

        debug!(%leaf_id, "recovering a watchtower-exited leaf");
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .recover_watchtower_exited_leaf(RecoverWatchtowerExitedLeafRequest {
                leaf_id: leaf_id.to_string(),
                recovery_tx_signing_job: Some(SigningJob {
                    signing_public_key: signing_public_key.serialize().to_vec(),
                    raw_tx: serialize(&recovery_tx),
                    signing_nonce_commitment: Some(nonce_commitment.commitments.try_into()?),
                }),
                user_signature: user_signature.serialize_der().to_vec(),
            })
            .await?;

        let signing_result: SigningResult = response
            .recovery_tx_signing_result
            .as_ref()
            .map(TryInto::try_into)
            .transpose()?
            .ok_or(ServiceError::MissingTreeSignatures)?;
        let verifying_key = PublicKey::from_slice(&response.verifying_key)
            .map_err(|_| ServiceError::InvalidVerifyingKey)?;

        let signature = self
            .spark_signer
            .sign_watchtower_exit_recovery(SignWatchtowerExitRecoveryRequest {
                leaf_id: leaf_id.clone(),
                sighash,
                verifying_key,
                nonce_commitment,
                statechain_commitments: signing_result.signing_commitments,
                statechain_signatures: signing_result.signature_shares,
                statechain_public_keys: signing_result.public_keys,
            })
            .await?;

        let mut witness = Witness::new();
        witness.push(signature.serialize()?);
        recovery_tx.input[0].witness = witness;
        Ok(recovery_tx)
    }
}

/// The preimage of the statement authorising one recovery. The signer hashes it
/// with SHA-256 before signing, which is the digest the operators verify
/// against. The sighash is part of it, so an authorisation cannot be replayed
/// onto a different recovery transaction.
fn recovery_statement(network: Network, leaf_id: &TreeNodeId, sighash: &[u8; 32]) -> Vec<u8> {
    let mut payload = RECOVER_ACTION.as_bytes().to_vec();
    // Upper-case, unlike the static-deposit statements.
    payload.extend_from_slice(network.to_string().to_uppercase().as_bytes());
    payload.extend_from_slice(leaf_id.to_string().as_bytes());
    payload.extend_from_slice(sighash);
    payload
}

#[cfg(test)]
mod tests {
    use super::recovery_statement;
    use crate::{Network, tree::TreeNodeId};

    /// Pins the statement the operators rebuild in
    /// `createRecoverWatchtowerExitedLeafStatement`: a signature over anything
    /// else is rejected, and the fields are not recoverable from the failure.
    #[test]
    fn the_statement_is_the_operators_concatenation() {
        let leaf_id: TreeNodeId = "11111111-2222-3333-4444-555555555555".parse().unwrap();
        let sighash = [7u8; 32];

        let statement = recovery_statement(Network::Regtest, &leaf_id, &sighash);

        let mut expected = b"recover_watchtower_exited_leaf".to_vec();
        expected.extend_from_slice(b"REGTEST");
        expected.extend_from_slice(b"11111111-2222-3333-4444-555555555555");
        expected.extend_from_slice(&sighash);
        assert_eq!(statement, expected);
    }
}
