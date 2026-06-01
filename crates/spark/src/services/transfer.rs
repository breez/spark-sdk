use std::time::Duration;
use std::{collections::HashMap, sync::Arc};

use crate::Network;
use crate::address::SparkAddress;
use crate::operator::OperatorPool;
use crate::operator::rpc::spark::transfer_filter::Participant;
use crate::operator::rpc::spark::{HashVariant, StartTransferRequest, TransferFilter};
use crate::operator::rpc::{self as operator_rpc, OperatorRpcError};
use crate::services::models::{
    LeafKeyTweak, Transfer, map_signing_nonce_commitments, split_signing_commitments_by_variant,
};
use crate::services::{TransferId, TransferObserver, TransferStatus};
use crate::signer::{
    FrostSigningCommitmentsWithNonces, SecretSource, SecretToSplit, VerifiableSecretShare,
};
use crate::utils::leaf_key_tweak::prepare_leaf_key_tweaks_to_send;
use crate::utils::paging::{PagingFilter, PagingResult, pager};
use crate::utils::refund::{
    RefundSignatures, SignRefundsParams, SignedRefundTransactions, prepare_refund_so_signing_jobs,
    sign_refunds,
};
use crate::utils::tagged_hasher::TaggedHasher;
use crate::utils::time::web_time_to_prost_timestamp;

use bitcoin::Transaction;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::secp256k1::{PublicKey, SecretKey};
use frost_secp256k1_tr::Identifier;
use k256::Scalar;
use platform_utils::time::SystemTime;
use platform_utils::tokio;
use prost::Message as ProstMessage;
use tracing::{debug, error, trace, warn};

use crate::{
    bitcoin::sighash_from_tx,
    signer::{
        ClaimLeafInput, FrostDerivation, FrostJob, OperatorRecipient, PrepareClaimRequest,
        PrepareTransferRequest, Signer, SparkSigner, TransferLeafInput,
    },
    tree::{TreeNode, TreeNodeId},
};

use super::ServiceError;
use super::models::{PreparedTransferRequest, SignedTx};

/// Result of preparing a transfer package, containing both the package and the signed transaction data
pub(crate) struct PreparedTransferPackage {
    pub transfer_package: operator_rpc::spark::TransferPackage,
    pub cpfp_signed_txs: Vec<SignedTx>,
}

