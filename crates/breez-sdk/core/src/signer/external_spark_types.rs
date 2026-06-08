//! FFI-compatible types for the high-level [`ExternalSparkSigner`] trait.
//!
//! These mirror the flow-level `spark_wallet::SparkSigner` request/response
//! types using FFI-safe representations (bytes, strings, vectors of pairs),
//! reusing the primitive conversions from [`super::external_types`].

use bitcoin::secp256k1;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::SdkError;

use super::external_types::{
    EcdsaSignatureBytes, ExternalFrostCommitments, ExternalFrostSignatureShare, ExternalIdentifier,
    ExternalSigningCommitments, ExternalTreeNodeId, IdentifierCommitmentPair,
    IdentifierPublicKeyPair, IdentifierSignaturePair, SchnorrSignatureBytes, SecretBytes,
};

/// FFI-safe representation of `spark_wallet::FrostDerivation`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ExternalFrostDerivation {
    /// The signing key for a tree leaf.
    SigningLeaf { leaf_id: ExternalTreeNodeId },
    /// The static-deposit key at `index`.
    StaticDeposit { index: u32 },
    /// The HTLC-preimage key.
    HtlcPreimage,
    /// The wallet identity key.
    Identity,
}

impl ExternalFrostDerivation {
    pub fn from_derivation(d: &spark_wallet::FrostDerivation) -> Result<Self, SdkError> {
        Ok(match d {
            spark_wallet::FrostDerivation::SigningLeaf { leaf_id } => Self::SigningLeaf {
                leaf_id: ExternalTreeNodeId::from_tree_node_id(leaf_id)?,
            },
            spark_wallet::FrostDerivation::StaticDeposit { index } => {
                Self::StaticDeposit { index: *index }
            }
            spark_wallet::FrostDerivation::HtlcPreimage => Self::HtlcPreimage,
            spark_wallet::FrostDerivation::Identity => Self::Identity,
        })
    }

    pub fn to_derivation(&self) -> Result<spark_wallet::FrostDerivation, SdkError> {
        Ok(match self {
            Self::SigningLeaf { leaf_id } => spark_wallet::FrostDerivation::SigningLeaf {
                leaf_id: leaf_id.to_tree_node_id()?,
            },
            Self::StaticDeposit { index } => {
                spark_wallet::FrostDerivation::StaticDeposit { index: *index }
            }
            Self::HtlcPreimage => spark_wallet::FrostDerivation::HtlcPreimage,
            Self::Identity => spark_wallet::FrostDerivation::Identity,
        })
    }
}

/// FFI-safe representation of `spark_wallet::FrostJob`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalFrostJob {
    /// Which key to sign with.
    pub derivation: ExternalFrostDerivation,
    /// 32-byte BIP-341 sighash to sign.
    pub sighash: Vec<u8>,
    /// FROST group verifying key (33 bytes compressed).
    pub verifying_key: Vec<u8>,
    /// Per-operator round-1 commitments.
    pub operator_commitments: Vec<IdentifierCommitmentPair>,
    /// Optional adaptor public key (33 bytes compressed).
    pub adaptor_public_key: Option<Vec<u8>>,
}

impl ExternalFrostJob {
    pub fn from_frost_job(job: &spark_wallet::FrostJob) -> Result<Self, SdkError> {
        let operator_commitments = job
            .operator_commitments
            .iter()
            .map(|(id, comm)| {
                Ok(IdentifierCommitmentPair {
                    identifier: ExternalIdentifier::from_identifier(id),
                    commitment: ExternalSigningCommitments::from_signing_commitments(comm)?,
                })
            })
            .collect::<Result<Vec<_>, SdkError>>()?;
        Ok(Self {
            derivation: ExternalFrostDerivation::from_derivation(&job.derivation)?,
            sighash: job.sighash.to_vec(),
            verifying_key: job.verifying_key.serialize().to_vec(),
            operator_commitments,
            adaptor_public_key: job.adaptor_public_key.map(|pk| pk.serialize().to_vec()),
        })
    }

