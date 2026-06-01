//! High-level Spark signing trait.
//!
//! `SparkSigner` is the *flow-level* signing interface the Spark protocol
//! layer talks to. Its methods mirror the Turnkey Spark activity surface, so a
//! Turnkey backend is a thin adapter on top, while the in-process default
//! ([`SparkSignerAdapter`](super::SparkSignerAdapter)) does the same work
//! locally over the low-level [`Signer`](super::Signer) trait.
//!
//! The trait is **derivation-path-agnostic**: methods speak in Spark concepts
//! (leaf id, static-deposit index, transfer/claim) and never in BIP-32 paths.
//! Each implementation resolves those concepts to its own key material —
//! Turnkey inside its enclave, the default adapter by reproducing the exact
//! key derivation we use today.
//!
//! Several Spark flows are *compositions* of these methods rather than
//! dedicated trait methods:
//!  * deposit tree creation = `sign_frost` (the three root-tree txs)
//!  * cooperative exit = `sign_frost` (connector refunds) + `prepare_transfer`
//!  * lightning send = `sign_frost` (HTLC refunds) + `prepare_transfer`
//!  * timelock renewal / static-deposit refund = `sign_frost`

use std::collections::BTreeMap;

use bitcoin::secp256k1::{PublicKey, ecdsa, schnorr};
use frost_secp256k1_tr::{Identifier, round1::SigningCommitments, round2::SignatureShare};

use super::{FrostSigningCommitmentsWithNonces, SignerError};
use crate::services::TransferId;
use crate::tree::{TreeNode, TreeNodeId};

// ─── shared types ─────────────────────────────────────────────────────────

/// A Spark Operator (statechain participant) the signer addresses for
/// share-encryption.
#[derive(Debug, Clone)]
pub struct OperatorRecipient {
    /// FROST identifier (e.g. 0x000...01).
    pub identifier: Identifier,
    /// The operator's ECIES / identity public key.
    pub public_key: PublicKey,
}

/// An ECIES-encrypted package destined for one operator (e.g. encrypted
/// key-tweak shares or preimage shares).
#[derive(Debug, Clone)]
pub struct OperatorPackage {
    pub operator_identifier: Identifier,
    pub encrypted_package: Vec<u8>,
}

// ─── sign_frost ───────────────────────────────────────────────────────────

/// Which key the signer should use to produce a FROST share. These are
/// Spark-level concepts, never derivation paths; each `SparkSigner`
/// implementation maps them onto its own key material.
#[derive(Debug, Clone)]
pub enum FrostDerivation {
    /// The signing key for a tree node. Covers transfer/coop-exit refund
    /// signing, deposit tree-root signing, and timelock renewal.
    SigningLeaf { leaf_id: TreeNodeId },
    /// The static-deposit key at `index` (static-deposit refund).
    StaticDeposit { index: u32 },
    /// The HTLC-preimage key (lightning send).
    HtlcPreimage,
    /// The wallet identity key.
    Identity,
}

/// A single FROST share-signing job: produce a partial signature over
/// `sighash`, combined against the operators' round-1 commitments.
#[derive(Debug, Clone)]
pub struct FrostJob {
    /// Which key to sign with.
    pub derivation: FrostDerivation,
    /// 32-byte BIP-341 sighash to sign.
    pub sighash: [u8; 32],
    /// FROST group verifying key (untweaked).
    pub verifying_key: PublicKey,
    /// Per-operator round-1 commitments fetched from the coordinator.
    pub operator_commitments: BTreeMap<Identifier, SigningCommitments>,
    /// Optional adaptor public key (atomic-swap flows).
    pub adaptor_public_key: Option<PublicKey>,
}

/// The user-side FROST share plus the nonce commitment it was bound to, ready
/// to be packaged into a `UserSignedTxSigningJob` for the coordinator.
#[derive(Debug, Clone)]
pub struct FrostShareResult {
    /// The user's nonce commitment (round-1 output: hiding + binding).
    pub commitment: FrostSigningCommitmentsWithNonces,
    /// The user's signature share (round-2 output).
    pub signature_share: SignatureShare,
}

// ─── prepare_transfer ─────────────────────────────────────────────────────

/// A single leaf being sent in an outbound transfer. The signer derives the
/// old leaf key from `node.id` and generates a fresh receiver key internally.
#[derive(Debug, Clone)]
pub struct TransferLeafInput {
    pub node: TreeNode,
}

