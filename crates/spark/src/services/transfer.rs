use std::time::Duration;
use std::{collections::HashMap, sync::Arc};

use crate::Network;
use crate::operator::OperatorPool;
use crate::operator::rpc::spark::TransferFilter;
use crate::operator::rpc::spark::transfer_filter::Participant;
use crate::operator::rpc::{self as operator_rpc, OperatorRpcError};
use crate::services::models::{LeafKeyTweak, Transfer, map_signing_nonce_commitments};
use crate::services::{ProofMap, TransferId, TransferStatus};
use crate::signer::{
    FrostSigningCommitmentsWithNonces, PrivateKeySource, SecretToSplit, VerifiableSecretShare,
};
use crate::utils::leaf_key_tweak::prepare_leaf_key_tweaks_to_send;
use crate::utils::paging::{PagingFilter, PagingResult, pager};
use crate::utils::refund::{
    RefundSignatures, SignedRefundTransactions, prepare_refund_so_signing_jobs,
    sign_aggregate_refunds, sign_refunds,
};

use bitcoin::Transaction;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::key::Secp256k1;
use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::secp256k1::{PublicKey, SecretKey};
use frost_secp256k1_tr::Identifier;
use k256::Scalar;
use prost::Message as ProstMessage;
use tokio_with_wasm::alias as tokio;
use tonic::Code;
use tracing::{debug, error, trace};

use crate::{
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
    pub direct_tx: Option<Transaction>,
    pub refund_tx: Option<Transaction>,
    pub direct_refund_tx: Option<Transaction>,
    pub direct_from_cpfp_refund_tx: Option<Transaction>,
    pub signing_nonce_commitment: FrostSigningCommitmentsWithNonces,
    pub direct_signing_nonce_commitment: FrostSigningCommitmentsWithNonces,
    pub direct_from_cpfp_signing_nonce_commitment: FrostSigningCommitmentsWithNonces,
    pub vout: u32,
}

/// Configuration for claiming transfers
pub struct ClaimTransferConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for ClaimTransferConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay_ms: 1000,
            max_delay_ms: 10000,
        }
    }
}

pub struct TransferService<S> {
    signer: Arc<S>,
    network: Network,
    split_secret_threshold: u32,
    operator_pool: Arc<OperatorPool<S>>,
}

impl<S: Signer> TransferService<S> {
    pub fn new(
        signer: Arc<S>,
        network: Network,
        split_secret_threshold: u32,
        operator_pool: Arc<OperatorPool<S>>,
    ) -> Self {
        Self {
            signer,
            network,
            split_secret_threshold,
            operator_pool,
        }
    }

    /// Creates and initiates a new transfer with the given leaves.
    ///
    /// Generates a transfer package containing encrypted key material, refund signatures,
    /// and proofs that are distributed to the statechain operators.
    pub async fn transfer_leaves_to(
        &self,
        leaves: Vec<TreeNode>,
        receiver_id: &PublicKey,
        signing_key_source: Option<PrivateKeySource>,
    ) -> Result<Transfer, ServiceError> {
        // build leaf key tweaks with new signing keys that we will send to the receiver
        let leaf_key_tweaks =
            prepare_leaf_key_tweaks_to_send(&self.signer, leaves, signing_key_source)?;
        let transfer = self
            .send_transfer_with_key_tweaks(&leaf_key_tweaks, receiver_id)
            .await?;

        Ok(transfer)
    }