    pub fn to_frost_job(&self) -> Result<spark_wallet::FrostJob, SdkError> {
        let sighash: [u8; 32] = self.sighash[..]
            .try_into()
            .map_err(|_| SdkError::Generic("FROST sighash must be 32 bytes".to_string()))?;
        let verifying_key = secp256k1::PublicKey::from_slice(&self.verifying_key)
            .map_err(|e| SdkError::Generic(format!("Invalid verifying key: {e}")))?;
        let operator_commitments: BTreeMap<_, _> = self
            .operator_commitments
            .iter()
            .map(|p| {
                Ok((
                    p.identifier.to_identifier()?,
                    p.commitment.to_signing_commitments()?,
                ))
            })
            .collect::<Result<_, SdkError>>()?;
        let adaptor_public_key = self
            .adaptor_public_key
            .as_ref()
            .map(|b| {
                secp256k1::PublicKey::from_slice(b)
                    .map_err(|e| SdkError::Generic(format!("Invalid adaptor public key: {e}")))
            })
            .transpose()?;
        Ok(spark_wallet::FrostJob {
            derivation: self.derivation.to_derivation()?,
            sighash,
            verifying_key,
            operator_commitments,
            adaptor_public_key,
        })
    }
}

/// FFI-safe representation of `spark_wallet::FrostShareResult`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalFrostShareResult {
    /// The user's nonce commitment (round-1 output).
    pub commitment: ExternalFrostCommitments,
    /// The user's signature share (round-2 output).
    pub signature_share: ExternalFrostSignatureShare,
}

impl ExternalFrostShareResult {
    pub fn from_frost_share_result(
        result: &spark_wallet::FrostShareResult,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            commitment: ExternalFrostCommitments::from_frost_commitments(&result.commitment)?,
            signature_share: ExternalFrostSignatureShare::from_signature_share(
                &result.signature_share,
            )?,
        })
    }

    pub fn to_frost_share_result(&self) -> Result<spark_wallet::FrostShareResult, SdkError> {
        Ok(spark_wallet::FrostShareResult {
            commitment: self.commitment.to_frost_commitments()?,
            signature_share: self.signature_share.to_signature_share()?,
        })
    }
}

/// FFI-safe representation of `spark_wallet::OperatorRecipient`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalOperatorRecipient {
    /// Numeric operator id (determines the Feldman share index).
    pub id: u64,
    /// FROST identifier.
    pub identifier: ExternalIdentifier,
    /// The operator's ECIES / identity public key (33 bytes compressed).
    pub public_key: Vec<u8>,
}

impl ExternalOperatorRecipient {
    pub fn from_operator_recipient(
        recipient: &spark_wallet::OperatorRecipient,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            id: recipient.id as u64,
            identifier: ExternalIdentifier::from_identifier(&recipient.identifier),
            public_key: recipient.public_key.serialize().to_vec(),
        })
    }

    pub fn to_operator_recipient(&self) -> Result<spark_wallet::OperatorRecipient, SdkError> {
        Ok(spark_wallet::OperatorRecipient {
            id: usize::try_from(self.id)
                .map_err(|_| SdkError::Generic("operator id out of range".to_string()))?,
            identifier: self.identifier.to_identifier()?,
            public_key: secp256k1::PublicKey::from_slice(&self.public_key)
                .map_err(|e| SdkError::Generic(format!("Invalid operator public key: {e}")))?,
        })
    }
}

/// FFI-safe representation of `spark_wallet::OperatorPackage`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalOperatorPackage {
    /// The operator this package is encrypted for.
    pub operator_identifier: ExternalIdentifier,
    /// The ECIES-encrypted package bytes.
    pub encrypted_package: Vec<u8>,
}

impl ExternalOperatorPackage {
    pub fn from_operator_package(pkg: &spark_wallet::OperatorPackage) -> Result<Self, SdkError> {
        Ok(Self {
            operator_identifier: ExternalIdentifier::from_identifier(&pkg.operator_identifier),
            encrypted_package: pkg.encrypted_package.clone(),
        })
    }

    pub fn to_operator_package(&self) -> Result<spark_wallet::OperatorPackage, SdkError> {
        Ok(spark_wallet::OperatorPackage {
            operator_identifier: self.operator_identifier.to_identifier()?,
            encrypted_package: self.encrypted_package.clone(),
        })
    }
}

fn public_key_from_bytes(bytes: &[u8], what: &str) -> Result<secp256k1::PublicKey, SdkError> {
    secp256k1::PublicKey::from_slice(bytes)
        .map_err(|e| SdkError::Generic(format!("Invalid {what}: {e}")))
}