#[derive(Debug, Clone)]
pub struct PrepareTransferRequest {
    pub transfer_id: TransferId,
    pub receiver_public_key: PublicKey,
    pub leaves: Vec<TransferLeafInput>,
    pub operator_recipients: Vec<OperatorRecipient>,
    pub threshold: u32,
}

/// The new signing key the signer generated for a sent leaf, returned so the
/// caller can persist the mapping.
#[derive(Debug, Clone)]
pub struct NewLeafKey {
    pub node_id: TreeNodeId,
    pub new_signing_public_key: PublicKey,
}

#[derive(Debug, Clone)]
pub struct PreparedTransfer {
    /// One ECIES-encrypted package per operator, covering every leaf's
    /// key-tweak shares (this is the `key_tweak_package` map).
    pub operator_packages: Vec<OperatorPackage>,
    /// The new key generated per leaf.
    pub new_leaf_keys: Vec<NewLeafKey>,
    /// ECDSA signature over the transfer-package payload (identity key).
    pub transfer_user_signature: ecdsa::Signature,
}

// ─── prepare_claim ────────────────────────────────────────────────────────

/// A single refund tx the signer must FROST-sign during a claim (with the
/// new receiver key it derives internally).
#[derive(Debug, Clone)]
pub struct ClaimRefundJob {
    /// 32-byte BIP-341 sighash to sign.
    pub sighash: [u8; 32],
    /// FROST group verifying key (untweaked).
    pub verifying_key: PublicKey,
    /// Per-operator round-1 commitments fetched from the coordinator.
    pub operator_commitments: BTreeMap<Identifier, SigningCommitments>,
}

/// A single leaf being claimed.
#[derive(Debug, Clone)]
pub struct ClaimLeafInput {
    /// The leaf as it lands at the receiver (pre-claim state).
    pub node: TreeNode,
    /// Sender's ECDSA signature binding this leaf to the transfer.
    pub sender_signature: ecdsa::Signature,
    /// ECIES ciphertext of the incoming leaf signing key, encrypted for this
    /// receiver; the signer decrypts it and derives the claim key tweak.
    pub leaf_key_ciphertext: Vec<u8>,
    /// Refund jobs (cpfp / direct / direct-from-cpfp) signed with the new key.
    pub cpfp_refund: ClaimRefundJob,
    pub direct_refund: Option<ClaimRefundJob>,
    pub direct_from_cpfp_refund: Option<ClaimRefundJob>,
}

#[derive(Debug, Clone)]
pub struct PrepareClaimRequest {
    pub transfer_id: TransferId,
    pub sender_identity_public_key: PublicKey,
    pub leaves: Vec<ClaimLeafInput>,
    pub operator_recipients: Vec<OperatorRecipient>,
    pub threshold: u32,
}

/// Signed refund shares for one claimed leaf, plus the new key the signer
/// derived for it.
#[derive(Debug, Clone)]
pub struct PreparedClaimLeaf {
    pub node_id: TreeNodeId,
    pub new_signing_public_key: PublicKey,
    pub cpfp_refund: FrostShareResult,
    pub direct_refund: Option<FrostShareResult>,
    pub direct_from_cpfp_refund: Option<FrostShareResult>,
}

#[derive(Debug, Clone)]
pub struct PreparedClaim {
    pub leaves: Vec<PreparedClaimLeaf>,
    /// One ECIES-encrypted claim-tweak package per operator.
    pub operator_packages: Vec<OperatorPackage>,
    /// ECDSA signature over the claim-package payload (identity key).
    pub claim_user_signature: ecdsa::Signature,
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
    /// One ECIES-encrypted preimage-share package per operator.
    pub operator_preimage_packages: Vec<OperatorPackage>,
}

// ─── prepare_static_deposit ───────────────────────────────────────────────

/// Static-deposit creation. Unlike a regular deposit, the static-deposit key
/// is exported (encrypted) to the SSP so it can co-sign refunds offline; the
/// signer also FROST-signs the deposit tree txs.
#[derive(Debug, Clone)]
pub struct PrepareStaticDepositRequest {
    /// Static-deposit address index.
    pub index: u32,
    /// SSP public key to which the static-deposit secret is encrypted.
    pub ssp_public_key: PublicKey,
    /// FROST jobs for the static-deposit tree txs.
    pub frost_jobs: Vec<FrostJob>,
}

#[derive(Debug, Clone)]
pub struct PreparedStaticDeposit {
    /// The static-deposit secret, ECIES-encrypted for the SSP.
    pub exported_secret: Vec<u8>,
    /// FROST shares for the deposit tree txs (same order as `frost_jobs`).
    pub frost_shares: Vec<FrostShareResult>,
}

