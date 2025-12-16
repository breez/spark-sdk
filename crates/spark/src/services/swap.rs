use std::{sync::Arc, time::Duration};

use bitcoin::{consensus::serialize, secp256k1::ecdsa::Signature};
use tracing::trace;
use web_time::SystemTime;

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::spark::{StartTransferRequest, TransferFilter, transfer_filter::Participant},
    },
    services::{LeafKeyTweak, ServiceError, Transfer, TransferId, TransferService},
    signer::{PrivateKeySource, Signer, from_bytes_to_scalar},
    ssp::{CompleteLeavesSwapInput, RequestLeavesSwapInput, ServiceProvider, UserLeafInput},
    tree::TreeNode,
    utils::{
        refund::{
            map_refund_signatures, prepare_leaf_refund_signing_data,
            prepare_refund_so_signing_jobs, sign_aggregate_refunds,
        },
        time::web_time_to_prost_timestamp,
    },
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
                signing_key: PrivateKeySource::Derived(leaf.id.clone()),
                new_signing_key: self.signer.generate_random_key().await?,
            });
        }

        let transfer_id = TransferId::generate();
        let expiry_time = SystemTime::now() + SWAP_EXPIRY_DURATION;
        let receiver_public_key = self.ssp_client.identity_public_key();
        // Prepare leaf data map with refund signing information
        let mut leaf_data_map =
            prepare_leaf_refund_signing_data(&self.signer, &leaf_key_tweaks, receiver_public_key)
                .await?;

        let signing_jobs = prepare_refund_so_signing_jobs(
            self.network,
            &leaf_key_tweaks,
            &mut leaf_data_map,
            false,
        )?;

        // TODO: Migrate to new transfer package format. leaves_to_send is deprecated.
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .start_leaf_swap_v2(StartTransferRequest {
                transfer_id: transfer_id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()
                    .await?
                    .serialize()
                    .to_vec(),
                receiver_identity_public_key: receiver_public_key.serialize().to_vec(),
                expiry_time: Some(
                    web_time_to_prost_timestamp(&expiry_time)
                        .map_err(|_| ServiceError::Generic("Invalid expiry time".to_string()))?,
                ),
                #[allow(deprecated)]
                leaves_to_send: signing_jobs,
                ..Default::default()
            })
            .await?;

        let transfer = response.transfer.ok_or(ServiceError::Generic(
            "response missing transfer".to_string(),
        ))?;
        let transfer: Transfer = transfer.try_into()?;

        let node_signatures = sign_aggregate_refunds(
            &self.signer,
            &leaf_data_map,
            &response.signing_results,
            None,
            None,
            None,
        )
        .await?;
        trace!("Signatures aggregated for transfer: {}", transfer.id);
        let refund_signatures = map_refund_signatures(node_signatures)?;

        let first_leaf = transfer
            .leaves
            .first()
            .ok_or(ServiceError::Generic("no leaves in transfer".to_string()))?;
        let first_leaf_id = &first_leaf.leaf.id;

        let cpfp_refund_signature =
            refund_signatures
                .cpfp_signatures
                .get(first_leaf_id)
                .ok_or(ServiceError::Generic(
                    "refund signature not found".to_string(),
                ))?;
        let direct_refund_signature = refund_signatures.direct_signatures.get(first_leaf_id);
        let direct_from_cpfp_refund_signature = refund_signatures
            .direct_from_cpfp_signatures
            .get(first_leaf_id);

        let (cpfp_adaptor_signature, cpfp_adaptor_private_key) =
            generate_adaptor_from_signature(cpfp_refund_signature)?;
        let maybe_direct_adaptor = direct_refund_signature
            .map(generate_adaptor_from_signature)
            .transpose()?;
        let maybe_direct_from_cpfp_adaptor = direct_from_cpfp_refund_signature
            .map(generate_adaptor_from_signature)
            .transpose()?;

        let mut user_leaves = Vec::new();
        user_leaves.push(UserLeafInput {
            leaf_id: first_leaf_id.to_string(),
            raw_unsigned_refund_transaction: hex::encode(serialize(
                &first_leaf.intermediate_refund_tx,
            )),
            direct_raw_unsigned_refund_transaction: first_leaf
                .intermediate_direct_refund_tx
                .as_ref()
                .map(|tx| hex::encode(serialize(tx))),
            direct_from_cpfp_raw_unsigned_refund_transaction: first_leaf
                .intermediate_direct_from_cpfp_refund_tx
                .as_ref()
                .map(|tx| hex::encode(serialize(tx))),
            adaptor_added_signature: hex::encode(cpfp_adaptor_signature.to_bytes()),
            direct_adaptor_added_signature: maybe_direct_adaptor
                .as_ref()
                .map(|(sig, _)| hex::encode(sig.to_bytes())),
            direct_from_cpfp_adaptor_added_signature: maybe_direct_from_cpfp_adaptor
                .as_ref()
                .map(|(sig, _)| hex::encode(sig.to_bytes())),
        });

        for leaf in transfer.leaves.iter().skip(1) {
            let cpfp_refund_signature =
                refund_signatures.cpfp_signatures.get(&leaf.leaf.id).ok_or(
                    ServiceError::Generic("refund signature not found".to_string()),
                )?;
            let direct_refund_signature = refund_signatures.direct_signatures.get(&leaf.leaf.id);
            let direct_from_cpfp_refund_signature = refund_signatures
                .direct_from_cpfp_signatures
                .get(&leaf.leaf.id);

            let cpfp_adaptor_signature = generate_signature_from_existing_adaptor(
                cpfp_refund_signature,
                &cpfp_adaptor_private_key,
            )?;
            let direct_adaptor_signature = match (direct_refund_signature, &maybe_direct_adaptor) {
                (Some(signature), Some((_, private_key))) => Some(
                    generate_signature_from_existing_adaptor(signature, private_key)?,
                ),
                _ => None,
            };
            let direct_from_cpfp_adaptor_signature = match (
                direct_from_cpfp_refund_signature,
                &maybe_direct_from_cpfp_adaptor,
            ) {
                (Some(signature), Some((_, private_key))) => Some(
                    generate_signature_from_existing_adaptor(signature, private_key)?,
                ),
                _ => None,
            };

            user_leaves.push(UserLeafInput {
                leaf_id: leaf.leaf.id.to_string(),
                raw_unsigned_refund_transaction: hex::encode(serialize(
                    &leaf.intermediate_refund_tx,
                )),
                direct_raw_unsigned_refund_transaction: leaf
                    .intermediate_direct_refund_tx
                    .as_ref()
                    .map(|tx| hex::encode(serialize(tx))),
                direct_from_cpfp_raw_unsigned_refund_transaction: leaf
                    .intermediate_direct_from_cpfp_refund_tx
                    .as_ref()
                    .map(|tx| hex::encode(serialize(tx))),
                adaptor_added_signature: hex::encode(cpfp_adaptor_signature.to_bytes()),
                direct_adaptor_added_signature: direct_adaptor_signature
                    .map(|sig| hex::encode(sig.to_bytes())),
                direct_from_cpfp_adaptor_added_signature: direct_from_cpfp_adaptor_signature
                    .map(|sig| hex::encode(sig.to_bytes())),
            });
        }
        let swap_response = self
            .ssp_client
            .request_leaves_swap(RequestLeavesSwapInput {
                adaptor_pubkey: hex::encode(cpfp_adaptor_private_key.public_key().to_sec1_bytes()),
                direct_adaptor_pubkey: maybe_direct_adaptor
                    .as_ref()
                    .map(|(_, private_key)| hex::encode(private_key.public_key().to_sec1_bytes())),
                direct_from_cpfp_adaptor_pubkey: maybe_direct_from_cpfp_adaptor
                    .as_ref()
                    .map(|(_, private_key)| hex::encode(private_key.public_key().to_sec1_bytes())),
                total_amount_sats: leaf_sum,
                target_amount_sats: target_sum,
                fee_sats: 0, // TODO: Request fee estimate from SSP
                user_leaves,
                idempotency_key: uuid::Uuid::now_v7().to_string(), // TODO: Generate a proper idempotency key
                target_amount_sats_list: maybe_target_amounts,
            })
            .await?;

        // TODO: Validate the amounts in swap_response match the leaf sum, and the target amounts are met.
        // TODO: javascript SDK applies adaptor to signature here for every leaf, but it seems to not do anything?
        let transfer = self
            .transfer_service
            .deliver_transfer_package(&transfer, &leaf_key_tweaks, refund_signatures)
            .await?;
        let complete_response = self
            .ssp_client
            .complete_leaves_swap(CompleteLeavesSwapInput {
                adaptor_secret_key: hex::encode(cpfp_adaptor_private_key.to_bytes()),
                direct_adaptor_secret_key: maybe_direct_adaptor
                    .as_ref()
                    .map(|(_, private_key)| hex::encode(private_key.to_bytes())),
                direct_from_cpfp_adaptor_secret_key: maybe_direct_from_cpfp_adaptor
                    .as_ref()
                    .map(|(_, private_key)| hex::encode(private_key.to_bytes())),
                user_outbound_transfer_external_id: transfer.id.to_string(),
                leaves_swap_request_id: swap_response.id,
            })
            .await?;
        let transfer_id = complete_response
            .inbound_transfer
            .and_then(|t| t.spark_id)
            .ok_or(ServiceError::Generic(
                "inbound transfer spark_id missing".to_string(),
            ))?;
        let transfers = self
            .operator_pool
            .get_coordinator()
            .client
            .query_all_transfers(TransferFilter {
                participant: Some(Participant::ReceiverIdentityPublicKey(
                    self.signer
                        .get_identity_public_key()
                        .await?
                        .serialize()
                        .to_vec(),
                )),
                transfer_ids: vec![transfer_id],
                network: self.network.to_proto_network() as i32,
                ..Default::default()
            })
            .await?;

        let transfer = transfers
            .transfers
            .into_iter()
            .nth(0)
            .ok_or(ServiceError::Generic("transfer not found".to_string()))?;
        let transfer = Transfer::try_from(transfer)?;

        trace!("Claiming transfer with id: {}", transfer.id);
        let claimed_nodes = self
            .transfer_service
            .claim_transfer(&transfer, None)
            .await
            .map_err(|e: ServiceError| {
                ServiceError::Generic(format!("Failed to claim transfer: {e:?}"))
            })?;

        // TODO: in case of error the js sdk cancels initiated transfers

        Ok(claimed_nodes)
    }
}

