use std::time::Duration;
use std::{collections::HashMap, str::FromStr, sync::Arc};

use crate::Network;
use crate::core::{initial_sequence, next_sequence};
use crate::operator::rpc::common::SignatureIntent;
use crate::operator::rpc::spark::transfer_filter::Participant;
use crate::operator::rpc::spark::{
    ExtendLeafRequest, FinalizeNodeSignaturesRequest, NodeSignatures, SigningJob, TransferFilter,
};
use crate::operator::rpc::{self as operator_rpc, OperatorRpcError};
use crate::services::models::{
    LeafKeyTweak, Transfer, map_public_keys, map_signature_shares, map_signing_nonce_commitments,
};
use crate::services::{PagingFilter, ProofMap, TransferId, TransferStatus};
use crate::signer::{
    AggregateFrostRequest, PrivateKeySource, SecretToSplit, SignFrostRequest, VerifiableSecretShare,
};
use crate::utils::anchor::ephemeral_anchor_output;
use crate::utils::refund::{create_refund_tx, sign_refunds};

use bitcoin::absolute::LockTime;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::key::Secp256k1;
use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::secp256k1::{PublicKey, SecretKey};
use bitcoin::transaction::Version;
use bitcoin::{OutPoint, Transaction, TxIn};
use frost_secp256k1_tr::{Identifier, round1::SigningCommitments};
use k256::Scalar;
use prost::Message as ProstMessage;
use tracing::{debug, error, trace};

use crate::{
    bitcoin::sighash_from_tx,
    signer::Signer,
    tree::{TreeNode, TreeNodeId},
};

use super::ServiceError;

/// Helper struct for leaf refund signing data
#[derive(Debug, Clone)]
pub struct LeafRefundSigningData {
    pub signing_private_key: PrivateKeySource,
    pub signing_public_key: PublicKey,
    pub receiving_public_key: PublicKey,
    pub tx: Transaction,
    pub refund_tx: Option<Transaction>,
    pub signing_nonce_commitment: SigningCommitments,
    pub vout: u32,
}

/// Configuration for claiming transfers
pub struct ClaimTransferConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub should_extend_timelocks: bool,
    pub should_refresh_timelocks: bool,
}

impl Default for ClaimTransferConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay_ms: 1000,
            max_delay_ms: 10000,
            should_extend_timelocks: true,
            should_refresh_timelocks: true,
        }
    }
}

pub struct TransferService<S: Signer> {
    signer: S,
    network: Network,
    split_secret_threshold: u32,
    coordinator_client: Arc<operator_rpc::SparkRpcClient<S>>,
    operator_clients: Vec<Arc<operator_rpc::SparkRpcClient<S>>>,
}

impl<S: Signer> TransferService<S> {
    pub fn new(
        signer: S,
        network: Network,
        split_secret_threshold: u32,
        coordinator_client: Arc<operator_rpc::SparkRpcClient<S>>,
        operator_clients: Vec<Arc<operator_rpc::SparkRpcClient<S>>>,
    ) -> Self {
        Self {
            signer,
            network,
            split_secret_threshold,
            coordinator_client,
            operator_clients,
        }
    }