/// Helper struct for leaf refund signing data
#[derive(Debug, Clone)]
pub struct LeafRefundSigningData {
    pub signing_private_key: SecretSource,
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
    /// For coop exit signing: the connector transaction output to use as prev_out
    pub connector_prev_out: Option<bitcoin::TxOut>,
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

pub struct TransferService {
    signer: Arc<dyn Signer>,
    spark_signer: Arc<dyn SparkSigner>,
    network: Network,
    split_secret_threshold: u32,
    operator_pool: Arc<OperatorPool>,
    transfer_observer: Option<Arc<dyn TransferObserver>>,
}

impl TransferService {
    pub fn new(
        signer: Arc<dyn Signer>,
        spark_signer: Arc<dyn SparkSigner>,
        network: Network,
        split_secret_threshold: u32,
        operator_pool: Arc<OperatorPool>,
        transfer_observer: Option<Arc<dyn TransferObserver>>,
    ) -> Self {
        Self {
            signer,
            spark_signer,
            network,
            split_secret_threshold,
            operator_pool,
            transfer_observer,
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
        transfer_id: Option<TransferId>,
        signing_key_source: Option<SecretSource>,
        spark_invoice: Option<String>,
    ) -> Result<Transfer, ServiceError> {
        let unwrapped_transfer_id = match &transfer_id {
            Some(transfer_id) => transfer_id.clone(),
            None => TransferId::generate(),
        };

        if let Some(transfer_observer) = &self.transfer_observer {
            let identity_public_key = &self.signer.get_identity_public_key().await?;
            if identity_public_key != receiver_id {
                let receiver_address = SparkAddress::new(*receiver_id, self.network, None);
                let amount_sats: u64 = leaves.iter().map(|l| l.value).sum();
                transfer_observer
                    .before_send_transfer(
                        &unwrapped_transfer_id,
                        &spark_invoice
                            .clone()
                            .or(receiver_address.to_address_string().ok())
                            .ok_or(ServiceError::Generic(
                                "No pay request available".to_string(),
                            ))?,
                        amount_sats,
                    )
                    .await?;
            }
        }

        // build leaf key tweaks with new signing keys that we will send to the receiver
        let leaf_key_tweaks =
            prepare_leaf_key_tweaks_to_send(&self.signer, leaves, signing_key_source).await?;
        let transfer_res = self
            .send_transfer_with_key_tweaks(
                &unwrapped_transfer_id,
                &leaf_key_tweaks,
                receiver_id,
                spark_invoice,
            )
            .await;
        let transfer = match (&transfer_id, transfer_res) {
            (_, Ok(t)) => t,
            (Some(transfer_id), Err(e)) => {
                return self
                    .recover_transfer_on_rpc_connection_error(transfer_id, e)
                    .await;
            }
            (None, Err(e)) => return Err(e),
        };

        Ok(transfer)
    }

    pub(crate) async fn recover_transfer_on_rpc_connection_error(
        &self,
        transfer_id: &TransferId,
        error: ServiceError,
    ) -> Result<Transfer, ServiceError> {
        if let ServiceError::ServiceConnectionError(operator_rpc_error) = &error
            && let OperatorRpcError::Connection(status) = operator_rpc_error.as_ref()
            && matches!(
                status.code(),
                tonic::Code::Internal | tonic::Code::AlreadyExists
            )
        {
            // There was an RPC connection error. Check if the transfer already exists remotely.
            let operator_transfers = self
                .operator_pool
                .get_coordinator()
                .client
                .query_all_transfers(TransferFilter {
                    transfer_ids: vec![transfer_id.to_string()],
                    network: self.network.to_proto_network() as i32,
                    participant: Some(Participant::SenderIdentityPublicKey(
                        self.signer
                            .get_identity_public_key()
                            .await?
                            .serialize()
                            .to_vec(),
                    )),
                    ..Default::default()
                })
                .await?;
            if let Some(transfer) = operator_transfers.transfers.into_iter().nth(0) {
                debug!("Recovered transfer {} after connection error", transfer.id);

                return transfer.try_into();
            }
        }

        Err(error)
    }

    pub async fn send_transfer_with_key_tweaks(
        &self,
        transfer_id: &TransferId,
        leaf_key_tweaks: &[LeafKeyTweak],
        receiver_id: &PublicKey,
        spark_invoice: Option<String>,
    ) -> Result<Transfer, ServiceError> {
        let prepared_package = self
            .prepare_transfer_package(
                transfer_id,
                leaf_key_tweaks,
                receiver_id,
                None,
                None, // No adaptor public key for regular transfers
            )
            .await?;

        // Make request to start transfer
        let start_transfer_request = operator_rpc::spark::StartTransferRequest {
            transfer_id: transfer_id.to_string(),
            owner_identity_public_key: self
                .signer
                .get_identity_public_key()
                .await?
                .serialize()
                .to_vec(),
            receiver_identity_public_key: receiver_id.serialize().to_vec(),
            transfer_package: Some(prepared_package.transfer_package),
            spark_invoice: spark_invoice.unwrap_or_default(),
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

    pub(crate) async fn prepare_send_transfer_key_tweaks(
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
            .subtract_secrets(&leaf.signing_key, &leaf.new_signing_key)
            .await?;

        // Split the secret into threshold shares with proofs
        let shares = self
            .signer
            .split_secret_with_proofs(
                &SecretToSplit::SecretSource(privkey_tweak),
                self.split_secret_threshold,
                self.operator_pool.len(),
            )
            .await?;

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
            SecretSource::Derived(_) => {
                return Err(ServiceError::Generic(
                    "Trying to share derived private key".to_string(),
                ));
            }
            SecretSource::Encrypted(private_key) => {
                self.signer
                    .encrypt_secret_for_receiver(private_key, receiver_public_key)
                    .await?
            }
        };

        // Create the signing payload: leaf_id || transfer_id || secret_cipher
        let mut payload = Vec::new();
        payload.extend_from_slice(leaf.node.id.to_string().as_bytes());
        payload.extend_from_slice(transfer_id.to_string().as_bytes());
        payload.extend_from_slice(&secret_cipher);

        // Sign the hash with identity key
        let signature = self
            .signer
            .sign_message_ecdsa_with_identity_key(&payload)
            .await?;

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

    pub(crate) async fn prepare_transfer_package(
        &self,
        transfer_id: &TransferId,
        leaf_key_tweaks: &[LeafKeyTweak],
        receiver_public_key: &PublicKey,
        payment_hash: Option<&sha256::Hash>,
        cpfp_adaptor_public_key: Option<&PublicKey>,
    ) -> Result<PreparedTransferPackage, ServiceError> {
        if leaf_key_tweaks.is_empty() {
            return Err(ServiceError::InvalidInput(
                "prepare_transfer_package requires at least one leaf".to_string(),
            ));
        }

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
                node_id_count: 0,
            })
            .await?
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(&sc.signing_nonce_commitments))
            .collect::<Result<Vec<_>, _>>()?;

        let [cpfp_chunk, direct_chunk, direct_from_cpfp_chunk] =
            split_signing_commitments_by_variant(&signing_commitments, leaf_key_tweaks.len())?;
        let cpfp_signing_commitments = cpfp_chunk.to_vec();
        let direct_signing_commitments = direct_chunk.to_vec();
        let direct_from_cpfp_signing_commitments = direct_from_cpfp_chunk.to_vec();

        // Refund signing (operator-commits-first) with the old leaf key.
        let SignedRefundTransactions {
            cpfp_signed_tx,
            direct_signed_tx,
            direct_from_cpfp_signed_tx,
        } = sign_refunds(SignRefundsParams {
            spark_signer: &self.spark_signer,
            leaves: leaf_key_tweaks,
            cpfp_signing_commitments,
            direct_signing_commitments,
            direct_from_cpfp_signing_commitments,
            receiver_pubkey: receiver_public_key,
            payment_hash,
            network: self.network,
            cpfp_adaptor_public_key,
        })
        .await?;

        // Key-tweak / Feldman-split / ECIES / transfer-payload signing. The
        // signer generates the new receiver key and produces the per-operator
        // encrypted key-tweak packages plus the transfer-package signature.
        let prepared = self
            .spark_signer
            .prepare_transfer(PrepareTransferRequest {
                transfer_id: transfer_id.clone(),
                receiver_public_key: *receiver_public_key,
                leaves: leaf_key_tweaks
                    .iter()
                    .map(|l| TransferLeafInput {
                        node: l.node.clone(),
                    })
                    .collect(),
                operator_recipients: self.operator_recipients(),
                threshold: self.split_secret_threshold,
            })
            .await?;

        let transfer_package = operator_rpc::spark::TransferPackage {
            leaves_to_send: cpfp_signed_tx
                .iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            direct_leaves_to_send: direct_signed_tx
                .iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            direct_from_cpfp_leaves_to_send: direct_from_cpfp_signed_tx
                .iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            key_tweak_package: prepared
                .operator_packages
                .into_iter()
                .map(|p| {
                    (
                        hex::encode(p.operator_identifier.serialize()),
                        p.encrypted_package,
                    )
                })
                .collect(),
            user_signature: prepared.transfer_user_signature.serialize_der().to_vec(),
            hash_variant: HashVariant::V2.into(),
        };

        Ok(PreparedTransferPackage {
            transfer_package,
            cpfp_signed_txs: cpfp_signed_tx,
        })
    }

