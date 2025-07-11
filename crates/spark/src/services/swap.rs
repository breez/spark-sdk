use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime},
};

use bitcoin::{consensus::serialize, secp256k1::ecdsa};
use prost_types::Timestamp;

use crate::{
    Network,
    operator::rpc::{
        OperatorRpcError, SparkRpcClient,
        spark::{StartTransferRequest, TransferFilter, transfer_filter::Participant},
    },
    services::{
        LeafKeyTweak, LeafRefundSigningData, ServiceError, Transfer, TransferId, TransferService,
    },
    signer::{PrivateKeySource, Signer, from_bytes_to_scalar},
    ssp::{RequestLeavesSwapInput, ServiceProvider, UserLeafInput},
    tree::{TreeNode, TreeNodeId},
    utils::refund::{prepare_refund_so_signing_jobs, sign_aggregate_refunds},
};

const SWAP_EXPIRY_DURATION: Duration = Duration::from_secs(2 * 60);

pub struct Swap<S> {
    coordinator_client: Arc<SparkRpcClient<S>>,
    network: Network,
    signer: S,
    ssp_client: Arc<ServiceProvider<S>>,
    transfer_service: Arc<TransferService<S>>,
}

impl<S> Swap<S>
where
    S: Signer,
{
    pub fn new(
        coordinator_client: Arc<SparkRpcClient<S>>,
        network: Network,
        signer: S,
        ssp_client: Arc<ServiceProvider<S>>,
        transfer_service: Arc<TransferService<S>>,
    ) -> Self {
        Swap {
            coordinator_client,
            network,
            signer,
            ssp_client,
            transfer_service,
        }
    }

    /// Swaps the specified leaves for new leaves with the target amounts. Returns a transfer object that should be claimed to obtain the new leaves.
    pub async fn swap_leaves(
        &self,
        leaves: &[TreeNode],
        target_amounts: Vec<u64>,
    ) -> Result<Transfer, ServiceError> {
        if target_amounts.is_empty() {
            return Err(ServiceError::InvalidAmount);
        }

        let target_sum: u64 = target_amounts.iter().sum();
        let leaf_sum: u64 = leaves.iter().map(|leaf| leaf.value).sum();
        if leaf_sum < target_sum {
            return Err(ServiceError::InsufficientFunds);
        }

        // The target amounts are more than or equal to the leaf values. Continue with split.

        // Build leaf key tweaks with new signing keys that will be swapped to the ssp.
        let leaf_key_tweaks = leaves
            .iter()
            .map(|leaf| {
                Ok(LeafKeyTweak {
                    node: leaf.clone(),
                    signing_key: PrivateKeySource::Derived(leaf.id.clone()),
                    new_signing_key: self.signer.generate_random_key()?,
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        let transfer_id = TransferId::generate();
        let receiver_public_key = self.ssp_client.identity_public_key();
        // Prepare leaf data map with refund signing information
        let mut leaf_data_map = HashMap::new();
        for leaf_key in leaf_key_tweaks.iter() {
            let signing_nonce_commitment = self.signer.generate_frost_signing_commitments().await?;

            leaf_data_map.insert(
                leaf_key.node.id.clone(),
                LeafRefundSigningData {
                    signing_public_key: self
                        .signer
                        .get_public_key_from_private_key_source(&leaf_key.signing_key)?,
                    signing_private_key: leaf_key.signing_key.clone(),
                    receiving_public_key: receiver_public_key,
                    tx: leaf_key.node.node_tx.clone(),
                    refund_tx: leaf_key.node.refund_tx.clone(),
                    signing_nonce_commitment,
                    vout: leaf_key.node.vout,
                },
            );
        }

        let signing_jobs = prepare_refund_so_signing_jobs(
            self.network,
            &leaf_key_tweaks,
            &mut leaf_data_map,
            false,
        )?;

        // TODO: Migrate to new transfer package format. leaves_to_send is deprecated.
        let response = self
            .coordinator_client
            .start_leaf_swap(StartTransferRequest {
                transfer_id: transfer_id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()?
                    .serialize()
                    .to_vec(),
                receiver_identity_public_key: receiver_public_key.serialize().to_vec(),
                expiry_time: Some(Timestamp::from(SystemTime::now() + SWAP_EXPIRY_DURATION)),
                #[allow(deprecated)]
                leaves_to_send: signing_jobs,
                ..Default::default()
            })
            .await?;

        let transfer = response
            .transfer
            .ok_or(ServiceError::ServiceConnectionError(
                OperatorRpcError::Unexpected("response missing transfer".to_string()),
            ))?;
        let transfer: Transfer = transfer.try_into()?;

        let signed_refunds = sign_aggregate_refunds(
            &self.signer,
            &leaf_data_map,
            &response.signing_results,
            None,
        )
        .await?;
        let first_leaf = transfer
            .leaves
            .first()
            .ok_or(ServiceError::ServiceConnectionError(
                OperatorRpcError::Unexpected("no leaves in transfer".to_string()),
            ))?;
        let first_leaf_id = first_leaf.leaf.id.to_string();
        let refund_signature = signed_refunds
            .iter()
            .find(|r| r.node_id == first_leaf_id)
            .ok_or(ServiceError::ServiceConnectionError(
                OperatorRpcError::Unexpected("refund signature not found".to_string()),
            ))?;
        let (adaptor_signature, adaptor_private_key) =
            generate_adaptor_from_signature(&refund_signature.refund_tx_signature)?;

        let mut user_leaves = Vec::new();
        user_leaves.push(UserLeafInput {
            leaf_id: first_leaf_id,
            raw_unsigned_refund_transaction: hex::encode(serialize(
                &first_leaf.intermediate_refund_tx,
            )),
            adaptor_added_signature: hex::encode(adaptor_signature.to_bytes()),
        });

        for leaf in transfer.leaves.iter().skip(1) {
            let refund_signature = signed_refunds
                .iter()
                .find(|r| r.node_id == leaf.leaf.id.to_string())
                .ok_or(ServiceError::ServiceConnectionError(
                    OperatorRpcError::Unexpected("refund signature not found".to_string()),
                ))?;
            let signature = generate_signature_from_existing_adaptor(
                &refund_signature.refund_tx_signature,
                &adaptor_private_key,
            )?;
            user_leaves.push(UserLeafInput {
                leaf_id: leaf.leaf.id.to_string(),
                raw_unsigned_refund_transaction: hex::encode(serialize(
                    &leaf.intermediate_refund_tx,
                )),
                adaptor_added_signature: hex::encode(signature.to_bytes()),
            });
        }
        let swap_response = self
            .ssp_client
            .request_leaves_swap(RequestLeavesSwapInput {
                adaptor_pubkey: hex::encode(adaptor_private_key.public_key().to_sec1_bytes()),
                total_amount_sats: leaf_sum,
                target_amount_sats: target_sum,
                fee_sats: 0, // TODO: Request fee estimate from SSP
                user_leaves,
                idempotency_key: uuid::Uuid::now_v7().to_string(), // TODO: Generate a proper idempotency key
                target_amount_sats_list: Some(target_amounts),
            })
            .await?;

        // TODO: Validate the amounts in swap_response match the leaf sum, and the target amounts are met.
        // TODO: javascript SDK applies adaptor to signature here for every leaf, but it seems to not do anything?
        let refund_signature_map = signed_refunds
            .into_iter()
            .map(|r| {
                let node_id: TreeNodeId = match r.node_id.parse() {
                    Ok(id) => id,
                    Err(_) => return Err(ServiceError::Generic("invalid node_id".to_string())),
                };
                Ok((
                    node_id,
                    ecdsa::Signature::from_compact(&r.refund_tx_signature).map_err(|_| {
                        ServiceError::Generic("invalid refund tx signature".to_string())
                    })?,
                ))
            })
            .collect::<Result<HashMap<_, _>, ServiceError>>()?;
        let transfer = self
            .transfer_service
            .deliver_transfer_package(&transfer, &leaf_key_tweaks, refund_signature_map)
            .await?;
        let complete_response = self
            .ssp_client
            .complete_leaves_swap(
                &hex::encode(adaptor_private_key.to_bytes()),
                &transfer.id.to_string(),
                &swap_response.id,
            )
            .await?;
        let transfer_id = complete_response.inbound_transfer.spark_id.ok_or(
            ServiceError::ServiceConnectionError(OperatorRpcError::Unexpected(
                "inbound transfer spark_id missing".to_string(),
            )),
        )?;
        let transfers = self
            .coordinator_client
            .query_all_transfers(TransferFilter {
                participant: Some(Participant::ReceiverIdentityPublicKey(
                    self.signer.get_identity_public_key()?.serialize().to_vec(),
                )),
                transfer_ids: vec![transfer_id],
                network: self.network.to_proto_network() as i32,
                ..Default::default()
            })
            .await?;

        let transfer =
            transfers
                .transfers
                .into_iter()
                .nth(0)
                .ok_or(ServiceError::ServiceConnectionError(
                    OperatorRpcError::Unexpected("transfer not found".to_string()),
                ))?;
        transfer.try_into()
    }
}

fn generate_adaptor_from_signature(
    signature: &[u8],
) -> Result<(k256::schnorr::Signature, k256::SecretKey), ServiceError> {
    if signature.len() != 64 {
        return Err(ServiceError::Generic(
            "Invalid signature length".to_string(),
        ));
    }
    // last 32 bytes of the signature are the s value
    let s = from_bytes_to_scalar(&signature[32..])?;
    let adaptor_private_key = k256::SecretKey::random(&mut rand::thread_rng());
    let new_s = s.sub(adaptor_private_key.to_nonzero_scalar().as_ref());

    let mut new_signature_bytes = signature[..32].to_vec();
    new_signature_bytes.extend_from_slice(&new_s.to_bytes());
    let ns: &[u8] = &new_signature_bytes;
    Ok((
        k256::schnorr::Signature::try_from(ns)
            .map_err(|_| ServiceError::Generic("failed to adapt signature".to_string()))?,
        adaptor_private_key,
    ))
}

fn generate_signature_from_existing_adaptor(
    signature: &[u8],
    adaptor_private_key: &k256::SecretKey,
) -> Result<k256::schnorr::Signature, ServiceError> {
    if signature.len() != 64 {
        return Err(ServiceError::Generic(
            "Invalid signature length".to_string(),
        ));
    }
    let s = from_bytes_to_scalar(&signature[32..])?;
    let new_s = s.sub(adaptor_private_key.to_nonzero_scalar().as_ref());

    let mut new_signature_bytes = signature[..32].to_vec();
    new_signature_bytes.extend_from_slice(&new_s.to_bytes());
    let ns: &[u8] = &new_signature_bytes;
    k256::schnorr::Signature::try_from(ns)
        .map_err(|_| ServiceError::Generic("failed to adapt signature".to_string()))
}