// ─── sign_spark_invoice ───────────────────────────────────────────────────

/// Which Spark invoice payload is being signed (the two have different hash
/// inputs). Spark invoices are unrelated to Lightning.
#[derive(Debug, Clone, Copy)]
pub enum SparkInvoiceKind {
    Sats,
    Tokens,
}

#[derive(Debug, Clone)]
pub struct SignSparkInvoiceRequest {
    pub kind: SparkInvoiceKind,
    /// 32-byte invoice hash to Schnorr-sign with the identity key.
    pub invoice_hash: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct SignedSparkInvoice {
    pub signature: schnorr::Signature,
}

// ─── prepare_token_transaction ────────────────────────────────────────────

/// Discriminator for the kind of token-transaction signature requested. Lets
/// policy-enforcing signers gate freeze vs. spend separately.
#[derive(Debug, Clone, Copy)]
pub enum TokenTransactionKind {
    /// Issuer-side signature on a freeze message.
    Freeze,
    /// Owner-side signature on a partial token transaction (`compute_hash`).
    Partial,
    /// Owner-side signature on a finalized token transaction
    /// (`SHA256(SHA256(tx_hash || op_pubkey_hash))`).
    Final,
}

#[derive(Debug, Clone)]
pub struct PrepareTokenTransactionRequest {
    pub kind: TokenTransactionKind,
    /// 32-byte digest to Schnorr-sign with the identity key.
    pub digest: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct PreparedTokenTransaction {
    pub signature: schnorr::Signature,
}

// ─── trait ────────────────────────────────────────────────────────────────

/// High-level signing surface exposed to the Spark protocol layer.
#[macros::async_trait]
pub trait SparkSigner: Send + Sync + 'static {
    /// Returns the wallet's identity public key.
    async fn get_identity_public_key(&self) -> Result<PublicKey, SignerError>;

    /// Produce FROST shares for a batch of jobs (maps to `SPARK_SIGN_FROST`).
    /// Used directly by deposit tree creation, transfer/coop-exit refund
    /// signing, timelock renewal, static-deposit refund, lightning send, and
    /// swap (with adaptor). Results are returned in the same order as `jobs`.
    async fn sign_frost(&self, jobs: Vec<FrostJob>) -> Result<Vec<FrostShareResult>, SignerError>;

    /// Prepare an outbound transfer: per-leaf key-tweak (old − new), Feldman
    /// split, ECIES-encrypt to receiver and operators, and identity-key
    /// signature over the transfer-package payload. Generates the new leaf
    /// keys internally. Refund FROST signing is a separate `sign_frost` call.
    /// Maps to `SPARK_PREPARE_TRANSFER`.
    async fn prepare_transfer(
        &self,
        request: PrepareTransferRequest,
    ) -> Result<PreparedTransfer, SignerError>;

    /// Claim an inbound transfer: verify each sender signature, ECIES-decrypt
    /// the incoming leaf key, derive the receiver's new HD leaf key,
    /// FROST-sign the claim refunds with that new key, and produce
    /// ECIES-encrypted claim-tweak packages. Maps to `SPARK_CLAIM_TRANSFER`.
    async fn prepare_claim(
        &self,
        request: PrepareClaimRequest,
    ) -> Result<PreparedClaim, SignerError>;

    /// Prepare a Lightning receive: generate a random preimage in-enclave,
    /// Feldman-split it for the operators, return its hash for BOLT11.
    /// Maps to `SPARK_PREPARE_LIGHTNING_RECEIVE`.
    async fn prepare_lightning_receive(
        &self,
        request: PrepareLightningReceiveRequest,
    ) -> Result<PreparedLightningReceive, SignerError>;

    /// Prepare a static deposit: export the static-deposit secret to the SSP
    /// (ECIES) and FROST-sign the deposit tree txs.
    async fn prepare_static_deposit(
        &self,
        request: PrepareStaticDepositRequest,
    ) -> Result<PreparedStaticDeposit, SignerError>;

    /// Schnorr-sign a Spark invoice (sats or tokens) with the identity key.
    /// Spark invoices are unrelated to Lightning.
    async fn sign_spark_invoice(
        &self,
        request: SignSparkInvoiceRequest,
    ) -> Result<SignedSparkInvoice, SignerError>;

    /// Prepare a token transaction (freeze / partial / final): Schnorr-sign
    /// the digest with the identity key.
    async fn prepare_token_transaction(
        &self,
        request: PrepareTokenTransactionRequest,
    ) -> Result<PreparedTokenTransaction, SignerError>;
}