    /// Builds the operator-recipient list for share-encryption from the pool.
    fn operator_recipients(&self) -> Vec<OperatorRecipient> {
        self.operator_pool
            .get_all_operators()
            .map(|op| OperatorRecipient {
                id: op.id,
                identifier: op.identifier,
                public_key: op.identity_public_key,
            })
            .collect()
    }

    pub async fn prepare_transfer_request(
        &self,
        transfer_id: &TransferId,
        leaves: &[LeafKeyTweak],
        receiver_public_key: &PublicKey,
        payment_hash: Option<&sha256::Hash>,
        expiry_time: Option<SystemTime>,
        cpfp_adaptor_public_key: Option<&PublicKey>,
    ) -> Result<PreparedTransferRequest, ServiceError> {
        let prepared_package = self
            .prepare_transfer_package(
                transfer_id,
                leaves,
                receiver_public_key,
                payment_hash,
                cpfp_adaptor_public_key,
            )
            .await?;

        Ok(PreparedTransferRequest {
            transfer_request: StartTransferRequest {
                transfer_id: transfer_id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()
                    .await?
                    .serialize()
                    .to_vec(),
                receiver_identity_public_key: receiver_public_key.serialize().to_vec(),
                expiry_time: expiry_time
                    .map(|t| web_time_to_prost_timestamp(&t))
                    .transpose()
                    .map_err(|_| ServiceError::Generic("Invalid expiry time".to_string()))?,
                transfer_package: Some(prepared_package.transfer_package),
                ..Default::default()
            },
            cpfp_signed_txs: prepared_package.cpfp_signed_txs,
        })
    }

