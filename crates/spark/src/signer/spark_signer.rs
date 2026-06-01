//! High-level Spark signing trait.
//!
//! `SparkSigner` is the *flow-level* signing interface that the Spark
//! protocol layer talks to. Each trait method is one protocol operation —
//! "prepare this deposit", "prepare this transfer", "claim this transfer".
//! The trait shape mirrors the Turnkey Spark activity surface so a Turnkey
//! backend can be a thin adapter on top, while the default in-process
//! adapter (`SparkSignerAdapter`) does the same work locally over the
//! existing low-level [`Signer`](super::Signer) trait.
//!
//! Each method takes a complete request struct and returns a complete
//! prepared-package struct. The caller (service code) is responsible for
//! gathering inputs (operator commitments, leaf metadata, sighashes the
//! signer needs to bind) and shipping the returned package to the
//! coordinator. The signer never sees raw "sign this hash" or raw FROST
//! round-1/round-2 primitives — every operation has a known protocol
//! purpose.

use std::collections::BTreeMap;

use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::{Identifier, round1::SigningCommitments};

use super::{FrostSigningCommitmentsWithNonces, SignerError};
use crate::services::TransferId;
use crate::tree::{TreeNode, TreeNodeId};

// ─── Common pieces ────────────────────────────────────────────────────────

/// A Spark Operator (statechain participant) the signer will need to address
/// for share-encryption or commitment-binding.
#[derive(Debug, Clone)]
pub struct OperatorRecipient {
    /// FROST identifier (e.g. 0x000...01).
    pub identifier: Identifier,
    /// ECIES encryption / identity public key for the operator.
    pub public_key: PublicKey,
}

/// A FROST share-signing job for one Bitcoin sighash, plus the operator
/// commitments the signer must combine its share against.
#[derive(Debug, Clone)]
pub struct FrostJob {
    /// 32-byte BIP-341 sighash.
    pub sighash: [u8; 32],
    /// FROST group verifying key (untweaked).
    pub verifying_key: PublicKey,
    /// Per-operator round-1 commitments fetched from the coordinator.
    pub operator_commitments: BTreeMap<Identifier, SigningCommitments>,
    /// Optional adaptor public key (atomic-swap flows).
    pub adaptor_public_key: Option<PublicKey>,
}

/// The user-side FROST share + the nonce commitment it was bound to, ready
/// to be packaged into a `UserSignedTxSigningJob` for the coordinator.
#[derive(Debug, Clone)]
pub struct FrostShareResult {
    /// The user's nonce commitment (round-1 output, hiding + binding).
    pub commitment: FrostSigningCommitmentsWithNonces,
    /// The user's signature share (round-2 output).
    pub signature_share: frost_secp256k1_tr::round2::SignatureShare,
}

// ─── prepare_deposit ──────────────────────────────────────────────────────

/// Inputs for signing a deposit-tree-creation. The signer produces three
/// FROST shares — one each for the cpfp-root, cpfp-refund, and
/// direct-from-cpfp-refund transactions.
#[derive(Debug, Clone)]
pub struct PrepareDepositRequest {
    pub cpfp_root: FrostJob,
    pub cpfp_refund: FrostJob,
    pub direct_from_cpfp_refund: FrostJob,
}

#[derive(Debug, Clone)]
pub struct PreparedDeposit {
    pub cpfp_root: FrostShareResult,
    pub cpfp_refund: FrostShareResult,
    pub direct_from_cpfp_refund: FrostShareResult,
}

// ─── prepare_transfer ─────────────────────────────────────────────────────

/// A single leaf being sent in a transfer.
#[derive(Debug, Clone)]
pub struct TransferLeafInput {
    /// The leaf as the sender currently owns it.
    pub node: TreeNode,
    /// FROST share jobs for that leaf's three refund variants (current
    /// timelock for the receiver). Each is operator-commits-first.
    pub cpfp_refund: FrostJob,
    pub direct_refund: Option<FrostJob>,
    pub direct_from_cpfp_refund: Option<FrostJob>,
}

