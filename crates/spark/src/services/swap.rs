use std::{str::FromStr, sync::Arc, time::Duration};

use bitcoin::consensus::serialize;
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use rand::rngs::OsRng;
use tracing::debug;
use web_time::SystemTime;

use crate::bitcoin::sighash_from_tx;
use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::spark::{
            AdaptorPublicKeyPackage, InitiateSwapPrimaryTransferRequest, TransferFilter,
            transfer_filter::Participant,
        },
    },
    services::{LeafKeyTweak, ServiceError, SigningResult, Transfer, TransferId, TransferService},
    signer::{SecretSource, Signer},
    ssp::{RequestSwapInput, ServiceProvider, UserLeafInput},
    tree::{TreeNode, TreeNodeId},
    utils::{frost::sign_aggregate_frost, refund::RefundSignatures},
};

const SWAP_EXPIRY_DURATION: Duration = Duration::from_secs(2 * 60);

pub struct Swap {
    network: Network,
    operator_pool: Arc<OperatorPool>,
    signer: Arc<dyn Signer>,
    ssp_client: Arc<ServiceProvider>,
    transfer_service: Arc<TransferService>,
}

impl Swap {
    pub fn new(
        network: Network,
        operator_pool: Arc<OperatorPool>,
        signer: Arc<dyn Signer>,
        ssp_client: Arc<ServiceProvider>,
        transfer_service: Arc<TransferService>,
    ) -> Self {
        Swap {
            network,
            operator_pool,
            signer,
            ssp_client,
            transfer_service,
        }
    }