fn generate_adaptor_from_signature(
    signature: &Signature,
) -> Result<(k256::schnorr::Signature, k256::SecretKey), ServiceError> {
    let signature_bytes = signature.serialize_compact();
    // last 32 bytes of the signature are the s value
    let s = from_bytes_to_scalar(&signature_bytes[32..])?;
    let adaptor_private_key = k256::SecretKey::random(&mut rand::thread_rng());
    let new_s = s.sub(adaptor_private_key.to_nonzero_scalar().as_ref());

    let mut new_signature_bytes = signature_bytes[..32].to_vec();
    new_signature_bytes.extend_from_slice(&new_s.to_bytes());
    let ns: &[u8] = &new_signature_bytes;
    Ok((
        k256::schnorr::Signature::try_from(ns)
            .map_err(|_| ServiceError::Generic("failed to adapt signature".to_string()))?,
        adaptor_private_key,
    ))
}

fn generate_signature_from_existing_adaptor(
    signature: &Signature,
    adaptor_private_key: &k256::SecretKey,
) -> Result<k256::schnorr::Signature, ServiceError> {
    let signature_bytes = signature.serialize_compact();
    let s = from_bytes_to_scalar(&signature_bytes[32..])?;
    let new_s = s.sub(adaptor_private_key.to_nonzero_scalar().as_ref());

    let mut new_signature_bytes = signature_bytes[..32].to_vec();
    new_signature_bytes.extend_from_slice(&new_s.to_bytes());
    let ns: &[u8] = &new_signature_bytes;
    k256::schnorr::Signature::try_from(ns)
        .map_err(|_| ServiceError::Generic("failed to adapt signature".to_string()))
}