fn sighash_32(bytes: &[u8]) -> Result<[u8; 32], SdkError> {
    bytes
        .try_into()
        .map_err(|_| SdkError::Generic("sighash must be 32 bytes".to_string()))
}

// ─── prepare_transfer ───────────────────────────────────────────────────────

/// FFI-safe representation of `spark_wallet::TransferLeafInput`. Conveys the old
/// leaf id and the new (post-transfer) leaf id; the signer derives keys from them.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalTransferLeafInput {
    pub node_id: ExternalTreeNodeId,
    pub new_leaf_id: ExternalTreeNodeId,
}

impl ExternalTransferLeafInput {
    pub fn from_transfer_leaf_input(
        leaf: &spark_wallet::TransferLeafInput,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            node_id: ExternalTreeNodeId::from_tree_node_id(&leaf.node.id)?,
            new_leaf_id: ExternalTreeNodeId::from_tree_node_id(&leaf.new_leaf_id)?,
        })
    }
}

/// FFI-safe representation of `spark_wallet::NewLeafKey`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalNewLeafKey {
    pub node_id: ExternalTreeNodeId,
    /// New signing public key (33 bytes compressed).
    pub new_signing_public_key: Vec<u8>,
}

impl ExternalNewLeafKey {
    pub fn to_new_leaf_key(&self) -> Result<spark_wallet::NewLeafKey, SdkError> {
        Ok(spark_wallet::NewLeafKey {
            node_id: self.node_id.to_tree_node_id()?,
            new_signing_public_key: public_key_from_bytes(
                &self.new_signing_public_key,
                "new signing public key",
            )?,
        })
    }
}

/// FFI-safe representation of `spark_wallet::PrepareTransferRequest`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPrepareTransferRequest {
    pub transfer_id: String,
    /// Receiver public key (33 bytes compressed).
    pub receiver_public_key: Vec<u8>,
    pub leaves: Vec<ExternalTransferLeafInput>,
    pub operator_recipients: Vec<ExternalOperatorRecipient>,
    pub threshold: u32,
}

impl ExternalPrepareTransferRequest {
    pub fn from_prepare_transfer_request(
        r: &spark_wallet::PrepareTransferRequest,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            transfer_id: r.transfer_id.to_string(),
            receiver_public_key: r.receiver_public_key.serialize().to_vec(),
            leaves: r
                .leaves
                .iter()
                .map(ExternalTransferLeafInput::from_transfer_leaf_input)
                .collect::<Result<_, _>>()?,
            operator_recipients: r
                .operator_recipients
                .iter()
                .map(ExternalOperatorRecipient::from_operator_recipient)
                .collect::<Result<_, _>>()?,
            threshold: r.threshold,
        })
    }
}

/// FFI-safe representation of `spark_wallet::PreparedTransfer`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPreparedTransfer {
    pub operator_packages: Vec<ExternalOperatorPackage>,
    pub new_leaf_keys: Vec<ExternalNewLeafKey>,
    pub transfer_user_signature: EcdsaSignatureBytes,
}

impl ExternalPreparedTransfer {
    pub fn to_prepared_transfer(&self) -> Result<spark_wallet::PreparedTransfer, SdkError> {
        Ok(spark_wallet::PreparedTransfer {
            operator_packages: self
                .operator_packages
                .iter()
                .map(ExternalOperatorPackage::to_operator_package)
                .collect::<Result<_, _>>()?,
            new_leaf_keys: self
                .new_leaf_keys
                .iter()
                .map(ExternalNewLeafKey::to_new_leaf_key)
                .collect::<Result<_, _>>()?,
            transfer_user_signature: self.transfer_user_signature.to_signature()?,
        })
    }
}

// ─── prepare_claim ──────────────────────────────────────────────────────────

/// FFI-safe representation of `spark_wallet::ClaimLeafInput`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalClaimLeafInput {
    pub node_id: ExternalTreeNodeId,
    pub sender_signature: Vec<u8>,
    pub leaf_key_ciphertext: Vec<u8>,
}

impl ExternalClaimLeafInput {
    pub fn from_claim_leaf_input(leaf: &spark_wallet::ClaimLeafInput) -> Result<Self, SdkError> {
        Ok(Self {
            node_id: ExternalTreeNodeId::from_tree_node_id(&leaf.node.id)?,
            sender_signature: leaf.sender_signature.clone(),
            leaf_key_ciphertext: leaf.leaf_key_ciphertext.clone(),
        })
    }
}