    /// Creates and initiates a new transfer with the given leaves.
    ///
    /// Generates a transfer package containing encrypted key material, refund signatures,
    /// and proofs that are distributed to the statechain operators.
    pub async fn transfer_leaves_to(
        &self,
        leaves: &[TreeNode],
        receiver_id: &PublicKey,
    ) -> Result<Transfer, ServiceError> {
        // check if we need to refresh or extend timelocks
        let leaves = self.check_refresh_timelock_nodes(leaves).await?;
        let leaves = self.check_extend_timelock_nodes(leaves).await?;

        // build leaf key tweaks with new signing keys that we will send to the receiver
        let leaf_key_tweaks = leaves
            .iter()
            .map(|leaf| {
                let our_key = PrivateKeySource::Derived(leaf.id.clone());
                let ephemeral_key = self.signer.generate_random_key()?;

                Ok(LeafKeyTweak {
                    node: leaf.clone(),
                    signing_key: our_key,
                    new_signing_key: ephemeral_key,
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        let transfer = self
            .send_transfer_with_key_tweaks(&leaf_key_tweaks, receiver_id)
            .await?;

        Ok(transfer)
    }

    pub async fn send_transfer_with_key_tweaks(
        &self,
        leaf_key_tweaks: &[LeafKeyTweak],
        receiver_id: &PublicKey,
    ) -> Result<Transfer, ServiceError> {
        let transfer_id = TransferId::generate();

        let key_tweak_input_map = self
            .prepare_send_transfer_key_tweaks(
                &transfer_id,
                receiver_id,
                &leaf_key_tweaks,
                HashMap::new(),
            )
            .await?;

        let transfer_package = self
            .prepare_transfer_package(
                &transfer_id,
                key_tweak_input_map,
                &leaf_key_tweaks,
                receiver_id,
            )
            .await?;

        // Make request to start transfer
        let start_transfer_request = operator_rpc::spark::StartTransferRequest {
            transfer_id: transfer_id.to_string(),
            owner_identity_public_key: self.signer.get_identity_public_key()?.serialize().to_vec(),
            receiver_identity_public_key: receiver_id.serialize().to_vec(),
            transfer_package: Some(transfer_package),
            ..Default::default()
        };
        trace!(
            "About to send start_transfer_request: {:?}",
            start_transfer_request
        );
        let transfer = self
            .coordinator_client
            .start_transfer(start_transfer_request)
            .await?
            .transfer
            .ok_or(ServiceError::ServiceConnectionError(
                OperatorRpcError::Unexpected("No transfer from operator".to_string()),
            ))?;

        Ok(transfer.try_into()?)
    }

    async fn prepare_send_transfer_key_tweaks(
        &self,
        transfer_id: &TransferId,
        receiver_public_key: &PublicKey,
        leaves: &[LeafKeyTweak],
        refund_signatures: HashMap<TreeNodeId, Signature>,
    ) -> Result<HashMap<Identifier, Vec<operator_rpc::spark::SendLeafKeyTweak>>, ServiceError> {
        let mut leaves_tweaks_map = HashMap::new();

        for leaf in leaves {
            let refund_signature = refund_signatures.get(&leaf.node.id).cloned();
            let leaf_tweaks_map = self
                .prepare_single_send_transfer_key_tweak(
                    transfer_id,
                    leaf,
                    receiver_public_key,
                    refund_signature,
                )
                .await?;

            // Merge the leaf tweaks into the main map
            for (identifier, leaf_tweak) in leaf_tweaks_map {
                leaves_tweaks_map
                    .entry(identifier)
                    .or_insert_with(Vec::new)
                    .push(leaf_tweak);
            }
        }

        Ok(leaves_tweaks_map)
    }

    /// Prepares a single leaf key tweak for transfer
    async fn prepare_single_send_transfer_key_tweak(
        &self,
        transfer_id: &TransferId,
        leaf: &LeafKeyTweak,
        receiver_public_key: &PublicKey,
        refund_signature: Option<Signature>,
    ) -> Result<HashMap<Identifier, operator_rpc::spark::SendLeafKeyTweak>, ServiceError> {
        let signing_operators: Vec<_> = self
            .operator_clients
            .iter()
            .map(|c| c.operator.clone())
            .collect();

        // Calculate the key tweak by subtracting keys
        let privkey_tweak = self
            .signer
            .subtract_private_keys(&leaf.signing_key, &leaf.new_signing_key)?;

        // Split the secret into threshold shares with proofs
        let shares = self.signer.split_secret_with_proofs(
            &SecretToSplit::PrivateKey(privkey_tweak),
            self.split_secret_threshold,
            signing_operators.len(),
        )?;

        trace!(
            "prepare transfer: Split secret into {} shares",
            shares.len()
        );

        // TODO: move secp to a field of self to avoid creating it every time
        let secp = bitcoin::secp256k1::Secp256k1::new();
        // Create pubkey shares tweak map
        let mut pubkey_shares_tweak = HashMap::new();
        for operator in &signing_operators {
            let operator_identifier = hex::encode(operator.identifier.serialize());

            let share = find_share(&shares, operator.id).ok_or_else(|| {
                ServiceError::Generic(format!("Share not found for operator {}", operator.id))
            })?;
            trace!("Found share for operator {}: {:?}", operator.id, share);

            let pubkey_tweak = SecretKey::from_slice(&share.secret_share.share.to_bytes())
                .map_err(|_| ServiceError::Generic("Invalid secret share".to_string()))?
                .public_key(&secp);
            pubkey_shares_tweak.insert(operator_identifier, pubkey_tweak.serialize().to_vec());
        }

        // Encrypt the leaf private key for the receiver
        let secret_cipher = match &leaf.new_signing_key {
            PrivateKeySource::Derived(_) => {
                return Err(ServiceError::Generic(
                    "Trying to share derived private key".to_string(),
                ));
            }
            PrivateKeySource::Encrypted(private_key) => self
                .signer
                .encrypt_private_key_for_receiver(private_key, receiver_public_key)?,
        };

        // Create the signing payload: leaf_id || transfer_id || secret_cipher
        let mut payload = Vec::new();
        payload.extend_from_slice(leaf.node.id.to_string().as_bytes());
        payload.extend_from_slice(transfer_id.to_string().as_bytes());
        payload.extend_from_slice(&secret_cipher);

        // Sign the hash with identity key
        let signature = self.signer.sign_message_ecdsa_with_identity_key(&payload)?;

        trace!(
            "Prepared leaf key tweak for transfer: leaf_id={}, transfer_id={}, signature={}",
            leaf.node.id,
            transfer_id,
            hex::encode(signature.serialize_compact())
        );

        // Create leaf tweaks map for each signing operator
        let mut leaf_tweaks_map = HashMap::new();

        for operator in &signing_operators {
            let share = find_share(&shares, operator.id).ok_or_else(|| {
                ServiceError::Generic(format!("Share not found for operator {}", operator.id))
            })?;

            let send_leaf_key_tweak = operator_rpc::spark::SendLeafKeyTweak {
                leaf_id: leaf.node.id.to_string(),
                secret_share_tweak: Some(operator_rpc::spark::SecretShare {
                    secret_share: share.secret_share.share.to_bytes().to_vec(),
                    proofs: share
                        .proofs
                        .iter()
                        .map(|p| p.to_sec1_bytes().to_vec())
                        .collect(),
                }),
                pubkey_shares_tweak: pubkey_shares_tweak.clone(),
                secret_cipher: secret_cipher.clone(),
                signature: signature.serialize_compact().to_vec(),
                refund_signature: refund_signature
                    .clone()
                    .map(|s| s.serialize_compact().to_vec())
                    .unwrap_or_default(),
            };

            leaf_tweaks_map.insert(operator.identifier, send_leaf_key_tweak);
        }

        Ok(leaf_tweaks_map)
    }

    async fn prepare_transfer_package(
        &self,
        transfer_id: &TransferId,
        key_tweak_input_map: HashMap<Identifier, Vec<operator_rpc::spark::SendLeafKeyTweak>>,
        leaf_key_tweaks: &[LeafKeyTweak],
        receiver_public_key: &PublicKey,
    ) -> Result<operator_rpc::spark::TransferPackage, ServiceError> {
        let signing_commitments = self
            .coordinator_client
            .get_signing_commitments(operator_rpc::spark::GetSigningCommitmentsRequest {
                node_ids: leaf_key_tweaks
                    .iter()
                    .map(|l| l.node.id.to_string())
                    .collect(),
            })
            .await?
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(sc.signing_nonce_commitments.clone()))
            .collect::<Result<Vec<_>, _>>()?;

        let leaf_signing_jobs = sign_refunds(
            &self.signer,
            leaf_key_tweaks,
            signing_commitments,
            receiver_public_key,
            self.network,
        )
        .await?;

        let encrypted_key_tweaks = self.encrypt_key_tweaks(&key_tweak_input_map)?;

        let unsigned_transfer_package = operator_rpc::spark::TransferPackage {
            leaves_to_send: leaf_signing_jobs
                .into_iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            key_tweak_package: encrypted_key_tweaks
                .into_iter()
                .map(|(k, v)| (hex::encode(k.serialize()), v))
                .collect(),
            user_signature: Vec::new(),
        };

        let signed_transfer_package =
            self.sign_transfer_package(transfer_id, unsigned_transfer_package)?;

        Ok(signed_transfer_package)
    }

    /// Encrypts key tweaks for each signing operator using their identity public keys
    fn encrypt_key_tweaks(
        &self,
        key_tweak_input_map: &HashMap<Identifier, Vec<operator_rpc::spark::SendLeafKeyTweak>>,
    ) -> Result<HashMap<Identifier, Vec<u8>>, ServiceError> {
        let mut encrypted_key_tweaks = HashMap::new();

        for (key, value) in key_tweak_input_map {
            // Create the protobuf message to encrypt
            let proto_to_encrypt = operator_rpc::spark::SendLeafKeyTweaks {
                leaves_to_send: value.clone(),
            };

            let proto_to_encrypt_binary = proto_to_encrypt.encode_to_vec();

            // Get the operator by identifier
            let operator_client = self
                .operator_clients
                .iter()
                .find(|c| c.operator.identifier == *key)
                .ok_or_else(|| ServiceError::Generic("Operator not found".to_string()))?;

            // Encrypt the binary data using the operator's identity public key
            let encrypted_proto = self.encrypt_with_public_key(
                &operator_client.operator.identity_public_key,
                &proto_to_encrypt_binary,
            )?;

            encrypted_key_tweaks.insert(key.clone(), encrypted_proto);
        }

        Ok(encrypted_key_tweaks)
    }

    /// Encrypts data using ECIES with the given public key
    fn encrypt_with_public_key(
        &self,
        public_key: &PublicKey,
        data: &[u8],
    ) -> Result<Vec<u8>, ServiceError> {
        // Convert bitcoin PublicKey to the format expected by ecies crate
        let public_key_bytes = public_key.serialize_uncompressed();

        // Use ECIES to encrypt the data
        ecies::encrypt(&public_key_bytes, data)
            .map_err(|e| ServiceError::Generic(format!("ECIES encryption failed: {}", e)))
    }

    fn sign_transfer_package(
        &self,
        transfer_id: &TransferId,
        transfer_package: operator_rpc::spark::TransferPackage,
    ) -> Result<operator_rpc::spark::TransferPackage, ServiceError> {
        let signing_payload =
            self.get_transfer_package_signing_payload(transfer_id, &transfer_package)?;

        let signature = self
            .signer
            .sign_message_ecdsa_with_identity_key(&signing_payload)
            .map_err(|e| ServiceError::SignerError(e))?;

        // Create a new transfer package with the signature
        let mut signed_package = transfer_package;
        signed_package.user_signature = signature.serialize_der().to_vec();

        Ok(signed_package)
    }

    /// Creates the signing payload for a transfer package by hashing the transfer ID and encrypted payload
    fn get_transfer_package_signing_payload(
        &self,
        transfer_id: &TransferId,
        transfer_package: &operator_rpc::spark::TransferPackage,
    ) -> Result<Vec<u8>, ServiceError> {
        let transfer_id_bytes = hex::decode(&transfer_id.to_string().replace("-", "")).unwrap();
        // Get the encrypted payload and convert to sorted key-value pairs
        let encrypted_payload = &transfer_package.key_tweak_package;
        let mut pairs: Vec<(String, Vec<u8>)> = encrypted_payload
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Sort by key to ensure deterministic ordering
        pairs.sort_by(|a, b| a.0.cmp(&b.0));

        // Build the message following the JavaScript pattern:
        // transfer_id_bytes + key + ":" + value + ";" for each pair
        let mut message = transfer_id_bytes;

        for (key, value) in pairs {
            message.extend_from_slice(key.as_bytes());
            message.extend_from_slice(b":");
            message.extend_from_slice(&value);
            message.extend_from_slice(b";");
        }

        Ok(message)
    }

    /// Claims a transfer with retry logic and automatic leaf preparation
    pub async fn claim_transfer(
        &self,
        transfer: &Transfer,
        config: Option<ClaimTransferConfig>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let config = config.unwrap_or_default();

        let mut retry_count = 0;
        loop {
            if retry_count >= config.max_retries {
                return Err(ServiceError::MaxRetriesExceeded);
            }

            // Introduce an exponential backoff delay before retrying.
            if retry_count > 0 {
                let delay_ms =
                    (config.base_delay_ms * 2u64.pow(retry_count - 1)).min(config.max_delay_ms);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }

            // Verify the pending transfer and get leaf key map
            let leaf_key_map = match self.verify_pending_transfer(transfer).await {
                Ok(map) => map,
                Err(e) => {
                    error!("Failed to verify pending transfer: {}", e);
                    retry_count += 1;
                    continue;
                }
            };

            // Prepare leaves to claim
            let leaves_to_claim = match self
                .prepare_leaves_for_claiming(transfer, &leaf_key_map)
                .await
            {
                Ok(leaves) => leaves,
                Err(ServiceError::NoLeavesToClaim) => {
                    debug!("There are no leaves to claim for this transfer");
                    return Ok(Vec::new());
                }
                Err(e) => {
                    error!("Failed to prepare leaves for claiming: {}", e);
                    retry_count += 1;
                    continue;
                }
            };

            // Actually claim the transfer
            let result = match self
                .claim_transfer_with_leaves(transfer, leaves_to_claim)
                .await
            {
                Ok(res) => res,
                Err(e) => {
                    error!("Failed to claim transfer with leaves: {}", e);
                    retry_count += 1;
                    continue;
                }
            };

            // Post-process the claimed nodes
            let result = self.post_process_claimed_nodes(result, &config).await?;

            return Ok(result);
        }
    }

    /// Prepares leaves for claiming by creating LeafKeyTweak structs
    async fn prepare_leaves_for_claiming(
        &self,
        transfer: &Transfer,
        leaf_key_map: &HashMap<TreeNodeId, PrivateKeySource>,
    ) -> Result<Vec<LeafKeyTweak>, ServiceError> {
        let mut leaves_to_claim = Vec::new();

        for leaf in &transfer.leaves {
            let Some(leaf_key) = leaf_key_map.get(&leaf.leaf.id) else {
                continue;
            };

            leaves_to_claim.push(LeafKeyTweak {
                node: leaf.leaf.clone(),
                signing_key: leaf_key.clone(),
                new_signing_key: PrivateKeySource::Derived(leaf.leaf.id.clone()),
            });
        }

        if leaves_to_claim.is_empty() {
            return Err(ServiceError::NoLeavesToClaim);
        }

        Ok(leaves_to_claim)
    }

    /// Post-processes claimed nodes (timelock operations)
    async fn post_process_claimed_nodes(
        &self,
        nodes: Vec<TreeNode>,
        config: &ClaimTransferConfig,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let mut result = nodes;

        if config.should_refresh_timelocks {
            result = self.check_refresh_timelock_nodes(&result).await?;
        }

        if config.should_extend_timelocks {
            result = self.check_extend_timelock_nodes(result).await?;
        }

        Ok(result)
    }

    /// Checks and refreshes timelock nodes if needed
    async fn check_refresh_timelock_nodes(
        &self,
        nodes: &[TreeNode],
    ) -> Result<Vec<TreeNode>, ServiceError> {
        // TODO: Implement timelock refresh logic
        // For now, return nodes unchanged
        Ok(nodes.to_vec())
    }

    /// Refreshes timelocks on a chain of connected nodes to prevent expiration.
    /// Updates sequence numbers on both node transactions and refund transactions
    /// in a coordinated manner across the entire chain.
    async fn refresh_timelock_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        todo!()
    }

    /// Checks and extends timelock nodes if needed
    async fn check_extend_timelock_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        // if node needs to be extended, call extend_time_lock
        // TODO: implement
        // For now, return nodes unchanged
        Ok(nodes)
    }

    /// Extends the timelock on a single node by creating new node and refund transactions.
    /// Creates a new node transaction that spends the current node with an extended timelock,
    /// and a corresponding refund transaction. This is more comprehensive than refreshing
    /// as it creates entirely new transactions rather than just updating sequence numbers.
    pub async fn extend_time_lock(&self, node: &TreeNode) -> Result<Vec<TreeNode>, ServiceError> {
        let signing_key = PrivateKeySource::Derived(node.id.clone());
        let signing_public_key = self
            .signer
            .get_public_key_from_private_key_source(&signing_key)?;

        let refund_tx = node
            .refund_tx
            .clone()
            .ok_or(ServiceError::Generic("No refund tx".to_string()))?;

        let new_node_sequence = next_sequence(refund_tx.input[0].sequence).ok_or(
            ServiceError::Generic("Failed to get next sequence".to_string()),
        )?;

        let mut new_node_tx = bitcoin::Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![],
        };

        new_node_tx.input.push(TxIn {
            previous_output: OutPoint {
                txid: node.node_tx.compute_txid(),
                vout: 0,
            },
            sequence: new_node_sequence,
            ..Default::default()
        });

        // TODO: js references applying a fee here, but is commented out. To do so, instead of cloning the output, we create a new one with the fee applied
        new_node_tx.output.push(node.node_tx.output[0].clone());
        new_node_tx.output.push(ephemeral_anchor_output());

        let new_refund_tx = create_refund_tx(
            initial_sequence(),
            OutPoint {
                txid: new_node_tx.compute_txid(),
                vout: 0,
            },
            new_node_tx.output[0].value.to_sat(),
            &signing_public_key,
            self.network,
        )
        .map_err(|e| ServiceError::Generic(e.to_string()))?;

        let node_sighash = sighash_from_tx(&new_node_tx, 0, &node.node_tx.output[0])?;
        let refund_sighash = sighash_from_tx(&new_refund_tx, 0, &new_node_tx.output[0])?;

        let new_node_signing_commitments = self.signer.generate_frost_signing_commitments().await?;
        let new_refund_signing_commitments =
            self.signer.generate_frost_signing_commitments().await?;

        let new_node_signing_job = SigningJob {
            signing_public_key: signing_public_key.serialize().to_vec(),
            raw_tx: bitcoin::consensus::serialize(&new_node_tx),
            signing_nonce_commitment: Some(new_node_signing_commitments.try_into()?),
        };

        let new_refund_signing_job = SigningJob {
            signing_public_key: signing_public_key.serialize().to_vec(),
            raw_tx: bitcoin::consensus::serialize(&new_refund_tx),
            signing_nonce_commitment: Some(new_refund_signing_commitments.try_into()?),
        };

        let response = self
            .coordinator_client
            .extend_leaf(ExtendLeafRequest {
                leaf_id: node.id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()?
                    .serialize()
                    .to_vec(),
                node_tx_signing_job: Some(new_node_signing_job),
                refund_tx_signing_job: Some(new_refund_signing_job),
            })
            .await?;

        let node_tx_signing_result =
            response
                .node_tx_signing_result
                .ok_or(ServiceError::Generic(
                    "Node tx signing result is none".to_string(),
                ))?;
        let refund_tx_signing_result =
            response
                .refund_tx_signing_result
                .ok_or(ServiceError::Generic(
                    "Refund tx signing result is none".to_string(),
                ))?;

        let new_node_tx_verifying_key =
            PublicKey::from_slice(&node_tx_signing_result.verifying_key)
                .map_err(|_| ServiceError::ValidationError("Invalid verifying key".to_string()))?;
        let new_refund_tx_verifying_key =
            PublicKey::from_slice(&refund_tx_signing_result.verifying_key)
                .map_err(|_| ServiceError::ValidationError("Invalid verifying key".to_string()))?;

        let new_node_tx_signing_result =
            node_tx_signing_result
                .signing_result
                .ok_or(ServiceError::Generic(
                    "Node tx signing result is none".to_string(),
                ))?;
        let new_refund_tx_signing_result =
            refund_tx_signing_result
                .signing_result
                .ok_or(ServiceError::Generic(
                    "Refund tx signing result is none".to_string(),
                ))?;

        let new_node_statechain_commitments =
            map_signing_nonce_commitments(new_node_tx_signing_result.signing_nonce_commitments)?;
        let new_refund_statechain_commitments =
            map_signing_nonce_commitments(new_refund_tx_signing_result.signing_nonce_commitments)?;

        let new_node_statechain_signatures =
            map_signature_shares(new_node_tx_signing_result.signature_shares)?;
        let new_refund_statechain_signatures =
            map_signature_shares(new_refund_tx_signing_result.signature_shares)?;

        let new_node_statechain_public_keys =
            map_public_keys(new_node_tx_signing_result.public_keys)?;
        let new_refund_statechain_public_keys =
            map_public_keys(new_refund_tx_signing_result.public_keys)?;

        // sign node and refund txs
        let node_user_signature = self
            .signer
            .sign_frost(SignFrostRequest {
                message: node_sighash.as_byte_array(),
                public_key: &signing_public_key,
                private_key: &signing_key,
                verifying_key: &new_node_tx_verifying_key,
                self_commitment: &new_node_signing_commitments,
                statechain_commitments: new_node_statechain_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;

        let refund_user_signature = self
            .signer
            .sign_frost(SignFrostRequest {
                message: refund_sighash.as_byte_array(),
                public_key: &signing_public_key,
                private_key: &signing_key,
                verifying_key: &new_refund_tx_verifying_key,
                self_commitment: &new_refund_signing_commitments,
                statechain_commitments: new_refund_statechain_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;

        let node_signature = self
            .signer
            .aggregate_frost(AggregateFrostRequest {
                message: node_sighash.as_byte_array(),
                statechain_signatures: new_node_statechain_signatures,
                statechain_public_keys: new_node_statechain_public_keys,
                verifying_key: &new_node_tx_verifying_key,
                statechain_commitments: new_node_statechain_commitments,
                self_commitment: &new_node_signing_commitments,
                public_key: &signing_public_key,
                self_signature: &node_user_signature,
                adaptor_public_key: None,
            })
            .await?;

        let refund_signature = self
            .signer
            .aggregate_frost(AggregateFrostRequest {
                message: refund_sighash.as_byte_array(),
                statechain_signatures: new_refund_statechain_signatures,
                statechain_public_keys: new_refund_statechain_public_keys,
                verifying_key: &new_refund_tx_verifying_key,
                statechain_commitments: new_refund_statechain_commitments,
                self_commitment: &new_refund_signing_commitments,
                public_key: &signing_public_key,
                self_signature: &refund_user_signature,
                adaptor_public_key: None,
            })
            .await?;

        let nodes = self
            .coordinator_client
            .finalize_node_signatures(FinalizeNodeSignaturesRequest {
                intent: SignatureIntent::Extend.into(),
                node_signatures: vec![NodeSignatures {
                    node_id: response.leaf_id,
                    node_tx_signature: node_signature.serialize()?.to_vec(),
                    refund_tx_signature: refund_signature.serialize()?.to_vec(),
                }],
            })
            .await?
            .nodes;

        Ok(nodes
            .into_iter()
            .map(|n| n.try_into())
            .collect::<Result<Vec<TreeNode>, _>>()?)
    }

    /// Low-level claim transfer operation
    async fn claim_transfer_with_leaves(
        &self,
        transfer: &Transfer,
        leaves_to_claim: Vec<LeafKeyTweak>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        trace!("Claiming transfer with leaves: {:?}", leaves_to_claim);
        // Check if we need to apply key tweaks first
        let proof_map = if transfer.status == TransferStatus::SenderKeyTweaked {
            Some(
                self.claim_transfer_tweak_keys(transfer, &leaves_to_claim)
                    .await?,
            )
        } else {
            None
        };

        // Sign refunds and get node signatures
        let node_signatures = self
            .claim_transfer_sign_refunds(transfer, &leaves_to_claim, proof_map.as_ref())
            .await?;

        // Finalize the node signatures with the coordinator
        let finalized_nodes = self.finalize_node_signatures(&node_signatures).await?;

        Ok(finalized_nodes)
    }

    /// Claims transfer by applying key tweaks across all operators
    async fn claim_transfer_tweak_keys(
        &self,
        transfer: &Transfer,
        leaves: &[LeafKeyTweak],
    ) -> Result<ProofMap, ServiceError> {
        let (leaves_tweaks_map, proof_map) = self.prepare_claim_leaves_key_tweaks(leaves).await?;

        // Send claim transfer tweak keys to all signing operators in parallel
        let mut tasks = Vec::new();

        for operator_client in &self.operator_clients {
            let leaves_to_receive = leaves_tweaks_map.get(&operator_client.operator.identifier);
            if let Some(leaves_to_receive) = leaves_to_receive {
                let identity_public_key =
                    self.signer.get_identity_public_key()?.serialize().to_vec();
                let leaves_to_receive = leaves_to_receive.clone();

                let task = async move {
                    operator_client
                        .claim_transfer_tweak_keys(
                            operator_rpc::spark::ClaimTransferTweakKeysRequest {
                                transfer_id: transfer.id.to_string(),
                                owner_identity_public_key: identity_public_key,
                                leaves_to_receive,
                            },
                        )
                        .await
                };
                tasks.push(task);
            }
        }

        futures::future::try_join_all(tasks).await?;

        Ok(proof_map)
    }

    /// Prepares claim leaves key tweaks for all operators
    async fn prepare_claim_leaves_key_tweaks(
        &self,
        leaves: &[LeafKeyTweak],
    ) -> Result<
        (
            HashMap<Identifier, Vec<operator_rpc::spark::ClaimLeafKeyTweak>>,
            ProofMap,
        ),
        ServiceError,
    > {
        let mut leaf_data_map = HashMap::new();
        let mut proof_map = HashMap::new();

        for leaf in leaves {
            let (leaf_key_tweaks, proof) = self.prepare_claim_leaf_key_tweaks(leaf).await?;
            proof_map.insert(leaf.node.id.clone(), proof);

            for (identifier, leaf_tweak) in leaf_key_tweaks {
                leaf_data_map
                    .entry(identifier)
                    .or_insert_with(Vec::new)
                    .push(leaf_tweak);
            }
        }

        Ok((leaf_data_map, proof_map))
    }

    /// Prepares claim key tweaks for a single leaf
    async fn prepare_claim_leaf_key_tweaks(
        &self,
        leaf: &LeafKeyTweak,
    ) -> Result<
        (
            HashMap<Identifier, operator_rpc::spark::ClaimLeafKeyTweak>,
            k256::PublicKey,
        ),
        ServiceError,
    > {
        let signing_operators: Vec<_> = self
            .operator_clients
            .iter()
            .map(|c| c.operator.clone())
            .collect();

        // Calculate the public key tweak by subtracting private keys given public keys
        let privkey_tweak = self
            .signer
            .subtract_private_keys(&leaf.signing_key, &leaf.new_signing_key)?;

        // Split the secret into threshold shares with proofs
        let shares = self.signer.split_secret_with_proofs(
            &SecretToSplit::PrivateKey(privkey_tweak),
            self.split_secret_threshold,
            signing_operators.len(),
        )?;

        trace!("prepare claim: Split secret into {} shares", shares.len());

        // Create pubkey shares tweak map
        let mut pubkey_shares_tweak = HashMap::new();
        let secp = Secp256k1::new();
        for operator in &signing_operators {
            let operator_identifier = hex::encode(operator.identifier.serialize());

            let share = find_share(&shares, operator.id).ok_or_else(|| {
                ServiceError::Generic(format!("Share not found for operator {}", operator.id))
            })?;

            let pubkey_tweak = SecretKey::from_slice(&share.secret_share.share.to_bytes())
                .map_err(|_| ServiceError::Generic("Invalid secret share".to_string()))?
                .public_key(&secp);
            pubkey_shares_tweak.insert(operator_identifier, pubkey_tweak.serialize().to_vec());
        }

        trace!("Creating leaf tweaks map for each operator");

        // Create leaf tweaks map for each signing operator
        let mut leaf_tweaks_map = HashMap::new();
        for operator in &signing_operators {
            let share = find_share(&shares, operator.id).ok_or_else(|| {
                ServiceError::Generic(format!("Share not found for operator {}", operator.id))
            })?;

            let claim_leaf_key_tweak = operator_rpc::spark::ClaimLeafKeyTweak {
                leaf_id: leaf.node.id.to_string(),
                secret_share_tweak: Some(operator_rpc::spark::SecretShare {
                    secret_share: share.secret_share.share.to_bytes().to_vec(),
                    proofs: share
                        .proofs
                        .iter()
                        .map(|p| p.to_sec1_bytes().to_vec())
                        .collect(),
                }),
                pubkey_shares_tweak: pubkey_shares_tweak.clone(),
            };

            leaf_tweaks_map.insert(operator.identifier, claim_leaf_key_tweak);
        }

        let proof = shares
            .first()
            .and_then(|s| s.proofs.first())
            .ok_or(ServiceError::Generic("No proof found".to_string()))?;

        Ok((leaf_tweaks_map, proof.clone()))
    }

    /// Claims transfer by signing refunds with the coordinator
    async fn claim_transfer_sign_refunds(
        &self,
        transfer: &Transfer,
        leaf_keys: &[LeafKeyTweak],
        // TODO: do something with proofs? Currently not used in js implementation
        _proof_map: Option<&ProofMap>,
    ) -> Result<Vec<operator_rpc::spark::NodeSignatures>, ServiceError> {
        // Prepare leaf data map with refund signing information
        let mut leaf_data_map = HashMap::new();
        for leaf_key in leaf_keys {
            let signing_nonce_commitment = self.signer.generate_frost_signing_commitments().await?;

            leaf_data_map.insert(
                leaf_key.node.id.clone(),
                LeafRefundSigningData {
                    signing_private_key: leaf_key.new_signing_key.clone(),
                    signing_public_key: self
                        .signer
                        .get_public_key_from_private_key_source(&leaf_key.new_signing_key)?,
                    receiving_public_key: self
                        .signer
                        .get_public_key_from_private_key_source(&leaf_key.new_signing_key)?,
                    tx: leaf_key.node.node_tx.clone(),
                    refund_tx: None,
                    signing_nonce_commitment,
                    vout: leaf_key.node.vout,
                },
            );
        }

        // Prepare refund signing jobs for the coordinator
        let signing_jobs =
            self.prepare_refund_so_signing_jobs(leaf_keys, &mut leaf_data_map, true)?;

        // Call the coordinator to get signing results
        let response = self
            .coordinator_client
            .claim_transfer_sign_refunds(operator_rpc::spark::ClaimTransferSignRefundsRequest {
                transfer_id: transfer.id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()?
                    .serialize()
                    .to_vec(),
                signing_jobs,
            })
            .await?;

        // Sign the refunds using FROST
        let node_signatures = self
            .sign_refunds(
                &leaf_data_map
                    .into_iter()
                    .map(|(key, data)| (key, data.into()))
                    .collect(),
                &response.signing_results,
                None,
            )
            .await?;

        Ok(node_signatures)
    }

    /// Prepares refund signing jobs for claim operations
    fn prepare_refund_so_signing_jobs(
        &self,
        leaves: &[LeafKeyTweak],
        leaf_data_map: &mut HashMap<TreeNodeId, LeafRefundSigningData>,
        is_for_claim: bool,
    ) -> Result<Vec<operator_rpc::spark::LeafRefundTxSigningJob>, ServiceError> {
        let mut signing_jobs = Vec::new();

        for leaf in leaves {
            let refund_signing_data: &mut LeafRefundSigningData =
                leaf_data_map.get_mut(&leaf.node.id).ok_or_else(|| {
                    ServiceError::Generic(format!("Leaf data not found for leaf {}", leaf.node.id))
                })?;

            let old_sequence = leaf
                .node
                .refund_tx
                .as_ref()
                .ok_or(ServiceError::Generic("No refund transaction".to_string()))?
                .input[0]
                .sequence;
            let sequence = if is_for_claim {
                old_sequence // TODO: is this correct?
            } else {
                next_sequence(old_sequence).ok_or(ServiceError::Generic(
                    "Failed to get next sequence".to_string(),
                ))?
            };

            let refund_tx = create_refund_tx(
                sequence,
                bitcoin::OutPoint {
                    txid: leaf.node.node_tx.compute_txid(),
                    vout: 0,
                },
                leaf.node.value,
                &refund_signing_data.receiving_public_key,
                self.network,
            )?;

            let signing_job = operator_rpc::spark::LeafRefundTxSigningJob {
                leaf_id: leaf.node.id.to_string(),
                refund_tx_signing_job: Some(operator_rpc::spark::SigningJob {
                    signing_public_key: refund_signing_data.signing_public_key.serialize().to_vec(),
                    raw_tx: bitcoin::consensus::serialize(&refund_tx),
                    signing_nonce_commitment: Some(
                        refund_signing_data.signing_nonce_commitment.try_into()?,
                    ),
                }),
            };

            refund_signing_data.refund_tx = Some(refund_tx);

            signing_jobs.push(signing_job);
        }

        Ok(signing_jobs)
    }

    /// Signs refund transactions using FROST threshold signatures
    async fn sign_refunds(
        &self,
        leaf_data_map: &HashMap<TreeNodeId, LeafRefundSigningData>,
        operator_signing_results: &[operator_rpc::spark::LeafRefundTxSigningResult],
        adaptor_pubkey: Option<&PublicKey>,
    ) -> Result<Vec<operator_rpc::spark::NodeSignatures>, ServiceError> {
        let mut node_signatures = Vec::new();

        for operator_signing_result in operator_signing_results {
            let leaf_id = TreeNodeId::from_str(&operator_signing_result.leaf_id)
                .map_err(|e| ServiceError::ValidationError(e))?;

            let leaf_data = leaf_data_map.get(&leaf_id).ok_or_else(|| {
                ServiceError::Generic(format!(
                    "Leaf data not found for leaf {}",
                    operator_signing_result.leaf_id
                ))
            })?;

            let refund_tx_signing_result = operator_signing_result
                .refund_tx_signing_result
                .as_ref()
                .ok_or_else(|| {
                    ServiceError::ValidationError("Missing refund tx signing result".to_string())
                })?;

            let refund_tx = leaf_data
                .refund_tx
                .as_ref()
                .ok_or_else(|| ServiceError::Generic("Missing refund transaction".to_string()))?;

            let refund_tx_sighash = sighash_from_tx(refund_tx, 0, &leaf_data.tx.output[0])?;

            // Map operator signing commitments and signature shares
            let signing_nonce_commitments = map_signing_nonce_commitments(
                refund_tx_signing_result.signing_nonce_commitments.clone(),
            )?;
            let signature_shares =
                map_signature_shares(refund_tx_signing_result.signature_shares.clone())?;
            let public_keys = map_public_keys(refund_tx_signing_result.public_keys.clone())?;

            let verifying_key = PublicKey::from_slice(&operator_signing_result.verifying_key)
                .map_err(|_| ServiceError::ValidationError("Invalid verifying key".to_string()))?;

            // Sign with FROST
            let user_signature = self
                .signer
                .sign_frost(SignFrostRequest {
                    message: refund_tx_sighash.as_byte_array(),
                    public_key: &leaf_data.signing_public_key,
                    private_key: &leaf_data.signing_private_key,
                    verifying_key: &verifying_key,
                    self_commitment: &leaf_data.signing_nonce_commitment,
                    statechain_commitments: signing_nonce_commitments.clone(),
                    adaptor_public_key: adaptor_pubkey,
                })
                .await?;

            // Aggregate FROST signatures
            let refund_aggregate = self
                .signer
                .aggregate_frost(AggregateFrostRequest {
                    message: refund_tx_sighash.as_byte_array(),
                    statechain_signatures: signature_shares,
                    statechain_public_keys: public_keys,
                    verifying_key: &verifying_key,
                    statechain_commitments: signing_nonce_commitments,
                    self_commitment: &leaf_data.signing_nonce_commitment,
                    public_key: &leaf_data.signing_public_key,
                    self_signature: &user_signature,
                    adaptor_public_key: adaptor_pubkey,
                })
                .await?;

            node_signatures.push(operator_rpc::spark::NodeSignatures {
                node_id: operator_signing_result.leaf_id.clone(),
                refund_tx_signature: refund_aggregate.serialize()?.to_vec(),
                node_tx_signature: Vec::new(),
            });
        }

        Ok(node_signatures)
    }

    /// Finalizes node signatures with the coordinator
    async fn finalize_node_signatures(
        &self,
        node_signatures: &[operator_rpc::spark::NodeSignatures],
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let response = self
            .coordinator_client
            .finalize_node_signatures(operator_rpc::spark::FinalizeNodeSignaturesRequest {
                intent: operator_rpc::common::SignatureIntent::Transfer as i32,
                node_signatures: node_signatures.to_vec(),
            })
            .await?;

        let nodes = response
            .nodes
            .into_iter()
            .map(|node| node.try_into())
            .collect::<Result<Vec<TreeNode>, _>>()?;

        Ok(nodes)
    }

    pub async fn verify_pending_transfer(
        &self,
        transfer: &Transfer,
    ) -> Result<HashMap<TreeNodeId, PrivateKeySource>, ServiceError> {
        let mut leaf_key_map = HashMap::new();
        let secp = bitcoin::secp256k1::Secp256k1::new();

        for transfer_leaf in &transfer.leaves {
            // Build the payload: leaf_id + transfer_id + secret_cipher
            let leaf_id_string = transfer_leaf.leaf.id.to_string();
            let transfer_id_string = transfer.id.to_string();

            let mut payload = Vec::new();
            payload.extend_from_slice(leaf_id_string.as_bytes());
            payload.extend_from_slice(transfer_id_string.as_bytes());
            payload.extend_from_slice(&transfer_leaf.secret_cipher);

            // Hash the payload
            let digest = sha256::Hash::hash(&payload);
            let message = bitcoin::secp256k1::Message::from_digest(digest.to_byte_array());

            let signature = match transfer_leaf.signature {
                Some(signature) => signature,
                None => {
                    return Err(ServiceError::Generic(
                        "Transfer leaf signature is missing".to_string(),
                    ));
                }
            };
            // Verify the signature (signature is already a Signature type in TransferLeaf)
            secp.verify_ecdsa(&message, &signature, &transfer.sender_identity_public_key)
                .map_err(|e| ServiceError::SignatureVerificationFailed(e.to_string()))?;

            // Decrypt the secret cipher and get the corresponding public key
            // The signer persists the private key internally and returns the public key
            let private_key = PrivateKeySource::new_encrypted(transfer_leaf.secret_cipher.clone());

            leaf_key_map.insert(transfer_leaf.leaf.id.clone(), private_key);
        }

        Ok(leaf_key_map)
    }

    pub async fn transfer_leaves_to_self(
        &self,
        leaves: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let leaf_key_tweaks = leaves
            .iter()
            .map(|leaf| {
                let current_signing_key =
                    PrivateKeySource::Derived(leaf.parent_node_id.clone().ok_or(
                        ServiceError::Generic("Leaf has no parent node id".to_string()),
                    )?);
                let ephemeral_key = self.signer.generate_random_key()?;

                Ok(LeafKeyTweak {
                    node: leaf.clone(),
                    signing_key: current_signing_key,
                    new_signing_key: ephemeral_key,
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        let transfer = self
            .send_transfer_with_key_tweaks(
                &leaf_key_tweaks,
                &self.signer.get_identity_public_key()?,
            )
            .await?;

        let pending_transfer =
            self.query_transfer(&transfer.id)
                .await?
                .ok_or(ServiceError::Generic(
                    "Pending transfer not found".to_string(),
                ))?;

        let resulting_nodes = self.claim_transfer(&pending_transfer, None).await?;

        Ok(resulting_nodes)
    }

    /// Queries all transfers for the current identity
    ///
    /// By default, returns the first 100 transfers
    pub async fn query_all_transfers(
        &self,
        paging: &PagingFilter,
    ) -> Result<Vec<Transfer>, ServiceError> {
        trace!(
            "Querying all transfers with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let response = self
            .coordinator_client
            .query_all_transfers(TransferFilter {
                participant: Some(Participant::SenderOrReceiverIdentityPublicKey(
                    self.signer.get_identity_public_key()?.serialize().to_vec(),
                )),
                network: self.network.to_proto_network() as i32,
                limit: paging.limit as i64,
                offset: paging.offset as i64,
                types: vec![
                    operator_rpc::spark::TransferType::Transfer.into(),
                    operator_rpc::spark::TransferType::PreimageSwap.into(),
                    operator_rpc::spark::TransferType::CooperativeExit.into(),
                    operator_rpc::spark::TransferType::UtxoSwap.into(),
                ],
                ..Default::default()
            })
            .await?;

        Ok(response
            .transfers
            .into_iter()
            .map(|t| t.try_into())
            .collect::<Result<Vec<Transfer>, _>>()?)
    }

    /// Queries pending transfers from the operator
    pub async fn query_pending_transfers(
        &self,
        paging: &PagingFilter,
    ) -> Result<Vec<Transfer>, ServiceError> {
        trace!("Querying all pending transfers");
        let response = self
            .coordinator_client
            .query_pending_transfers(operator_rpc::spark::TransferFilter {
                participant: Some(Participant::SenderOrReceiverIdentityPublicKey(
                    self.signer.get_identity_public_key()?.serialize().to_vec(),
                )),
                offset: paging.offset as i64,
                limit: paging.limit as i64,
                network: self.network.to_proto_network() as i32,
                ..Default::default()
            })
            .await?;

        Ok(response
            .transfers
            .into_iter()
            .map(|t| t.try_into())
            .collect::<Result<Vec<Transfer>, _>>()?)
    }

    /// Queries pending transfers from the operator
    pub async fn query_pending_receiver_transfers(
        &self,
        paging: &PagingFilter,
    ) -> Result<Vec<Transfer>, ServiceError> {
        trace!("Querying all pending receiver transfers");
        let response = self
            .coordinator_client
            .query_pending_transfers(operator_rpc::spark::TransferFilter {
                network: self.network.to_proto_network() as i32,
                participant: Some(Participant::ReceiverIdentityPublicKey(
                    self.signer.get_identity_public_key()?.serialize().to_vec(),
                )),
                offset: paging.offset as i64,
                limit: paging.limit as i64,
                ..Default::default()
            })
            .await?;

        Ok(response
            .transfers
            .into_iter()
            .map(|t| t.try_into())
            .collect::<Result<Vec<Transfer>, _>>()?)
    }

    pub async fn query_transfer(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<Transfer>, ServiceError> {
        trace!("Querying transfer with id: {}", transfer_id);
        let response = self
            .coordinator_client
            .query_all_transfers(TransferFilter {
                participant: Some(Participant::SenderOrReceiverIdentityPublicKey(
                    self.signer.get_identity_public_key()?.serialize().to_vec(),
                )),
                transfer_ids: vec![transfer_id.to_string()],
                network: self.network.to_proto_network() as i32,
                ..Default::default()
            })
            .await?;

        match response.transfers.first() {
            Some(transfer) => {
                let transfer = transfer.clone().try_into()?;
                Ok(Some(transfer))
            }
            None => Ok(None),
        }
    }

    pub async fn deliver_transfer_package(
        &self,
        transfer: &Transfer,
        leaves: &Vec<LeafKeyTweak>,
        refund_signature_map: HashMap<TreeNodeId, Signature>,
    ) -> Result<Transfer, ServiceError> {
        let key_tweak_input_map = self
            .prepare_send_transfer_key_tweaks(
                &transfer.id,
                &transfer.receiver_identity_public_key,
                &leaves,
                refund_signature_map,
            )
            .await?;

        let transfer_package = self
            .prepare_transfer_package(
                &transfer.id,
                key_tweak_input_map,
                &leaves,
                &transfer.receiver_identity_public_key,
            )
            .await?;

        let response = self
            .coordinator_client
            .finalize_transfer(
                operator_rpc::spark::FinalizeTransferWithTransferPackageRequest {
                    transfer_id: transfer.id.to_string(),
                    owner_identity_public_key: self
                        .signer
                        .get_identity_public_key()?
                        .serialize()
                        .to_vec(),
                    transfer_package: Some(transfer_package),
                },
            )
            .await?;

        match response.transfer {
            Some(transfer) => Ok(transfer.try_into()?),
            None => Err(ServiceError::ServiceConnectionError(
                OperatorRpcError::Unexpected("No transfer response from operator".to_string()),
            )),
        }
    }
}

fn find_share(
    shares: &[VerifiableSecretShare],
    operator_id: usize,
) -> Option<&VerifiableSecretShare> {
    let target_share_index = Scalar::from((operator_id + 1) as u64);

    for share in shares {
        if share.secret_share.index == target_share_index {
            return Some(share);
        }
    }

    trace!(
        "Found no share for operator id: {}, shares: {:?}",
        operator_id, shares
    );
    None
}