#[derive(Debug, Clone)]
pub struct PrepareTransferRequest {
    pub transfer_id: TransferId,
    pub receiver_public_key: PublicKey,
    pub leaves: Vec<TransferLeafInput>,
    /// Operators that will receive Feldman-split key-tweak shares
    /// (each addressed by identifier with their ECIES key).
    pub operator_recipients: Vec<OperatorRecipient>,
    pub threshold: u32,
    /// 32-byte hash of the transfer-package contents to be signed by the
    /// sender's identity key.
    pub transfer_package_payload_hash: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct PreparedTransferLeafOutput {
    pub node_id: TreeNodeId,
    pub cpfp_refund: FrostShareResult,
    pub direct_refund: Option<FrostShareResult>,
    pub direct_from_cpfp_refund: Option<FrostShareResult>,
    /// Per-operator ECIES-encrypted Feldman shares of the leaf key tweak.
    pub operator_key_tweak_packages: Vec<OperatorPackage>,
}

/// One ECIES-encrypted Feldman share of a per-leaf or per-claim key tweak,
/// addressed to a specific operator.
#[derive(Debug, Clone)]
pub struct OperatorPackage {
    pub operator_identifier: Identifier,
    pub encrypted_package: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PreparedTransfer {
    pub leaves: Vec<PreparedTransferLeafOutput>,
    /// ECDSA signature over the transfer-package payload, produced with the
    /// sender's identity key.
    pub transfer_user_signature: bitcoin::secp256k1::ecdsa::Signature,
}

// ─── prepare_claim ────────────────────────────────────────────────────────

/// A single leaf being claimed.
#[derive(Debug, Clone)]
pub struct ClaimLeafInput {
    /// Leaf as it lands at the receiver (pre-claim state).
    pub node: TreeNode,
    /// Sender's ECDSA signature binding this leaf to the transfer.
    pub sender_signature: bitcoin::secp256k1::ecdsa::Signature,
    /// ECIES ciphertext of the leaf signing key, encrypted for this receiver.
    pub leaf_key_ciphertext: Vec<u8>,
    /// FROST share jobs for the claim-side refund variants (current timelock,
    /// signed with the *new* receiver-side signing key the signer derives).
    pub cpfp_refund: FrostJob,
    pub direct_refund: Option<FrostJob>,
    pub direct_from_cpfp_refund: Option<FrostJob>,
}

#[derive(Debug, Clone)]
pub struct PrepareClaimRequest {
    pub transfer_id: TransferId,
    pub sender_identity_public_key: PublicKey,
    pub leaves: Vec<ClaimLeafInput>,
    pub operator_recipients: Vec<OperatorRecipient>,
    pub threshold: u32,
    /// 32-byte hash of the claim-package contents to be signed by the
    /// receiver's identity key.
    pub claim_package_payload_hash: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct PreparedClaimLeafOutput {
    pub node_id: TreeNodeId,
    /// Public key the signer derived for this leaf (so the service can
    /// thread it into the package and persist the mapping).
    pub new_signing_public_key: PublicKey,
    pub cpfp_refund: FrostShareResult,
    pub direct_refund: Option<FrostShareResult>,
    pub direct_from_cpfp_refund: Option<FrostShareResult>,
    pub operator_key_tweak_packages: Vec<OperatorPackage>,
}

#[derive(Debug, Clone)]
pub struct PreparedClaim {
    pub leaves: Vec<PreparedClaimLeafOutput>,
    pub claim_user_signature: bitcoin::secp256k1::ecdsa::Signature,
}

// ─── prepare_coop_exit ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PrepareCoopExitRequest {
    pub transfer_id: TransferId,
    /// SSP identity public key, addressed as the receiver of the transfer.
    pub receiver_public_key: PublicKey,
    /// Leaves being transferred to the SSP, each with its connector-refund
    /// share jobs (decremented timelock).
    pub leaves: Vec<TransferLeafInput>,
    pub operator_recipients: Vec<OperatorRecipient>,
    pub threshold: u32,
    pub transfer_package_payload_hash: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct PreparedCoopExit {
    pub leaves: Vec<PreparedTransferLeafOutput>,
    pub transfer_user_signature: bitcoin::secp256k1::ecdsa::Signature,
}

// ─── prepare_lightning_receive ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PrepareLightningReceiveRequest {
    pub operator_recipients: Vec<OperatorRecipient>,
    pub threshold: u32,
}

#[derive(Debug, Clone)]
pub struct PreparedLightningReceive {
    /// SHA256 of the in-enclave preimage, for BOLT11 invoice construction.
    /// The raw preimage never leaves the signer.
    pub payment_hash: [u8; 32],
    /// Per-operator ECIES-encrypted Feldman shares of the preimage.
    pub operator_preimage_packages: Vec<OperatorPackage>,
}

// ─── prepare_token_transaction ────────────────────────────────────────────

/// Discriminator for what kind of token-transaction signature is being
/// requested. Lets policy-enforcing signers gate freeze vs. spend separately.
#[derive(Debug, Clone, Copy)]
pub enum TokenTransactionKind {
    /// Issuer-side signature on a freeze message.
    Freeze,
    /// Owner-side signature on a partial token transaction (`compute_hash`).
    Partial,
    /// Owner-side signature on a finalized token transaction (`SHA256(SHA256(tx_hash || op_pubkey_hash))`).
    Final,
}

#[derive(Debug, Clone)]
pub struct PrepareTokenTransactionRequest {
    pub kind: TokenTransactionKind,
    /// 32-byte digest the signer should Schnorr-sign with the identity key.
    pub digest: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct PreparedTokenTransaction {
    pub signature: bitcoin::secp256k1::schnorr::Signature,
}

// ─── trait ────────────────────────────────────────────────────────────────

/// High-level signing surface exposed to the Spark protocol layer.
#[macros::async_trait]
pub trait SparkSigner: Send + Sync + 'static {
    /// Returns the wallet's identity public key.
    async fn get_identity_public_key(&self) -> Result<PublicKey, SignerError>;