/// FFI-safe representation of `spark_wallet::PrepareClaimRequest`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPrepareClaimRequest {
    pub transfer_id: String,
    /// Sender identity public key (33 bytes compressed).
    pub sender_identity_public_key: Vec<u8>,
    pub leaves: Vec<ExternalClaimLeafInput>,
    pub operator_recipients: Vec<ExternalOperatorRecipient>,
    pub threshold: u32,
}

impl ExternalPrepareClaimRequest {
    pub fn from_prepare_claim_request(
        r: &spark_wallet::PrepareClaimRequest,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            transfer_id: r.transfer_id.to_string(),
            sender_identity_public_key: r.sender_identity_public_key.serialize().to_vec(),
            leaves: r
                .leaves
                .iter()
                .map(ExternalClaimLeafInput::from_claim_leaf_input)
                .collect::<Result<_, _>>()?,
            operator_recipients: r
                .operator_recipients
                .iter()
                .map(ExternalOperatorRecipient::from_operator_recipient)
                .collect::<Result<_, _>>()?,
            threshold: r.threshold,
        })
    }
}

/// FFI-safe representation of `spark_wallet::PreparedClaim`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPreparedClaim {
    pub operator_packages: Vec<ExternalOperatorPackage>,
}

impl ExternalPreparedClaim {
    pub fn to_prepared_claim(&self) -> Result<spark_wallet::PreparedClaim, SdkError> {
        Ok(spark_wallet::PreparedClaim {
            operator_packages: self
                .operator_packages
                .iter()
                .map(ExternalOperatorPackage::to_operator_package)
                .collect::<Result<_, _>>()?,
        })
    }
}

// ─── prepare_lightning_receive ──────────────────────────────────────────────

/// FFI-safe representation of `spark_wallet::PrepareLightningReceiveRequest`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPrepareLightningReceiveRequest {
    pub operator_recipients: Vec<ExternalOperatorRecipient>,
    pub threshold: u32,
}

impl ExternalPrepareLightningReceiveRequest {
    pub fn from_prepare_lightning_receive_request(
        r: &spark_wallet::PrepareLightningReceiveRequest,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            operator_recipients: r
                .operator_recipients
                .iter()
                .map(ExternalOperatorRecipient::from_operator_recipient)
                .collect::<Result<_, _>>()?,
            threshold: r.threshold,
        })
    }
}

/// FFI-safe representation of `spark_wallet::PreparedLightningReceive`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPreparedLightningReceive {
    /// SHA256 of the in-enclave preimage (32 bytes).
    pub payment_hash: Vec<u8>,
    pub operator_preimage_packages: Vec<ExternalOperatorPackage>,
}

impl ExternalPreparedLightningReceive {
    pub fn to_prepared_lightning_receive(
        &self,
    ) -> Result<spark_wallet::PreparedLightningReceive, SdkError> {
        Ok(spark_wallet::PreparedLightningReceive {
            payment_hash: sighash_32(&self.payment_hash)?,
            operator_preimage_packages: self
                .operator_preimage_packages
                .iter()
                .map(ExternalOperatorPackage::to_operator_package)
                .collect::<Result<_, _>>()?,
        })
    }
}

// ─── prepare_static_deposit ─────────────────────────────────────────────────

/// FFI-safe representation of `spark_wallet::PrepareStaticDepositRequest`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPrepareStaticDepositRequest {
    pub index: u32,
    /// SSP public key (33 bytes compressed).
    pub ssp_public_key: Vec<u8>,
    pub frost_jobs: Vec<ExternalFrostJob>,
}

impl ExternalPrepareStaticDepositRequest {
    pub fn from_prepare_static_deposit_request(
        r: &spark_wallet::PrepareStaticDepositRequest,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            index: r.index,
            ssp_public_key: r.ssp_public_key.serialize().to_vec(),
            frost_jobs: r
                .frost_jobs
                .iter()
                .map(ExternalFrostJob::from_frost_job)
                .collect::<Result<_, _>>()?,
        })
    }
}

/// FFI-safe representation of `spark_wallet::PreparedStaticDeposit`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPreparedStaticDeposit {
    pub exported_secret: Vec<u8>,
    pub frost_shares: Vec<ExternalFrostShareResult>,
}