    /// Swaps the specified leaves for new leaves with the target amounts. Returns claimed leaves that should be inserted into the tree.
    /// If no target amounts are provided, the leaves will be swapped for an optimized set of leaves.
    pub async fn swap_leaves(
        &self,
        leaves: &[TreeNode],
        maybe_target_amounts: Option<Vec<u64>>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        debug!(
            "Starting swap for {} leaves, with target amounts: {:?}",
            leaves.len(),
            maybe_target_amounts
        );
        if leaves.is_empty() {
            return Err(ServiceError::Generic("no leaves to swap".to_string()));
        }

        if let Some(target_amounts) = &maybe_target_amounts {
            if target_amounts.is_empty() {
                return Err(ServiceError::InvalidAmount);
            }
            if target_amounts.contains(&0) {
                return Err(ServiceError::InvalidAmount);
            }
        }

        let leaf_sum: u64 = leaves.iter().map(|leaf| leaf.value).sum();

        // If no target amounts are provided, the target sum is the sum of the leaf values.
        let target_sum: u64 = maybe_target_amounts
            .as_ref()
            .map(|target_amounts| target_amounts.iter().sum())
            .unwrap_or(leaf_sum);

        if leaf_sum < target_sum {
            return Err(ServiceError::InsufficientFunds);
        }

        // The target amounts are more than or equal to the leaf values. Continue with split.

        // TODO: split swap into batches (js sdk uses chunks of 100 leaves)

        // Build leaf key tweaks with new signing keys that will be swapped to the ssp.
        let mut leaf_key_tweaks = Vec::with_capacity(leaves.len());
        for leaf in leaves {
            leaf_key_tweaks.push(LeafKeyTweak {
                node: leaf.clone(),
                signing_key: SecretSource::Derived(leaf.id.clone()),
                new_signing_key: SecretSource::Encrypted(
                    self.signer.generate_random_secret().await?,
                ),
            });
        }

        let transfer_id = TransferId::generate();
        let expiry_time =
            SystemTime::now()
                .checked_add(SWAP_EXPIRY_DURATION)
                .ok_or(ServiceError::Generic(
                    "failed to compute swap expiry time".to_string(),
                ))?;
        let receiver_public_key = self.ssp_client.identity_public_key();

        // Pre-generate adaptor key (only CPFP path is used for swap v3)
        let secp = Secp256k1::new();
        let cpfp_adaptor_private_key = SecretKey::new(&mut OsRng);
        let cpfp_adaptor_public_key = PublicKey::from_secret_key(&secp, &cpfp_adaptor_private_key);

        // Prepare the transfer request with signed refunds in the transfer_package.
        // This internally gets signing commitments and signs the refunds with adaptor public key.
        // The PreparedTransferRequest contains both the RPC request and the SignedTx data
        // needed for later FROST aggregation.
        let mut prepared_transfer_request = self
            .transfer_service
            .prepare_transfer_request(
                &transfer_id,
                &leaf_key_tweaks,
                &receiver_public_key,
                RefundSignatures::default(),
                None,
                Some(expiry_time),
                Some(&cpfp_adaptor_public_key),
            )
            .await?;

        // For swap v3, direct transactions should not be provided - clear them from the transfer package
        if let Some(ref mut transfer_package) =
            prepared_transfer_request.transfer_request.transfer_package
        {
            transfer_package.direct_leaves_to_send.clear();
            transfer_package.direct_from_cpfp_leaves_to_send.clear();
        }

        // Call initiate_swap_primary_transfer with the transfer_package and adaptor public keys
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .initiate_swap_primary_transfer(InitiateSwapPrimaryTransferRequest {
                transfer: Some(prepared_transfer_request.transfer_request),
                adaptor_public_keys: Some(AdaptorPublicKeyPackage {
                    adaptor_public_key: cpfp_adaptor_public_key.serialize().to_vec(),
                    // Direct adaptor keys are not used for swap v3
                    direct_adaptor_public_key: Vec::new(),
                    direct_from_cpfp_adaptor_public_key: Vec::new(),
                }),
            })
            .await?;

        let _transfer = response.transfer.ok_or(ServiceError::Generic(
            "response missing transfer".to_string(),
        ))?;

        // Build user_leaves for SSP request by aggregating FROST signatures with the signing
        // results from the RPC response. This produces the adaptor signatures needed by the SSP.
        let mut user_leaves = Vec::new();
        for signing_result in &response.signing_results {
            let leaf_id = TreeNodeId::from_str(&signing_result.leaf_id)
                .map_err(|_| ServiceError::Generic("invalid leaf_id in signing result".into()))?;

            // Find the matching SignedTx from prepared transfer
            let signed_tx = prepared_transfer_request
                .cpfp_signed_txs
                .iter()
                .find(|tx| tx.node_id == leaf_id)
                .ok_or_else(|| {
                    ServiceError::Generic(format!(
                        "No signed tx found for leaf_id: {}",
                        signing_result.leaf_id
                    ))
                })?;

            // Find the matching LeafKeyTweak to get the signing key
            let leaf_key_tweak = leaf_key_tweaks
                .iter()
                .find(|l| l.node.id == leaf_id)
                .ok_or_else(|| {
                    ServiceError::Generic(format!(
                        "No leaf key tweak found for leaf_id: {}",
                        signing_result.leaf_id
                    ))
                })?;

            // Parse the verifying key from the signing result
            let verifying_key = PublicKey::from_slice(&signing_result.verifying_key)
                .map_err(|_| ServiceError::InvalidPublicKey)?;

            // Parse the signing result from the RPC response
            let refund_signing_result = signing_result
                .refund_tx_signing_result
                .as_ref()
                .ok_or_else(|| {
                    ServiceError::Generic("missing refund_tx_signing_result".to_string())
                })?;
            let signing_result_data: SigningResult = refund_signing_result.try_into()?;

            let sighash = sighash_from_tx(
                &signed_tx.tx,
                0,
                &leaf_key_tweak.node.node_tx.output[leaf_key_tweak.node.vout as usize],
            )?;

            // Aggregate the FROST signature with the adaptor public key
            // This combines the user's signature share with the statechain's signature shares
            // to produce the final adaptor signature
            let adaptor_signature =
                sign_aggregate_frost(crate::utils::frost::SignAggregateFrostParams {
                    signer: &self.signer,
                    sighash: &sighash,
                    signing_public_key: &signed_tx.signing_public_key,
                    aggregating_public_key: &signed_tx.signing_public_key,
                    signing_private_key: &leaf_key_tweak.signing_key,
                    self_nonce_commitment: &signed_tx.self_nonce_commitment,
                    adaptor_public_key: Some(&cpfp_adaptor_public_key),
                    verifying_key: &verifying_key,
                    signing_result: signing_result_data,
                })
                .await
                .map_err(|e| ServiceError::Generic(format!("FROST aggregation failed: {e}")))?;

            user_leaves.push(UserLeafInput {
                leaf_id: leaf_id.to_string(),
                raw_unsigned_refund_transaction: hex::encode(serialize(&signed_tx.tx)),
                // Direct transactions are not used for swap v3
                direct_raw_unsigned_refund_transaction: None,
                direct_from_cpfp_raw_unsigned_refund_transaction: None,
                adaptor_added_signature: hex::encode(adaptor_signature.serialize().map_err(
                    |e| ServiceError::Generic(format!("Failed to serialize signature: {e}")),
                )?),
                direct_adaptor_added_signature: None,
                direct_from_cpfp_adaptor_added_signature: None,
            });
        }

        // Call request_swap to SSP (swap v3)
        let swap_response = self
            .ssp_client
            .request_swap(RequestSwapInput {
                adaptor_pubkey: hex::encode(cpfp_adaptor_public_key.serialize()),
                total_amount_sats: leaf_sum,
                target_amount_sats: maybe_target_amounts
                    .clone()
                    .unwrap_or_else(|| vec![target_sum]),
                fee_sats: 0, // TODO: Request fee estimate from SSP
                user_leaves,
                user_outbound_transfer_external_id: transfer_id.to_string(),
            })
            .await?;

        // TODO: Validate the amounts in swap_response match the leaf sum, and the target amounts are met.
        let inbound_transfer_id = swap_response
            .inbound_transfer
            .and_then(|t| t.spark_id)
            .ok_or(ServiceError::Generic(
                "inbound transfer spark_id missing".to_string(),
            ))?;
        let identity_public_key = self
            .signer
            .get_identity_public_key()
            .await?
            .serialize()
            .to_vec();
        let transfers = self
            .operator_pool
            .get_coordinator()
            .client
            .query_all_transfers(TransferFilter {
                participant: Some(Participant::ReceiverIdentityPublicKey(identity_public_key)),
                transfer_ids: vec![inbound_transfer_id],
                network: self.network.to_proto_network() as i32,
                ..Default::default()
            })
            .await?;

        let inbound_transfer = transfers
            .transfers
            .into_iter()
            .next()
            .ok_or(ServiceError::Generic("transfer not found".to_string()))?;
        let inbound_transfer = Transfer::try_from(inbound_transfer)?;

        let claimed_nodes = self
            .transfer_service
            .claim_transfer(&inbound_transfer, None)
            .await
            .map_err(|e: ServiceError| {
                ServiceError::Generic(format!("Failed to claim transfer: {e:?}"))
            })?;

        // TODO: in case of error the js sdk cancels initiated transfers

        Ok(claimed_nodes)
    }
}
