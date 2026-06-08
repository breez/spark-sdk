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
//!  * timelock renewal = `sign_frost`
//!
//! Static-deposit refund is the exception: it is *user-commits-first* (the user
//! nonce commitment must reach the operators before they sign), so it can't be
//! a single operator-commits-first `sign_frost`. It gets its own method pair —
//! `start_static_deposit_refund` + `sign_static_deposit_refund` — split across
//! the operator round-trip.

use std::collections::BTreeMap;

use bitcoin::secp256k1::{PublicKey, SecretKey, ecdsa, schnorr};
use frost_secp256k1_tr::{Identifier, round1::SigningCommitments, round2::SignatureShare};

use super::{FrostSigningCommitmentsWithNonces, SignerError};
use crate::services::TransferId;
use crate::tree::{TreeNode, TreeNodeId};

// ─── shared types ─────────────────────────────────────────────────────────

/// A Spark Operator (statechain participant) the signer addresses for
/// share-encryption.
#[derive(Debug, Clone)]
pub struct OperatorRecipient {
    /// Numeric operator id; determines the Feldman share index (`id + 1`).
    pub id: usize,
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

/// A single leaf being sent in an outbound transfer. The signer derives the old
/// leaf key from `node.id` and the new (post-transfer) leaf key from
/// `new_leaf_id`: a freshly generated id supplied per send, so the new key is a
/// deterministic HD derivation distinct from the old one (a key-addressed signer
/// such as Turnkey cannot use a random key).
#[derive(Debug, Clone)]
pub struct TransferLeafInput {
    pub node: TreeNode,
    pub new_leaf_id: TreeNodeId,
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

/// A single leaf being claimed. Refund signing is a separate `sign_frost`
/// call (with the new derived leaf key); this carries only what the key-tweak
/// step needs.
#[derive(Debug, Clone)]
pub struct ClaimLeafInput {
    /// The leaf as it lands at the receiver (pre-claim state).
    pub node: TreeNode,
    /// Sender's signature binding this leaf to the transfer (raw bytes;
    /// verified in-enclave by policy-enforcing signers).
    pub sender_signature: Vec<u8>,
    /// ECIES ciphertext of the incoming leaf signing key, encrypted for this
    /// receiver; the signer decrypts it and derives the claim key tweak.
    pub leaf_key_ciphertext: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PrepareClaimRequest {
    pub transfer_id: TransferId,
    pub sender_identity_public_key: PublicKey,
    pub leaves: Vec<ClaimLeafInput>,
    pub operator_recipients: Vec<OperatorRecipient>,
    pub threshold: u32,
}

#[derive(Debug, Clone)]
pub struct PreparedClaim {
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

// ─── static-deposit refund ────────────────────────────────────────────────

/// Begin a static-deposit refund. Unlike every other FROST flow this one is
/// *user-commits-first*: the user's nonce commitment must reach the operators
/// (in `initiate_static_deposit_utxo_refund`) before they produce their shares,
/// so refund signing is split across the operator round-trip instead of being a
/// single operator-commits-first [`sign_frost`](SparkSigner::sign_frost) call.
///
/// It is also the *exported/local-key* path: the static-deposit key is
/// recoverable client-side (the default adapter signs with the local
/// static-deposit key; a Turnkey backend exports the static-deposit account),
/// so the returned nonce travels with that already-local key material between
/// the two calls.
#[derive(Debug, Clone)]
pub struct StartStaticDepositRefundRequest {
    /// Static-deposit address index.
    pub index: u32,
    /// The refund user-statement bytes to ECDSA-sign with the identity key
    /// (sent to the operators as `user_signature`).
    pub user_statement: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct StartedStaticDepositRefund {
    /// Static-deposit signing public key (operator
    /// `SigningJob.signing_public_key`).
    pub signing_public_key: PublicKey,
    /// The user's FROST nonce commitment: forward it to the operators, then
    /// pass it back into
    /// [`sign_static_deposit_refund`](SparkSigner::sign_static_deposit_refund).
    pub nonce_commitment: FrostSigningCommitmentsWithNonces,
    /// ECDSA identity signature over `user_statement`.
    pub user_signature: ecdsa::Signature,
}

/// Finish a static-deposit refund once the operators have produced their
/// signing result for the refund transaction.
#[derive(Debug, Clone)]
pub struct SignStaticDepositRefundRequest {
    /// Static-deposit address index.
    pub index: u32,
    /// 32-byte BIP-341 sighash of the refund spend transaction.
    pub sighash: [u8; 32],
    /// FROST group verifying key (from the operator response).
    pub verifying_key: PublicKey,
    /// The nonce commitment returned by
    /// [`start_static_deposit_refund`](SparkSigner::start_static_deposit_refund).
    pub nonce_commitment: FrostSigningCommitmentsWithNonces,
    /// Operators' round-1 commitments for the refund tx.
    pub statechain_commitments: BTreeMap<Identifier, SigningCommitments>,
    /// Operators' round-2 signature shares for the refund tx.
    pub statechain_signatures: BTreeMap<Identifier, SignatureShare>,
    /// Operators' public keys for the refund tx.
    pub statechain_public_keys: BTreeMap<Identifier, PublicKey>,
}

// ─── static-deposit claim ─────────────────────────────────────────────────

/// Prepare a static-deposit claim. Like the refund, this is the
/// *exported/local-key* path: the SSP co-signs the claim and therefore needs
/// the static-deposit secret in the clear, so the signer exports it. The
/// default adapter reads its local static-deposit key; a Turnkey backend
/// exports the static-deposit account from the enclave.
#[derive(Debug, Clone)]
pub struct PrepareStaticDepositClaimRequest {
    /// Static-deposit address index.
    pub index: u32,
    /// The claim user-statement bytes to ECDSA-sign with the identity key.
    pub user_statement: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PreparedStaticDepositClaim {
    /// The static-deposit secret key, exported in the clear for the SSP.
    pub deposit_secret_key: SecretKey,
    /// ECDSA identity signature over `user_statement`.
    pub user_signature: ecdsa::Signature,
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

    /// Returns the signing public key for a tree leaf. Needed by callers that
    /// must construct transactions (refunds, etc.) before signing them.
    async fn get_public_key_for_leaf(&self, leaf_id: &TreeNodeId)
    -> Result<PublicKey, SignerError>;

    /// Returns the static-deposit signing public key at `index`. Pure
    /// public-key derivation (no signing): the wallet hands this to the
    /// operators to derive a static-deposit address. Analogous to
    /// [`get_public_key_for_leaf`](Self::get_public_key_for_leaf).
    async fn get_static_deposit_public_key(&self, index: u32) -> Result<PublicKey, SignerError>;

    /// Signs a server authentication challenge with the wallet identity key
    /// (ECDSA). Used for Spark operator (gRPC) and SSP session authentication.
    async fn sign_authentication_challenge(
        &self,
        challenge: &[u8],
    ) -> Result<ecdsa::Signature, SignerError>;

    /// Signs an arbitrary user message with the wallet identity key (ECDSA).
    /// Distinct from [`sign_authentication_challenge`](Self::sign_authentication_challenge)
    /// so a policy-enforcing signer can gate user-facing message signing
    /// separately from session authentication.
    async fn sign_message(&self, message: &[u8]) -> Result<ecdsa::Signature, SignerError>;

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

    /// Claim an inbound transfer (key-tweak step): verify each sender
    /// signature, ECIES-decrypt the incoming leaf key, derive the receiver's
    /// new HD leaf key, compute the claim key tweak, and produce
    /// ECIES-encrypted claim-tweak packages plus the claim-package signature.
    /// Refund signing is a separate `sign_frost` call (new derived leaf key).
    /// Maps to `SPARK_CLAIM_TRANSFER`.
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

    /// Begin a static-deposit refund: return the static-deposit signing public
    /// key, a fresh user FROST nonce commitment, and the identity-key ECDSA
    /// signature over the refund user-statement. See
    /// [`StartStaticDepositRefundRequest`] for why this is split from
    /// [`sign_static_deposit_refund`](Self::sign_static_deposit_refund).
    async fn start_static_deposit_refund(
        &self,
        request: StartStaticDepositRefundRequest,
    ) -> Result<StartedStaticDepositRefund, SignerError>;

    /// Finish a static-deposit refund: produce the user's FROST share over the
    /// refund sighash (bound to the nonce from `start_static_deposit_refund`)
    /// and aggregate it with the operators' shares into the final signature.
    async fn sign_static_deposit_refund(
        &self,
        request: SignStaticDepositRefundRequest,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError>;

    /// Prepare a static-deposit claim: export the static-deposit secret in the
    /// clear (the SSP co-signs and needs it) and ECDSA-sign the claim
    /// user-statement with the identity key.
    async fn prepare_static_deposit_claim(
        &self,
        request: PrepareStaticDepositClaimRequest,
    ) -> Result<PreparedStaticDepositClaim, SignerError>;

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