    /// Prepare a deposit-tree-creation: produce FROST shares for the three
    /// root-tree transactions (cpfp root, cpfp refund, direct-from-cpfp
    /// refund). Maps to `SPARK_SIGN_FROST` batched ×3.
    async fn prepare_deposit(
        &self,
        request: PrepareDepositRequest,
    ) -> Result<PreparedDeposit, SignerError>;

    /// Prepare an outbound transfer: per-leaf FROST refund shares + Feldman
    /// + ECIES key-tweak packages + identity-key signature over the transfer
    /// package payload. Maps to `SPARK_SIGN_FROST` (×N leaves, batched) +
    /// `SPARK_PREPARE_TRANSFER`.
    async fn prepare_transfer(
        &self,
        request: PrepareTransferRequest,
    ) -> Result<PreparedTransfer, SignerError>;

    /// Claim an inbound transfer: verify each sender signature, ECIES-decrypt
    /// the leaf-key ciphertext, derive the receiver's new HD leaf key,
    /// FROST-sign claim-side refunds, and produce ECIES-encrypted claim-tweak
    /// packages. Maps to `SPARK_CLAIM_TRANSFER` + `SPARK_SIGN_FROST`.
    async fn prepare_claim(
        &self,
        request: PrepareClaimRequest,
    ) -> Result<PreparedClaim, SignerError>;

    /// Prepare a cooperative-exit transfer (connector refund signing +
    /// key-tweak packages + identity signature). Same internal shape as
    /// `prepare_transfer` but uses decremented-timelock connector refunds.
    async fn prepare_coop_exit(
        &self,
        request: PrepareCoopExitRequest,
    ) -> Result<PreparedCoopExit, SignerError>;

    /// Prepare a Lightning receive: generate a random preimage in-enclave,
    /// Feldman-split it for the operators, return its hash for BOLT11.
    /// Maps to `SPARK_PREPARE_LIGHTNING_RECEIVE`.
    async fn prepare_lightning_receive(
        &self,
        request: PrepareLightningReceiveRequest,
    ) -> Result<PreparedLightningReceive, SignerError>;

    /// Prepare a token transaction (freeze / partial / final): Schnorr-sign
    /// the digest with the identity key.
    async fn prepare_token_transaction(
        &self,
        request: PrepareTokenTransactionRequest,
    ) -> Result<PreparedTokenTransaction, SignerError>;
}