impl ExternalPreparedStaticDeposit {
    pub fn to_prepared_static_deposit(
        &self,
    ) -> Result<spark_wallet::PreparedStaticDeposit, SdkError> {
        Ok(spark_wallet::PreparedStaticDeposit {
            exported_secret: self.exported_secret.clone(),
            frost_shares: self
                .frost_shares
                .iter()
                .map(ExternalFrostShareResult::to_frost_share_result)
                .collect::<Result<_, _>>()?,
        })
    }
}

// ─── static-deposit refund ──────────────────────────────────────────────────

/// FFI-safe representation of `spark_wallet::StartStaticDepositRefundRequest`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalStartStaticDepositRefundRequest {
    pub index: u32,
    pub user_statement: Vec<u8>,
}

impl ExternalStartStaticDepositRefundRequest {
    pub fn from_start_static_deposit_refund_request(
        r: &spark_wallet::StartStaticDepositRefundRequest,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            index: r.index,
            user_statement: r.user_statement.clone(),
        })
    }
}

/// FFI-safe representation of `spark_wallet::StartedStaticDepositRefund`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalStartedStaticDepositRefund {
    /// Static-deposit signing public key (33 bytes compressed).
    pub signing_public_key: Vec<u8>,
    pub nonce_commitment: ExternalFrostCommitments,
    pub user_signature: EcdsaSignatureBytes,
}

impl ExternalStartedStaticDepositRefund {
    pub fn to_started_static_deposit_refund(
        &self,
    ) -> Result<spark_wallet::StartedStaticDepositRefund, SdkError> {
        Ok(spark_wallet::StartedStaticDepositRefund {
            signing_public_key: public_key_from_bytes(
                &self.signing_public_key,
                "static-deposit signing public key",
            )?,
            nonce_commitment: self.nonce_commitment.to_frost_commitments()?,
            user_signature: self.user_signature.to_signature()?,
        })
    }
}

/// FFI-safe representation of `spark_wallet::SignStaticDepositRefundRequest`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalSignStaticDepositRefundRequest {
    pub index: u32,
    pub sighash: Vec<u8>,
    /// FROST group verifying key (33 bytes compressed).
    pub verifying_key: Vec<u8>,
    pub nonce_commitment: ExternalFrostCommitments,
    pub statechain_commitments: Vec<IdentifierCommitmentPair>,
    pub statechain_signatures: Vec<IdentifierSignaturePair>,
    pub statechain_public_keys: Vec<IdentifierPublicKeyPair>,
}

impl ExternalSignStaticDepositRefundRequest {
    pub fn from_sign_static_deposit_refund_request(
        r: &spark_wallet::SignStaticDepositRefundRequest,
    ) -> Result<Self, SdkError> {
        let statechain_commitments = r
            .statechain_commitments
            .iter()
            .map(|(id, comm)| {
                Ok(IdentifierCommitmentPair {
                    identifier: ExternalIdentifier::from_identifier(id),
                    commitment: ExternalSigningCommitments::from_signing_commitments(comm)?,
                })
            })
            .collect::<Result<Vec<_>, SdkError>>()?;
        let statechain_signatures = r
            .statechain_signatures
            .iter()
            .map(|(id, share)| {
                Ok(IdentifierSignaturePair {
                    identifier: ExternalIdentifier::from_identifier(id),
                    signature: ExternalFrostSignatureShare::from_signature_share(share)?,
                })
            })
            .collect::<Result<Vec<_>, SdkError>>()?;
        let statechain_public_keys = r
            .statechain_public_keys
            .iter()
            .map(|(id, pk)| IdentifierPublicKeyPair {
                identifier: ExternalIdentifier::from_identifier(id),
                public_key: pk.serialize().to_vec(),
            })
            .collect();
        Ok(Self {
            index: r.index,
            sighash: r.sighash.to_vec(),
            verifying_key: r.verifying_key.serialize().to_vec(),
            nonce_commitment: ExternalFrostCommitments::from_frost_commitments(
                &r.nonce_commitment,
            )?,
            statechain_commitments,
            statechain_signatures,
            statechain_public_keys,
        })
    }
}

// ─── sign_spark_invoice ─────────────────────────────────────────────────────

/// FFI-safe representation of `spark_wallet::SparkInvoiceKind`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ExternalSparkInvoiceKind {
    Sats,
    Tokens,
}