    /// Encrypts key tweaks for each signing operator using their identity public keys
    pub(crate) fn encrypt_key_tweaks(
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
        utils::ecies::encrypt(&public_key_bytes, data)
            .map_err(|e| ServiceError::Generic(format!("ECIES encryption failed: {e}")))
    }

    pub(crate) async fn sign_transfer_package(
        &self,
        transfer_id: &TransferId,
        transfer_package: operator_rpc::spark::TransferPackage,
    ) -> Result<operator_rpc::spark::TransferPackage, ServiceError> {
        let signing_payload =
            self.get_transfer_package_signing_payload(transfer_id, &transfer_package)?;

        let signature = self
            .signer
            .sign_message_ecdsa_with_identity_key(&signing_payload)
            .await
            .map_err(ServiceError::SignerError)?;

        // Create a new transfer package with the signature
        let mut signed_package = transfer_package;
        signed_package.user_signature = signature.serialize_der().to_vec();

        Ok(signed_package)
    }

    /// Creates the signing payload for a transfer package using tagged hashing.
    /// Uses V2 structured hashing with domain tag for collision resistance.
    fn get_transfer_package_signing_payload(
        &self,
        transfer_id: &TransferId,
        transfer_package: &operator_rpc::spark::TransferPackage,
    ) -> Result<Vec<u8>, ServiceError> {
        let transfer_id_bytes =
            hex::decode(transfer_id.to_string().replace('-', "")).map_err(|e| {
                ServiceError::ValidationError(format!("Failed to decode transfer ID: {e}"))
            })?;

        // Convert HashMap to BTreeMap for deterministic ordering
        let key_tweak_package: std::collections::BTreeMap<String, Vec<u8>> = transfer_package
            .key_tweak_package
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let signable_message = TaggedHasher::new(&["spark", "transfer", "signing payload"])
            .add_bytes(&transfer_id_bytes)
            .add_map_string_to_bytes(&key_tweak_package)
            .signable_message();

        Ok(signable_message)
    }

    /// Claims a transfer with retry logic and automatic leaf preparation.
    ///
    /// Returns the claimed leaves on success. If a concurrent instance of this
    /// wallet wins the race and finalizes the transfer, the coordinator's finalized
    /// leaves are returned uniformly — callers do not need to distinguish this case.
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