    pub async fn transfer_leaves_to_self(
        &self,
        leaves: Vec<TreeNode>,
        signing_key_source: Option<PrivateKeySource>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let transfer = self
            .transfer_leaves_to(
                leaves,
                &self.signer.get_identity_public_key()?,
                signing_key_source,
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
                leaf_key_tweaks,
                Default::default(),
            )
            .await?;

        let transfer_package = self
            .prepare_transfer_package(
                &transfer_id,
                key_tweak_input_map,
                leaf_key_tweaks,
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
            .operator_pool
            .get_coordinator()
            .client
            .start_transfer_v2(start_transfer_request)
            .await?
            .transfer
            .ok_or(ServiceError::Generic(
                "No transfer from operator".to_string(),
            ))?;

        transfer.try_into()
    }

    async fn prepare_send_transfer_key_tweaks(
        &self,
        transfer_id: &TransferId,
        receiver_public_key: &PublicKey,
        leaves: &[LeafKeyTweak],
        refund_signatures: RefundSignatures,
    ) -> Result<HashMap<Identifier, Vec<operator_rpc::spark::SendLeafKeyTweak>>, ServiceError> {
        let mut leaves_tweaks_map = HashMap::new();

        for leaf in leaves {
            let cpfp_refund_signature = refund_signatures
                .cpfp_signatures
                .get(&leaf.node.id)
                .cloned();
            let direct_refund_signature = refund_signatures
                .direct_signatures
                .get(&leaf.node.id)
                .cloned();
            let direct_from_cpfp_refund_signature = refund_signatures
                .direct_from_cpfp_signatures
                .get(&leaf.node.id)
                .cloned();

            let leaf_tweaks_map = self
                .prepare_single_send_transfer_key_tweak(
                    transfer_id,
                    leaf,
                    receiver_public_key,
                    cpfp_refund_signature,
                    direct_refund_signature,
                    direct_from_cpfp_refund_signature,
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
        cpfp_refund_signature: Option<Signature>,
        direct_refund_signature: Option<Signature>,
        direct_from_cpfp_refund_signature: Option<Signature>,
    ) -> Result<HashMap<Identifier, operator_rpc::spark::SendLeafKeyTweak>, ServiceError> {
        // Calculate the key tweak by subtracting keys
        let privkey_tweak = self
            .signer
            .subtract_private_keys(&leaf.signing_key, &leaf.new_signing_key)?;

        // Split the secret into threshold shares with proofs
        let shares = self.signer.split_secret_with_proofs(
            &SecretToSplit::PrivateKey(privkey_tweak),
            self.split_secret_threshold,
            self.operator_pool.len(),
        )?;

        trace!(
            "prepare transfer: Split secret into {} shares",
            shares.len()
        );

        // TODO: move secp to a field of self to avoid creating it every time
        let secp = bitcoin::secp256k1::Secp256k1::new();
        // Create pubkey shares tweak map
        let mut pubkey_shares_tweak = HashMap::new();
        for operator in self.operator_pool.get_all_operators() {
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

        for operator in self.operator_pool.get_all_operators() {
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
                refund_signature: cpfp_refund_signature
                    .map(|s| s.serialize_compact().to_vec())
                    .unwrap_or_default(),
                direct_refund_signature: direct_refund_signature
                    .map(|s| s.serialize_compact().to_vec())
                    .unwrap_or_default(),
                direct_from_cpfp_refund_signature: direct_from_cpfp_refund_signature
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
            .operator_pool
            .get_coordinator()
            .client
            .get_signing_commitments(operator_rpc::spark::GetSigningCommitmentsRequest {
                node_ids: leaf_key_tweaks
                    .iter()
                    .map(|l| l.node.id.to_string())
                    .collect(),
                count: 3,
            })
            .await?
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(&sc.signing_nonce_commitments))
            .collect::<Result<Vec<_>, _>>()?;

        let chunked_signing_commitments = signing_commitments
            .chunks(leaf_key_tweaks.len())
            .collect::<Vec<_>>();

        if chunked_signing_commitments.len() != 3 {
            return Err(ServiceError::SSPswapError(
                "Not enough signing commitments returned".to_string(),
            ));
        }

        let cpfp_signing_commitments = chunked_signing_commitments[0].to_vec();
        let direct_signing_commitments = chunked_signing_commitments[1].to_vec();
        let direct_from_cpfp_signing_commitments = chunked_signing_commitments[2].to_vec();

        let SignedRefundTransactions {
            cpfp_signed_tx,
            direct_signed_tx,
            direct_from_cpfp_signed_tx,
        } = sign_refunds(
            &self.signer,
            leaf_key_tweaks,
            cpfp_signing_commitments,
            direct_signing_commitments,
            direct_from_cpfp_signing_commitments,
            receiver_public_key,
            self.network,
        )
        .await?;

        let encrypted_key_tweaks = self.encrypt_key_tweaks(&key_tweak_input_map)?;

        let unsigned_transfer_package = operator_rpc::spark::TransferPackage {
            leaves_to_send: cpfp_signed_tx
                .into_iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            direct_leaves_to_send: direct_signed_tx
                .into_iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            direct_from_cpfp_leaves_to_send: direct_from_cpfp_signed_tx
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
                .operator_pool
                .get_operator_by_identifier(key)
                .ok_or_else(|| ServiceError::Generic("Operator not found".to_string()))?;

            // Encrypt the binary data using the operator's identity public key
            let encrypted_proto = self.encrypt_with_public_key(
                &operator_client.identity_public_key,
                &proto_to_encrypt_binary,
            )?;

            encrypted_key_tweaks.insert(*key, encrypted_proto);
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
            .map_err(|e| ServiceError::Generic(format!("ECIES encryption failed: {e}")))
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
            .map_err(ServiceError::SignerError)?;

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
        let transfer_id_bytes = hex::decode(transfer_id.to_string().replace("-", "")).unwrap();
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
                    if let ServiceError::TransferAlreadyClaimed = e {
                        return Err(e);
                    }

                    error!("Failed to claim transfer with leaves: {}", e);
                    retry_count += 1;
                    continue;
                }
            };

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
                    .await
                    .map_err(|e| {
                        debug!("Failed to claim transfer tweak keys: {}", e);
                        e
                    })?,
            )
        } else {
            None
        };

        // Sign refunds and get node signatures
        let node_signatures = self
            .claim_transfer_sign_refunds(transfer, &leaves_to_claim, proof_map.as_ref())
            .await
            .map_err(|e| {
                debug!("Failed to claim transfer sign refunds: {}", e);
                e
            })?;

        // Finalize the node signatures with the coordinator
        let finalized_nodes = self
            .finalize_node_signatures(&node_signatures)
            .await
            .map_err(|e| {
                debug!("Failed to finalize node signatures: {}", e);
                e
            })?;

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

        for operator in self.operator_pool.get_all_operators() {
            let leaves_to_receive = leaves_tweaks_map.get(&operator.identifier);
            if let Some(leaves_to_receive) = leaves_to_receive {
                let identity_public_key =
                    self.signer.get_identity_public_key()?.serialize().to_vec();
                let leaves_to_receive = leaves_to_receive.clone();

                let task = async move {
                    operator
                        .client
                        .claim_transfer_tweak_keys(
                            operator_rpc::spark::ClaimTransferTweakKeysRequest {
                                transfer_id: transfer.id.to_string(),
                                owner_identity_public_key: identity_public_key,
                                leaves_to_receive,
                            },
                        )
                        .await
                        .map_err(|e| {
                            if let OperatorRpcError::Connection(status) = &e
                                && status.code() == Code::AlreadyExists
                            {
                                return ServiceError::TransferAlreadyClaimed;
                            }

                            e.into()
                        })
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
        // Calculate the public key tweak by subtracting private keys given public keys
        let privkey_tweak = self
            .signer
            .subtract_private_keys(&leaf.signing_key, &leaf.new_signing_key)?;

        // Split the secret into threshold shares with proofs
        let shares = self.signer.split_secret_with_proofs(
            &SecretToSplit::PrivateKey(privkey_tweak),
            self.split_secret_threshold,
            self.operator_pool.len(),
        )?;

        trace!("prepare claim: Split secret into {} shares", shares.len());

        // Create pubkey shares tweak map
        let mut pubkey_shares_tweak = HashMap::new();
        let secp = Secp256k1::new();
        for operator in self.operator_pool.get_all_operators() {
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
        for operator in self.operator_pool.get_all_operators() {
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

        Ok((leaf_tweaks_map, *proof))
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
            let direct_signing_nonce_commitment =
                self.signer.generate_frost_signing_commitments().await?;
            let direct_from_cpfp_signing_nonce_commitment =
                self.signer.generate_frost_signing_commitments().await?;

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
                    direct_tx: leaf_key.node.direct_tx.clone(),
                    refund_tx: None,
                    direct_refund_tx: None,
                    direct_from_cpfp_refund_tx: None,
                    signing_nonce_commitment,
                    direct_signing_nonce_commitment,
                    direct_from_cpfp_signing_nonce_commitment,
                    vout: leaf_key.node.vout,
                },
            );
        }

        // Prepare refund signing jobs for the coordinator
        let signing_jobs =
            prepare_refund_so_signing_jobs(self.network, leaf_keys, &mut leaf_data_map)?;

        // Call the coordinator to get signing results
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .claim_transfer_sign_refunds_v2(operator_rpc::spark::ClaimTransferSignRefundsRequest {
                transfer_id: transfer.id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()?
                    .serialize()
                    .to_vec(),
                signing_jobs,
            })
            .await
            .map_err(|e| {
                if let OperatorRpcError::Connection(status) = &e
                    && status.code() == Code::AlreadyExists
                {
                    return ServiceError::TransferAlreadyClaimed;
                }

                e.into()
            })?;

        // Sign the refunds using FROST
        let node_signatures = sign_aggregate_refunds(
            &self.signer,
            &leaf_data_map.into_iter().collect(),
            &response.signing_results,
            None,
            None,
            None,
        )
        .await?;

        Ok(node_signatures)
    }

    /// Finalizes node signatures with the coordinator
    async fn finalize_node_signatures(
        &self,
        node_signatures: &[operator_rpc::spark::NodeSignatures],
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .finalize_node_signatures_v2(operator_rpc::spark::FinalizeNodeSignaturesRequest {
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

    async fn query_transfers_inner(
        &self,
        paging: PagingFilter,
        transfer_ids: Option<Vec<String>>,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        trace!(
            "Querying transfers with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let order: crate::operator::rpc::spark::Order = paging.order.clone().into();
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .query_all_transfers(TransferFilter {
                order: order.into(),
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
                transfer_ids: transfer_ids.unwrap_or_default(),
                ..Default::default()
            })
            .await?;

        Ok(PagingResult {
            items: resp
                .transfers
                .into_iter()
                .map(|t| t.try_into())
                .collect::<Result<Vec<Transfer>, _>>()?,
            next: paging.next_from_offset(resp.offset),
        })
    }

    /// Queries transfers for the current identity
    pub async fn query_transfers(
        &self,
        paging: Option<PagingFilter>,
        transfer_ids: Option<Vec<String>>,
    ) -> Result<Vec<Transfer>, ServiceError> {
        let transfers = match paging {
            Some(paging) => {
                self.query_transfers_inner(paging, transfer_ids)
                    .await?
                    .items
            }
            None => {
                pager(
                    |f| self.query_transfers_inner(f, transfer_ids.clone()),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(transfers)
    }

    async fn query_pending_transfers_inner(
        &self,
        paging: PagingFilter,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        trace!(
            "Querying pending transfers with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
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

        Ok(PagingResult {
            items: resp
                .transfers
                .into_iter()
                .map(|t| t.try_into())
                .collect::<Result<Vec<Transfer>, _>>()?,
            next: paging.next_from_offset(resp.offset),
        })
    }

    /// Queries pending transfers from the operator
    pub async fn query_pending_transfers(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<Vec<Transfer>, ServiceError> {
        let transfers = match paging {
            Some(paging) => self.query_pending_transfers_inner(paging).await?.items,
            None => {
                pager(
                    |f| self.query_pending_transfers_inner(f),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(transfers)
    }

    async fn query_pending_receiver_transfers_inner(
        &self,
        paging: PagingFilter,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        trace!(
            "Querying pending receiver transfers with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
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

        Ok(PagingResult {
            items: resp
                .transfers
                .into_iter()
                .map(|t| t.try_into())
                .collect::<Result<Vec<Transfer>, _>>()?,
            next: paging.next_from_offset(resp.offset),
        })
    }

    /// Queries pending transfers from the operator
    pub async fn query_pending_receiver_transfers(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<Vec<Transfer>, ServiceError> {
        let transfers = match paging {
            Some(paging) => {
                self.query_pending_receiver_transfers_inner(paging)
                    .await?
                    .items
            }
            None => {
                pager(
                    |f| self.query_pending_receiver_transfers_inner(f),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(transfers)
    }

    pub async fn query_transfer(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<Transfer>, ServiceError> {
        trace!("Querying transfer with id: {}", transfer_id);
        let response = self
            .operator_pool
            .get_coordinator()
            .client
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
        leaves: &[LeafKeyTweak],
        refund_signatures: RefundSignatures,
    ) -> Result<Transfer, ServiceError> {
        let key_tweak_input_map = self
            .prepare_send_transfer_key_tweaks(
                &transfer.id,
                &transfer.receiver_identity_public_key,
                leaves,
                refund_signatures,
            )
            .await?;

        let transfer_package = self
            .prepare_transfer_package(
                &transfer.id,
                key_tweak_input_map,
                leaves,
                &transfer.receiver_identity_public_key,
            )
            .await?;

        let response = self
            .operator_pool
            .get_coordinator()
            .client
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
            None => Err(ServiceError::Generic(
                "No transfer response from operator".to_string(),
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