impl ExternalSparkInvoiceKind {
    pub fn from_kind(kind: &spark_wallet::SparkInvoiceKind) -> Self {
        match kind {
            spark_wallet::SparkInvoiceKind::Sats => Self::Sats,
            spark_wallet::SparkInvoiceKind::Tokens => Self::Tokens,
        }
    }
}

/// FFI-safe representation of `spark_wallet::SignSparkInvoiceRequest`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalSignSparkInvoiceRequest {
    pub kind: ExternalSparkInvoiceKind,
    pub invoice_hash: Vec<u8>,
}

impl ExternalSignSparkInvoiceRequest {
    pub fn from_sign_spark_invoice_request(
        r: &spark_wallet::SignSparkInvoiceRequest,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            kind: ExternalSparkInvoiceKind::from_kind(&r.kind),
            invoice_hash: r.invoice_hash.to_vec(),
        })
    }
}

/// FFI-safe representation of `spark_wallet::SignedSparkInvoice`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalSignedSparkInvoice {
    pub signature: SchnorrSignatureBytes,
}

impl ExternalSignedSparkInvoice {
    pub fn to_signed_spark_invoice(&self) -> Result<spark_wallet::SignedSparkInvoice, SdkError> {
        Ok(spark_wallet::SignedSparkInvoice {
            signature: self.signature.to_signature()?,
        })
    }
}

// ─── prepare_token_transaction ──────────────────────────────────────────────

/// FFI-safe representation of `spark_wallet::TokenTransactionKind`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ExternalTokenTransactionKind {
    Freeze,
    Partial,
    Final,
}

impl ExternalTokenTransactionKind {
    pub fn from_kind(kind: &spark_wallet::TokenTransactionKind) -> Self {
        match kind {
            spark_wallet::TokenTransactionKind::Freeze => Self::Freeze,
            spark_wallet::TokenTransactionKind::Partial => Self::Partial,
            spark_wallet::TokenTransactionKind::Final => Self::Final,
        }
    }
}

/// FFI-safe representation of `spark_wallet::PrepareTokenTransactionRequest`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPrepareTokenTransactionRequest {
    pub kind: ExternalTokenTransactionKind,
    pub digest: Vec<u8>,
}

impl ExternalPrepareTokenTransactionRequest {
    pub fn from_prepare_token_transaction_request(
        r: &spark_wallet::PrepareTokenTransactionRequest,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            kind: ExternalTokenTransactionKind::from_kind(&r.kind),
            digest: r.digest.to_vec(),
        })
    }
}

/// FFI-safe representation of `spark_wallet::PreparedTokenTransaction`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPreparedTokenTransaction {
    pub signature: SchnorrSignatureBytes,
}

impl ExternalPreparedTokenTransaction {
    pub fn to_prepared_token_transaction(
        &self,
    ) -> Result<spark_wallet::PreparedTokenTransaction, SdkError> {
        Ok(spark_wallet::PreparedTokenTransaction {
            signature: self.signature.to_signature()?,
        })
    }
}

// ─── static-deposit claim ───────────────────────────────────────────────────

/// FFI-safe representation of `spark_wallet::PrepareStaticDepositClaimRequest`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPrepareStaticDepositClaimRequest {
    pub index: u32,
    pub user_statement: Vec<u8>,
}

impl ExternalPrepareStaticDepositClaimRequest {
    pub fn from_prepare_static_deposit_claim_request(
        r: &spark_wallet::PrepareStaticDepositClaimRequest,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            index: r.index,
            user_statement: r.user_statement.clone(),
        })
    }
}

/// FFI-safe representation of `spark_wallet::PreparedStaticDepositClaim`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalPreparedStaticDepositClaim {
    /// The static-deposit secret key, exported in the clear for the SSP.
    pub deposit_secret_key: SecretBytes,
    pub user_signature: EcdsaSignatureBytes,
}

impl ExternalPreparedStaticDepositClaim {
    pub fn to_prepared_static_deposit_claim(
        &self,
    ) -> Result<spark_wallet::PreparedStaticDepositClaim, SdkError> {
        Ok(spark_wallet::PreparedStaticDepositClaim {
            deposit_secret_key: self.deposit_secret_key.to_secret_key()?,
            user_signature: self.user_signature.to_signature()?,
        })
    }
}