            match self.try_claim_transfer(transfer).await {
                Ok(nodes) => return Ok(nodes),
                Err(ServiceError::NoLeavesToClaim) => {
                    debug!("There are no leaves to claim for this transfer");
                    return Ok(Vec::new());
                }
                Err(e) => {
                    error!("Failed to claim transfer: {}", e);
                    // A concurrent instance of this wallet may have finalized the
                    // transfer — if so, use its leaves instead of retrying.
                    if let Some(leaves) =
                        self.finalized_leaves_if_already_claimed(&transfer.id).await
                    {
                        return Ok(leaves);
                    }
                    retry_count += 1;
                }
            }
        }
    }

    /// Single claim attempt: verify, prepare leaves, and run the 3-step claim flow.
    async fn try_claim_transfer(&self, transfer: &Transfer) -> Result<Vec<TreeNode>, ServiceError> {
        let leaf_key_map = self.verify_pending_transfer(transfer).await?;
        let leaves_to_claim = self
            .prepare_leaves_for_claiming(transfer, &leaf_key_map)
            .await?;
        self.claim_transfer_with_leaves(transfer, leaves_to_claim)
            .await
    }

    /// If the transfer is `Completed` on the coordinator, returns its finalized
    /// leaves. Used after a failed claim attempt to detect that another instance
    /// of this wallet already finalized the claim concurrently.
    ///
    /// A failed coordinator query is non-fatal here — it's logged and treated as
    /// "not completed" so the caller falls through to its normal error handling.
    async fn finalized_leaves_if_already_claimed(
        &self,
        transfer_id: &TransferId,
    ) -> Option<Vec<TreeNode>> {
        let completed = match self.query_transfer(transfer_id).await {
            Ok(Some(t)) if t.status == TransferStatus::Completed => t,
            Ok(_) => return None,
            Err(e) => {
                warn!("Failed to check if transfer {transfer_id} was claimed concurrently: {e:?}");
                return None;
            }
        };
        debug!(
            "Transfer {transfer_id} already claimed by another instance; using coordinator's finalized leaves"
        );
        let our_pubkey = self.signer.get_identity_public_key().await.ok()?;
        let leaves: Vec<TreeNode> = completed
            .leaves
            .into_iter()
            .map(|l| l.leaf)
            .filter(|leaf| {
                let is_ours = leaf.owner_identity_public_key == Some(our_pubkey);
                if !is_ours {
                    debug!(
                        "Dropping leaf {} from already-claimed transfer {transfer_id} — \
                         owner {:?} is no longer us",
                        leaf.id, leaf.owner_identity_public_key
                    );
                }
                is_ours
            })
            .collect();
        Some(leaves)
    }

    /// Prepares leaves for claiming by creating LeafKeyTweak structs
    async fn prepare_leaves_for_claiming(
        &self,
        transfer: &Transfer,
        leaf_key_map: &HashMap<TreeNodeId, SecretSource>,
    ) -> Result<Vec<LeafKeyTweak>, ServiceError> {
        let mut leaves_to_claim = Vec::new();

        for leaf in &transfer.leaves {
            let Some(leaf_key) = leaf_key_map.get(&leaf.leaf.id) else {
                continue;
            };

            leaves_to_claim.push(LeafKeyTweak {
                node: leaf.leaf_with_intermediate_txs(),
                signing_key: leaf_key.clone(),
                new_signing_key: SecretSource::Derived(leaf.leaf.id.clone()),
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

        let claim_package = self
            .prepare_claim_package(transfer, &leaves_to_claim)
            .await?;

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .claim_transfer(operator_rpc::spark::ClaimTransferRequest {
                transfer_id: transfer.id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()
                    .await?
                    .serialize()
                    .to_vec(),
                claim_package: Some(claim_package),
            })
            .await?;

        let claimed_transfer: Transfer = response
            .transfer
            .ok_or_else(|| {
                ServiceError::Generic("claim_transfer returned no transfer".to_string())
            })?
            .try_into()?;

        Ok(claimed_transfer
            .leaves
            .into_iter()
            .map(|l| l.leaf)
            .collect())
    }

    /// Assembles a signed `ClaimPackage` for the coordinator: per-operator
    /// ECIES-encrypted key tweaks, user-signed refund jobs, and an
    /// identity-key signature over the package payload.
    async fn prepare_claim_package(
        &self,
        transfer: &Transfer,
        leaves: &[LeafKeyTweak],
    ) -> Result<operator_rpc::spark::ClaimPackage, ServiceError> {
        if leaves.is_empty() {
            return Err(ServiceError::NoLeavesToClaim);
        }
        let node_id_count: u32 = leaves
            .len()
            .try_into()
            .map_err(|_| ServiceError::InvalidInput("too many leaves to claim".to_string()))?;

        // Fetch operator signing commitments. The receiver does not yet own the
        // leaves, so request commitments by count rather than by node id.
        let signing_commitments = self
            .operator_pool
            .get_coordinator()
            .client
            .get_signing_commitments(operator_rpc::spark::GetSigningCommitmentsRequest {
                node_ids: Vec::new(),
                count: 3,
                node_id_count,
            })
            .await?
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(&sc.signing_nonce_commitments))
            .collect::<Result<Vec<_>, _>>()?;

        let [cpfp_chunk, direct_chunk, direct_from_cpfp_chunk] =
            split_signing_commitments_by_variant(&signing_commitments, leaves.len())?;

        // Sign the claim refunds (current timelock) operator-commits-first via
        // sign_frost; the operators aggregate server-side during `claim_transfer`.
        let (cpfp_jobs, direct_jobs, direct_from_cpfp_jobs) = self
            .sign_claim_refunds(leaves, cpfp_chunk, direct_chunk, direct_from_cpfp_chunk)
            .await?;

        // Key-tweak step (decrypt incoming key, derive new key, compute tweak,
        // Feldman-split, ECIES per operator, sign the claim-package payload).
        let claim_leaves = leaves
            .iter()
            .map(|leaf| {
                let SecretSource::Encrypted(cipher) = &leaf.signing_key else {
                    return Err(ServiceError::InvalidInput(
                        "claim leaf signing key must be the encrypted incoming key".to_string(),
                    ));
                };
                let sender_signature = transfer
                    .leaves
                    .iter()
                    .find(|tl| tl.leaf.id == leaf.node.id)
                    .and_then(|tl| tl.signature)
                    .map(|s| s.serialize_compact().to_vec())
                    .unwrap_or_default();
                Ok(ClaimLeafInput {
                    node: leaf.node.clone(),
                    sender_signature,
                    leaf_key_ciphertext: cipher.as_slice().to_vec(),
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;

        let prepared = self
            .spark_signer
            .prepare_claim(PrepareClaimRequest {
                transfer_id: transfer.id.clone(),
                sender_identity_public_key: transfer.sender_identity_public_key,
                leaves: claim_leaves,
                operator_recipients: self.operator_recipients(),
                threshold: self.split_secret_threshold,
            })
            .await?;

        Ok(operator_rpc::spark::ClaimPackage {
            leaves_to_claim: cpfp_jobs,
            direct_leaves_to_claim: direct_jobs,
            direct_from_cpfp_leaves_to_claim: direct_from_cpfp_jobs,
            key_tweak_package: prepared
                .operator_packages
                .into_iter()
                .map(|p| {
                    (
                        hex::encode(p.operator_identifier.serialize()),
                        p.encrypted_package,
                    )
                })
                .collect(),
            user_signature: prepared.claim_user_signature.serialize_der().to_vec(),
            hash_variant: HashVariant::V2.into(),
        })
    }

    /// Signs claim refund transactions operator-commits-first. The operator
    /// aggregates server-side during `claim_transfer`.
    async fn sign_claim_refunds(
        &self,
        leaves: &[LeafKeyTweak],
        cpfp_commitments: &[std::collections::BTreeMap<
            Identifier,
            frost_secp256k1_tr::round1::SigningCommitments,
        >],
        direct_commitments: &[std::collections::BTreeMap<
            Identifier,
            frost_secp256k1_tr::round1::SigningCommitments,
        >],
        direct_from_cpfp_commitments: &[std::collections::BTreeMap<
            Identifier,
            frost_secp256k1_tr::round1::SigningCommitments,
        >],
    ) -> Result<
        (
            Vec<operator_rpc::spark::UserSignedTxSigningJob>,
            Vec<operator_rpc::spark::UserSignedTxSigningJob>,
            Vec<operator_rpc::spark::UserSignedTxSigningJob>,
        ),
        ServiceError,
    > {
        // Build per-leaf signing data using the receiver's new signing key.
        let mut leaf_data_map: HashMap<TreeNodeId, LeafRefundSigningData> = HashMap::new();
        for leaf in leaves {
            let signing_public_key = self
                .signer
                .public_key_from_secret(&leaf.new_signing_key)
                .await?;
            leaf_data_map.insert(
                leaf.node.id.clone(),
                LeafRefundSigningData {
                    signing_private_key: leaf.new_signing_key.clone(),
                    signing_public_key,
                    receiving_public_key: signing_public_key,
                    tx: leaf.node.node_tx.clone(),
                    direct_tx: leaf.node.direct_tx.clone(),
                    refund_tx: None,
                    direct_refund_tx: None,
                    direct_from_cpfp_refund_tx: None,
                    signing_nonce_commitment: self
                        .signer
                        .generate_random_signing_commitment()
                        .await?,
                    direct_signing_nonce_commitment: self
                        .signer
                        .generate_random_signing_commitment()
                        .await?,
                    direct_from_cpfp_signing_nonce_commitment: self
                        .signer
                        .generate_random_signing_commitment()
                        .await?,
                    vout: leaf.node.vout,
                    connector_prev_out: None,
                },
            );
        }

        // Build the claim refund transactions (current timelock) into
        // `leaf_data_map`. The (fused-form) signing jobs returned here are
        // discarded — we sign operator-commits-first below.
        prepare_refund_so_signing_jobs(self.network, leaves, &mut leaf_data_map, true)?;

        let mut cpfp_jobs = Vec::new();
        let mut direct_jobs = Vec::new();
        let mut direct_from_cpfp_jobs = Vec::new();
        for (i, leaf) in leaves.iter().enumerate() {
            let data = leaf_data_map.remove(&leaf.node.id).ok_or_else(|| {
                ServiceError::Generic(format!("Leaf data not found for leaf {}", leaf.node.id))
            })?;
            let verifying_key = leaf.node.verifying_public_key;

            let LeafRefundSigningData {
                signing_public_key,
                tx: node_tx,
                direct_tx,
                refund_tx,
                direct_refund_tx,
                direct_from_cpfp_refund_tx,
                ..
            } = data;

            let cpfp_refund_tx = refund_tx
                .ok_or_else(|| ServiceError::Generic("Missing cpfp refund tx".to_string()))?;
            let cpfp_sighash = sighash_from_tx(&cpfp_refund_tx, 0, &node_tx.output[0])?;
            cpfp_jobs.push(
                self.sign_claim_refund_job(
                    &leaf.node.id,
                    cpfp_refund_tx,
                    cpfp_sighash.as_byte_array(),
                    &signing_public_key,
                    cpfp_commitments[i].clone(),
                    &verifying_key,
                )
                .await?,
            );

            if let (Some(direct_tx), Some(direct_refund_tx)) = (direct_tx, direct_refund_tx) {
                let sighash = sighash_from_tx(&direct_refund_tx, 0, &direct_tx.output[0])?;
                direct_jobs.push(
                    self.sign_claim_refund_job(
                        &leaf.node.id,
                        direct_refund_tx,
                        sighash.as_byte_array(),
                        &signing_public_key,
                        direct_commitments[i].clone(),
                        &verifying_key,
                    )
                    .await?,
                );
            }

            if let Some(dfc_refund_tx) = direct_from_cpfp_refund_tx {
                let sighash = sighash_from_tx(&dfc_refund_tx, 0, &node_tx.output[0])?;
                direct_from_cpfp_jobs.push(
                    self.sign_claim_refund_job(
                        &leaf.node.id,
                        dfc_refund_tx,
                        sighash.as_byte_array(),
                        &signing_public_key,
                        direct_from_cpfp_commitments[i].clone(),
                        &verifying_key,
                    )
                    .await?,
                );
            }
        }

        Ok((cpfp_jobs, direct_jobs, direct_from_cpfp_jobs))
    }

    async fn sign_claim_refund_job(
        &self,
        node_id: &TreeNodeId,
        refund_tx: Transaction,
        sighash_bytes: &[u8; 32],
        signing_public_key: &PublicKey,
        operator_commitments: std::collections::BTreeMap<
            Identifier,
            frost_secp256k1_tr::round1::SigningCommitments,
        >,
        verifying_key: &PublicKey,
    ) -> Result<operator_rpc::spark::UserSignedTxSigningJob, ServiceError> {
        // The claim refund is signed with the receiver's new leaf key, which is
        // the derived key for this node id.
        let share = self
            .spark_signer
            .sign_frost(vec![FrostJob {
                derivation: FrostDerivation::SigningLeaf {
                    leaf_id: node_id.clone(),
                },
                sighash: *sighash_bytes,
                verifying_key: *verifying_key,
                operator_commitments: operator_commitments.clone(),
                adaptor_public_key: None,
            }])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| ServiceError::Generic("sign_frost returned no share".to_string()))?;

        let signed_tx = SignedTx {
            node_id: node_id.clone(),
            signing_public_key: *signing_public_key,
            tx: refund_tx,
            user_signature: share.signature_share,
            signing_commitments: operator_commitments,
            self_nonce_commitment: share.commitment,
            network: self.network,
        };
        (&signed_tx).try_into()
    }

    pub async fn verify_pending_transfer(
        &self,
        transfer: &Transfer,
    ) -> Result<HashMap<TreeNodeId, SecretSource>, ServiceError> {
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
            let private_key = SecretSource::new_encrypted(transfer_leaf.secret_cipher.clone());

            leaf_key_map.insert(transfer_leaf.leaf.id.clone(), private_key);
        }

        Ok(leaf_key_map)
    }

    async fn query_transfers_inner(
        &self,
        transfer_ids: &[TransferId],
        paging: PagingFilter,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        trace!(
            "Querying transfers with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let order: crate::operator::rpc::spark::Order = paging.order.into();
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .query_all_transfers(TransferFilter {
                order: order.into(),
                transfer_ids: transfer_ids.iter().map(|id| id.to_string()).collect(),
                participant: Some(Participant::SenderOrReceiverIdentityPublicKey(
                    self.signer
                        .get_identity_public_key()
                        .await?
                        .serialize()
                        .to_vec(),
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
        transfer_ids: &[TransferId],
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        let transfers = match paging {
            Some(paging) => self.query_transfers_inner(transfer_ids, paging).await?,
            None => {
                pager(
                    |f| self.query_transfers_inner(transfer_ids, f),
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
                    self.signer
                        .get_identity_public_key()
                        .await?
                        .serialize()
                        .to_vec(),
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
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        let transfers = match paging {
            Some(paging) => self.query_pending_transfers_inner(paging).await?,
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

    async fn query_claimable_receiver_transfers_inner(
        &self,
        paging: PagingFilter,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        trace!(
            "Querying pending (claimable) receiver transfers with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let resp = self
            .operator_pool
            .get_coordinator()
            .client
            .query_pending_transfers(operator_rpc::spark::TransferFilter {
                network: self.network.to_proto_network() as i32,
                participant: Some(Participant::ReceiverIdentityPublicKey(
                    self.signer
                        .get_identity_public_key()
                        .await?
                        .serialize()
                        .to_vec(),
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
    pub async fn query_claimable_receiver_transfers(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<Transfer>, ServiceError> {
        let transfers = match paging {
            Some(paging) => {
                self.query_claimable_receiver_transfers_inner(paging)
                    .await?
            }
            None => {
                pager(
                    |f| self.query_claimable_receiver_transfers_inner(f),
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
                    self.signer
                        .get_identity_public_key()
                        .await?
                        .serialize()
                        .to_vec(),
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
